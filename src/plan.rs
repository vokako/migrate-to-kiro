//! Turn an [`Inventory`] into a plan of file changes, then apply it safely.
//!
//! Safety rules, in order:
//! 1. Nothing is written by `build`; only `apply` touches the disk.
//! 2. Files this tool wrote earlier are updated only while they are unmodified
//!    (a content hash is stored with each one).
//! 3. Anything else that already exists is a conflict and is left alone unless
//!    `force` is set, in which case a backup is kept first.

use crate::mcp::{canonical, McpServer};
use crate::model::{Ctx, Finding, Inventory, Kind, Level, Severity, Tool};
use crate::rules::{render, Rule};
use crate::skills::{self, Skill, MARKER_FILE};
use crate::text::fnv1a;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const EXCLUDE_PATTERN: &str = ".kiro/steering/imported-*-local-*.md";
const MCP_SIDECAR: &str = ".migrate-to-kiro-mcp.json";

/// Reading order. `.mcp.json` and `.claude/skills` are shared, so the tool that
/// created the convention goes first and gets the credit.
const PRIORITY: [Tool; 5] = [Tool::Claude, Tool::CodeBuddy, Tool::Qoder, Tool::Cursor, Tool::Copilot];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create,
    Update,
    Unchanged,
    /// Exists and must not be overwritten without `--force`; carries the reason.
    Conflict(String),
}

#[derive(Debug, Clone)]
pub enum Payload {
    Steering { path: PathBuf, content: String },
    Mcp { file: PathBuf, name: String, config: Value },
    Skill { dir: PathBuf, skill: Box<Skill> },
}

#[derive(Debug, Clone)]
pub struct Change {
    pub kind: Kind,
    pub tool: Tool,
    pub level: Level,
    pub name: String,
    /// Short description of the result (inclusion mode, transport, file count).
    pub label: String,
    pub origin: String,
    pub target: String,
    pub action: Action,
    pub payload: Payload,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub changes: Vec<Change>,
    pub findings: Vec<Finding>,
    pub excludes: Vec<(PathBuf, String)>,
}

impl Plan {
    pub fn count(&self, f: impl Fn(&Action) -> bool) -> usize {
        self.changes.iter().filter(|c| f(&c.action)).count()
    }

    /// Changes `apply` would write.
    pub fn pending(&self, force: bool) -> usize {
        self.count(|a| match a {
            Action::Create | Action::Update => true,
            Action::Conflict(_) => force,
            Action::Unchanged => false,
        })
    }
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub written: Vec<PathBuf>,
    pub backups: Vec<PathBuf>,
    pub skipped_conflicts: usize,
    pub excludes_added: Vec<PathBuf>,
}

// ── scan ─────────────────────────────────────────────────────────────────

/// Read the selected tools' files. Never writes.
pub fn scan(tools: &[Tool], kinds: &[Kind], ctx: &Ctx) -> Inventory {
    let mut inv = Inventory::default();
    for t in PRIORITY.iter().filter(|t| tools.contains(t)) {
        if kinds.contains(&Kind::Steering) {
            crate::rules::discover(*t, ctx, &mut inv);
        }
        if kinds.contains(&Kind::Mcp) {
            crate::mcp::discover(*t, ctx, &mut inv);
        }
        if kinds.contains(&Kind::Skill) {
            crate::skills::discover(*t, ctx, &mut inv);
        }
    }
    inv
}

// ── build ────────────────────────────────────────────────────────────────

fn level_order(l: Level) -> u8 {
    match l {
        Level::User => 0,
        Level::Project => 1,
        Level::Local => 2,
    }
}

pub fn build(inv: Inventory, ctx: &Ctx) -> Plan {
    let mut plan = Plan { findings: inv.findings, ..Default::default() };

    for rule in inv.rules {
        plan_rule(rule, ctx, &mut plan);
    }
    plan_mcp(inv.mcp, ctx, &mut plan);
    for skill in inv.skills {
        plan_skill(skill, ctx, &mut plan);
    }

    plan.changes.sort_by(|a, b| {
        (level_order(a.level), a.kind, a.tool, &a.name).cmp(&(level_order(b.level), b.kind, b.tool, &b.name))
    });

    if plan.changes.iter().any(|c| c.kind == Kind::Steering && c.level == Level::Local) {
        if let Some(project) = &ctx.project {
            let git = project.join(".git");
            if git.is_dir() {
                plan.excludes.push((git.join("info").join("exclude"), EXCLUDE_PATTERN.into()));
            } else {
                plan.findings.push(Finding {
                    severity: Severity::Lossy,
                    tool: Tool::Claude,
                    kind: Some(Kind::Steering),
                    origin: None,
                    message: format!(
                        "personal notes were written to the workspace but {} has no .git directory to ignore them in; add `{EXCLUDE_PATTERN}` to your ignore file so they are not committed",
                        project.display()
                    ),
                });
            }
        }
    }
    plan
}

fn no_target(plan: &mut Plan, tool: Tool, kind: Kind, origin: &str) {
    plan.findings.push(Finding {
        severity: Severity::Skipped,
        tool,
        kind: Some(kind),
        origin: Some(origin.to_string()),
        message: "no project directory to write to".into(),
    });
}

fn plan_rule(rule: Rule, ctx: &Ctx, plan: &mut Plan) {
    let Some(kiro) = ctx.kiro_dir(rule.level) else { return no_target(plan, rule.tool, Kind::Steering, &rule.origin) };
    let path = kiro.join("steering").join(render::file_name(&rule));
    let content = render::render(&rule);
    let action = match fs::read_to_string(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound => Action::Create,
        Err(e) => Action::Conflict(format!("cannot read existing file: {e}")),
        Ok(existing) if existing == content => Action::Unchanged,
        Ok(existing) => match render::is_untouched(&existing) {
            Some(true) => Action::Update,
            Some(false) => Action::Conflict("edited after it was imported".into()),
            None => Action::Conflict("exists and was not created by migrate-to-kiro".into()),
        },
    };
    plan.changes.push(Change {
        kind: Kind::Steering,
        tool: rule.tool,
        level: rule.level,
        name: rule.name.clone(),
        label: rule.inclusion.label(),
        origin: rule.origin.clone(),
        target: ctx.display(&path),
        action,
        payload: Payload::Steering { path, content },
    });
}

// MCP ────────────────────────────────────────────────────────────────────

struct McpFile {
    root: Option<Value>,
    problem: Option<String>,
    /// name → hash of the config this tool wrote.
    owned: BTreeMap<String, String>,
}

fn load_mcp_file(file: &Path) -> McpFile {
    let (root, problem) = match fs::read_to_string(file) {
        Err(e) if e.kind() == ErrorKind::NotFound => (None, None),
        Err(e) => (None, Some(format!("cannot read {}: {e}", file.display()))),
        Ok(t) if t.trim().is_empty() => (None, None),
        Ok(t) => match crate::mcp::parse_json(&t) {
            Ok(v) if v.is_object() => (Some(v), None),
            Ok(_) => (None, Some(format!("{} is not a JSON object", file.display()))),
            Err(e) => (None, Some(format!("{} is not valid JSON ({e})", file.display()))),
        },
    };
    let owned = file
        .parent()
        .map(|p| p.join(MCP_SIDECAR))
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("servers").and_then(Value::as_object).cloned())
        .map(|m| m.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect())
        .unwrap_or_default();
    McpFile { root, problem, owned }
}

fn config_hash(v: &Value) -> String {
    format!("{:016x}", fnv1a(canonical(v).as_bytes()))
}

fn plan_mcp(servers: Vec<McpServer>, ctx: &Ctx, plan: &mut Plan) {
    let mut files: HashMap<PathBuf, McpFile> = HashMap::new();
    let mut taken: HashMap<(PathBuf, String), (Tool, Value)> = HashMap::new();

    for s in servers {
        let Some(kiro) = ctx.kiro_dir(s.level) else { no_target(plan, s.tool, Kind::Mcp, &s.origin); continue };
        let file = kiro.join("settings").join("mcp.json");

        // The same server often appears in several tools' files.
        if let Some((first, cfg)) = taken.get(&(file.clone(), s.name.clone())) {
            if *cfg != s.config {
                plan.findings.push(Finding {
                    severity: Severity::Skipped,
                    tool: s.tool,
                    kind: Some(Kind::Mcp),
                    origin: Some(s.origin.clone()),
                    message: format!("name clash: `{}` is already taken from {first} with a different configuration", s.name),
                });
            }
            continue;
        }
        taken.insert((file.clone(), s.name.clone()), (s.tool, s.config.clone()));

        let state = files.entry(file.clone()).or_insert_with(|| load_mcp_file(&file));
        let action = if let Some(problem) = &state.problem {
            Action::Conflict(problem.clone())
        } else {
            let existing = state.root.as_ref().and_then(|r| r.get("mcpServers")).and_then(|m| m.get(&s.name));
            match existing {
                None => Action::Create,
                Some(v) if *v == s.config => Action::Unchanged,
                Some(v) if state.owned.get(&s.name).map(String::as_str) == Some(config_hash(v).as_str()) => Action::Update,
                Some(_) => Action::Conflict("a server with this name already exists in Kiro".into()),
            }
        };
        plan.changes.push(Change {
            kind: Kind::Mcp,
            tool: s.tool,
            level: s.level,
            name: s.name.clone(),
            label: s.transport().into(),
            origin: s.origin.clone(),
            target: format!("{} [{}]", ctx.display(&file), s.name),
            action,
            payload: Payload::Mcp { file, name: s.name, config: s.config },
        });
    }
}

// Skills ─────────────────────────────────────────────────────────────────

fn plan_skill(skill: Skill, ctx: &Ctx, plan: &mut Plan) {
    let Some(kiro) = ctx.kiro_dir(skill.level) else { return no_target(plan, skill.tool, Kind::Skill, &skill.origin) };
    let dir = kiro.join("skills").join(&skill.name);
    let action = if !dir.exists() {
        Action::Create
    } else {
        let marker = fs::read_to_string(dir.join(MARKER_FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        match marker.as_ref().and_then(|m| m.get("hash")).and_then(Value::as_str).map(str::to_string) {
            None => Action::Conflict("exists and was not created by migrate-to-kiro".into()),
            Some(recorded) => match skills::dir_hash(&dir) {
                Err(e) => Action::Conflict(format!("cannot read existing skill: {e}")),
                Ok(now) if format!("{now:016x}") != recorded => Action::Conflict("edited after it was imported".into()),
                Ok(_) if recorded == format!("{:016x}", skill.hash) => Action::Unchanged,
                Ok(_) => Action::Update,
            },
        }
    };
    plan.changes.push(Change {
        kind: Kind::Skill,
        tool: skill.tool,
        level: skill.level,
        name: skill.name.clone(),
        label: format!("{} file(s)", skill.file_count()),
        origin: skill.origin.clone(),
        target: ctx.display(&dir),
        action,
        payload: Payload::Skill { dir, skill: Box::new(skill) },
    });
}

// ── apply ────────────────────────────────────────────────────────────────

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("migrate-to-kiro-tmp");
    fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub fn apply(plan: &Plan, force: bool) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    let mut mcp_by_file: BTreeMap<PathBuf, Vec<(&str, &Value)>> = BTreeMap::new();

    for c in &plan.changes {
        let go = match &c.action {
            Action::Unchanged => false,
            Action::Create | Action::Update => true,
            Action::Conflict(_) if force => true,
            Action::Conflict(_) => {
                report.skipped_conflicts += 1;
                false
            }
        };
        if !go {
            continue;
        }
        let forced = matches!(c.action, Action::Conflict(_));
        match &c.payload {
            Payload::Steering { path, content } => {
                if forced && path.is_file() {
                    let bak = path.with_extension("md.bak");
                    fs::copy(path, &bak).with_context(|| format!("back up {}", path.display()))?;
                    report.backups.push(bak);
                }
                write_atomic(path, content)?;
                report.written.push(path.clone());
            }
            Payload::Mcp { file, name, config } => {
                mcp_by_file.entry(file.clone()).or_default().push((name, config));
            }
            Payload::Skill { dir, skill } => {
                if let Some(bak) = install_skill(dir, skill, forced)? {
                    report.backups.push(bak);
                }
                report.written.push(dir.clone());
            }
        }
    }

    for (file, entries) in mcp_by_file {
        let state = load_mcp_file(&file);
        if let Some(problem) = state.problem {
            return Err(anyhow!("{problem}; nothing was written to it"));
        }
        let mut root = state.root.unwrap_or_else(|| json!({}));
        let existed = file.is_file();
        let servers = root
            .as_object_mut()
            .ok_or_else(|| anyhow!("{} is not a JSON object", file.display()))?
            .entry("mcpServers")
            .or_insert_with(|| json!({}));
        let servers = servers.as_object_mut().ok_or_else(|| anyhow!("`mcpServers` in {} is not an object", file.display()))?;
        let mut owned = state.owned;
        for (name, config) in entries {
            servers.insert(name.to_string(), config.clone());
            owned.insert(name.to_string(), config_hash(config));
        }
        if existed {
            let bak = file.with_extension("json.bak");
            fs::copy(&file, &bak).with_context(|| format!("back up {}", file.display()))?;
            report.backups.push(bak);
        }
        write_atomic(&file, &(serde_json::to_string_pretty(&root)? + "\n"))?;
        report.written.push(file.clone());
        if let Some(parent) = file.parent() {
            let sidecar = json!({ "servers": owned });
            write_atomic(&parent.join(MCP_SIDECAR), &(serde_json::to_string_pretty(&sidecar)? + "\n"))?;
        }
    }

    let have_local = plan
        .changes
        .iter()
        .any(|c| c.kind == Kind::Steering && c.level == Level::Local && matches!(&c.payload, Payload::Steering { path, .. } if path.is_file()));
    if have_local {
        for (file, pattern) in &plan.excludes {
            if add_line_once(file, pattern)? {
                report.excludes_added.push(file.clone());
            }
        }
    }
    Ok(report)
}

/// Copy a skill into place through a temporary folder so a failure never leaves
/// half a skill behind. Returns the backup location when an existing skill was replaced.
fn install_skill(dir: &Path, skill: &Skill, forced: bool) -> Result<Option<PathBuf>> {
    let kiro = dir.parent().and_then(Path::parent).ok_or_else(|| anyhow!("bad skill path {}", dir.display()))?;
    let tmp_root = kiro.join("migrate-to-kiro-tmp");
    let tmp = tmp_root.join(&skill.name);
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;

    fs::write(tmp.join("SKILL.md"), &skill.skill_md)?;
    for f in &skill.files {
        let to = tmp.join(&f.rel);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(&f.src, &to).with_context(|| format!("copy {}", f.src.display()))?;
    }
    let marker = json!({ "tool": skill.tool.slug(), "origin": skill.origin, "hash": format!("{:016x}", skill.hash) });
    fs::write(tmp.join(MARKER_FILE), serde_json::to_string_pretty(&marker)? + "\n")?;

    let mut backup = None;
    if dir.exists() {
        if forced {
            let dest = kiro.join("migrate-to-kiro-backup").join("skills").join(&skill.name);
            if dest.exists() {
                fs::remove_dir_all(&dest)?;
            }
            fs::create_dir_all(dest.parent().unwrap())?;
            fs::rename(dir, &dest)?;
            backup = Some(dest);
        } else {
            fs::remove_dir_all(dir)?;
        }
    }
    fs::create_dir_all(dir.parent().unwrap())?;
    fs::rename(&tmp, dir)?;
    let _ = fs::remove_dir(&tmp_root);
    Ok(backup)
}

fn add_line_once(file: &Path, line: &str) -> Result<bool> {
    let existing = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    if existing.lines().any(|l| l.trim() == line) {
        return Ok(false);
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    fs::write(file, out)?;
    Ok(true)
}
