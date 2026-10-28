//! One adapter per source tool. Each adapter reads only the locations that the
//! tool's own documentation names, and maps the tool's activation mode to the
//! closest Kiro steering inclusion mode.

use super::{Inclusion, Rule};
use crate::model::{slug, Ctx, Inventory, Kind, Level, Severity, Tool};
use crate::text::frontmatter::{self, Front};
use crate::text::{imports, secrets};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ── helpers ──────────────────────────────────────────────────────────────

fn note(d: &mut Inventory, severity: Severity, tool: Tool, origin: Option<String>, message: impl Into<String>) {
    d.note(severity, tool, Kind::Steering, origin, message);
}

fn md_files(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    if !dir.is_dir() {
        return vec![];
    }
    WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map_or(false, |x| exts.iter().any(|y| x.eq_ignore_ascii_case(y)))
        })
        .map(|e| e.into_path())
        .collect()
}

fn read(ctx: &Ctx, d: &mut Inventory, tool: Tool, path: &Path) -> Option<String> {
    match fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => Some(s),
            Err(_) => {
                note(d, Severity::Skipped, tool, Some(ctx.display(path)), "not valid UTF-8");
                None
            }
        },
        Err(e) => {
            note(d, Severity::Skipped, tool, Some(ctx.display(path)), format!("cannot read: {e}"));
            None
        }
    }
}

/// Name for a file below `root`: its relative path without extension, slugged.
fn name_from(root: &Path, file: &Path, strip_suffix: Option<&str>) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file).to_string_lossy().into_owned();
    let rel = match strip_suffix {
        Some(s) => rel.strip_suffix(s).unwrap_or(&rel).to_string(),
        None => match rel.rfind('.') {
            Some(i) => rel[..i].to_string(),
            None => rel,
        },
    };
    slug(&rel)
}

struct Draft<'a> {
    tool: Tool,
    level: Level,
    path: &'a Path,
    name: String,
    description: Option<String>,
    inclusion: Inclusion,
    body: &'a str,
    /// Extra lossy-conversion notes found by the adapter.
    notes: Vec<String>,
}

fn emit(ctx: &Ctx, d: &mut Inventory, dr: Draft) {
    let origin = ctx.display(dr.path);
    if dr.body.trim().is_empty() {
        note(d, Severity::Skipped, dr.tool, Some(origin), "empty");
        return;
    }

    let hits = secrets::scan(dr.body);
    if !hits.is_empty() {
        let list: Vec<String> = hits.iter().map(|(n, l)| format!("line {n} ({l})")).collect();
        note(d, 
            Severity::Skipped,
            dr.tool,
            Some(origin),
            format!("looks like it contains a credential: {}. Remove it from the source file and run again", list.join(", ")),
        );
        return;
    }

    let mut inclusion = dr.inclusion;
    let mut notes = dr.notes;
    if inclusion == Inclusion::Auto && dr.description.is_none() {
        inclusion = Inclusion::Manual;
        notes.push("no description, so Kiro cannot choose it automatically; set to manual".into());
    }

    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(parent) = dr.path.parent() {
        bases.push(parent.to_path_buf());
    }
    if let Some(p) = &ctx.project {
        bases.push(p.clone());
    }
    let ws = if dr.level == Level::User { None } else { ctx.project.as_deref() };
    let rewritten = imports::rewrite(dr.body, &bases, ws, &ctx.home);
    notes.extend(rewritten.notes);

    for n in notes {
        note(d, Severity::Lossy, dr.tool, Some(origin.clone()), n);
    }

    let mut name = dr.name;
    let base = name.clone();
    let mut n = 2;
    while d.rules.iter().any(|r| r.tool == dr.tool && r.level == dr.level && r.name == name) {
        name = format!("{base}-{n}");
        n += 1;
    }

    d.rules.push(Rule {
        tool: dr.tool,
        level: dr.level,
        name,
        description: dr.description,
        inclusion,
        body: rewritten.body,
        origin,
    });
}

fn native_agents_md(ctx: &Ctx, d: &mut Inventory, tool: Tool) {
    if let Some(p) = &ctx.project {
        let f = p.join("AGENTS.md");
        if f.is_file() {
            let origin = ctx.display(&f);
            // Several tools share the same AGENTS.md; report it once.
            if d.findings.iter().any(|x| x.origin.as_deref() == Some(origin.as_str()) && x.severity == Severity::Info) {
                return;
            }
            note(d, 
                Severity::Info,
                tool,
                Some(origin),
                "Kiro reads AGENTS.md natively (always included); nothing to migrate",
            );
        }
    }
}

fn globs_are_everything(globs: &[String]) -> bool {
    globs.iter().any(|g| matches!(g.as_str(), "*" | "**" | "**/*"))
}

// ── Cursor ───────────────────────────────────────────────────────────────

pub fn cursor(ctx: &Ctx, d: &mut Inventory) {
    let t = Tool::Cursor;
    if let Some(p) = &ctx.project {
        let dir = p.join(".cursor").join("rules");
        for f in md_files(&dir, &["mdc", "md"]) {
            let Some(text) = read(ctx, d, t, &f) else { continue };
            let (fm, body) = frontmatter::parse(&text);
            let globs = fm.items("globs");
            let desc = fm.str("description").map(String::from);
            let inclusion = if fm.bool("alwaysApply") == Some(true) {
                Inclusion::Always
            } else if !globs.is_empty() {
                Inclusion::FileMatch(globs)
            } else if desc.is_some() {
                Inclusion::Auto
            } else {
                Inclusion::Manual
            };
            emit(
                ctx,
                d,
                Draft {
                    tool: t,
                    level: Level::Project,
                    path: &f,
                    name: name_from(&dir, &f, None),
                    description: desc,
                    inclusion,
                    body,
                    notes: vec![],
                },
            );
        }

        let legacy = p.join(".cursorrules");
        if legacy.is_file() {
            if let Some(text) = read(ctx, d, t, &legacy) {
                emit(
                    ctx,
                    d,
                    Draft {
                        tool: t,
                        level: Level::Project,
                        path: &legacy,
                        name: "cursorrules".into(),
                        description: None,
                        inclusion: Inclusion::Always,
                        body: &text,
                        notes: vec![],
                    },
                );
            }
        }
        native_agents_md(ctx, d, t);
    }

    if ctx.include_user {
        note(d, 
            Severity::Info,
            t,
            None,
            "Cursor User Rules and Memories are kept in Cursor's settings, not in a documented file, so they cannot be read from disk. Copy them from Cursor Settings → Rules into ~/.kiro/steering/ by hand",
        );
    }
}

// ── GitHub Copilot ───────────────────────────────────────────────────────

pub fn copilot(ctx: &Ctx, d: &mut Inventory) {
    let t = Tool::Copilot;
    if let Some(p) = &ctx.project {
        let main = p.join(".github").join("copilot-instructions.md");
        if main.is_file() {
            if let Some(text) = read(ctx, d, t, &main) {
                emit(
                    ctx,
                    d,
                    Draft {
                        tool: t,
                        level: Level::Project,
                        path: &main,
                        name: "instructions".into(),
                        description: None,
                        inclusion: Inclusion::Always,
                        body: &text,
                        notes: vec![],
                    },
                );
            }
        }

        let idir = p.join(".github").join("instructions");
        for f in md_files(&idir, &["md"]) {
            let fname = f.file_name().and_then(|x| x.to_str()).unwrap_or("");
            if !fname.ends_with(".instructions.md") {
                continue;
            }
            let Some(text) = read(ctx, d, t, &f) else { continue };
            let (fm, body) = frontmatter::parse(&text);
            let apply_to = fm.items("applyTo");
            let mut notes = vec![];
            let inclusion = if apply_to.is_empty() {
                notes.push("no applyTo, so Copilot only uses it when attached by hand; set to manual".into());
                Inclusion::Manual
            } else if globs_are_everything(&apply_to) {
                Inclusion::Always
            } else {
                Inclusion::FileMatch(apply_to)
            };
            if fm.str("excludeAgent").is_some() {
                notes.push("excludeAgent has no Kiro equivalent and was dropped".into());
            }
            emit(
                ctx,
                d,
                Draft {
                    tool: t,
                    level: Level::Project,
                    path: &f,
                    name: name_from(&idir, &f, Some(".instructions.md")),
                    description: fm.str("description").map(String::from),
                    inclusion,
                    body,
                    notes,
                },
            );
        }

        let pdir = p.join(".github").join("prompts");
        for f in md_files(&pdir, &["md"]) {
            let fname = f.file_name().and_then(|x| x.to_str()).unwrap_or("");
            if !fname.ends_with(".prompt.md") {
                continue;
            }
            let Some(text) = read(ctx, d, t, &f) else { continue };
            let (fm, body) = frontmatter::parse(&text);
            let mut notes = vec!["prompt file → manual steering; use it with #<name> in chat".to_string()];
            if !fm.is_empty() {
                notes.push("prompt front matter (mode, tools, model) was dropped".into());
            }
            emit(
                ctx,
                d,
                Draft {
                    tool: t,
                    level: Level::Project,
                    path: &f,
                    name: format!("prompt-{}", name_from(&pdir, &f, Some(".prompt.md"))),
                    description: fm.str("description").map(String::from),
                    inclusion: Inclusion::Manual,
                    body,
                    notes,
                },
            );
        }
        native_agents_md(ctx, d, t);
    }

    if ctx.include_user {
        note(d, 
            Severity::Info,
            t,
            None,
            "Personal and organization custom instructions (and Copilot Memory) are managed on github.com, not in local files, so they cannot be read from disk. Copy them into ~/.kiro/steering/ by hand",
        );
    }
}

// ── Qoder ────────────────────────────────────────────────────────────────

fn qoder_inclusion(fm: &Front) -> (Inclusion, Option<String>) {
    let mut globs = fm.items("glob");
    if globs.is_empty() {
        globs = fm.items("paths");
    }
    let has_desc = fm.str("description").is_some();
    match fm.str("trigger") {
        Some("always_on") => (Inclusion::Always, None),
        Some("manual") => (Inclusion::Manual, None),
        Some("model_decision") if has_desc => (Inclusion::Auto, None),
        Some("model_decision") => (
            Inclusion::Manual,
            Some("trigger model_decision without a description is never injected by Qoder; set to manual".into()),
        ),
        Some("glob") if !globs.is_empty() => (Inclusion::FileMatch(globs), None),
        Some("glob") => (
            Inclusion::Manual,
            Some("trigger glob without a glob pattern is never injected by Qoder; set to manual".into()),
        ),
        Some(other) => (Inclusion::Manual, Some(format!("unknown trigger `{other}`; set to manual"))),
        None if !globs.is_empty() => (Inclusion::FileMatch(globs), None),
        None => match fm.bool("alwaysApply") {
            Some(false) => (Inclusion::Manual, None),
            _ => (Inclusion::Always, None),
        },
    }
}

pub fn qoder(ctx: &Ctx, d: &mut Inventory) {
    let t = Tool::Qoder;
    let mut roots: Vec<(Level, PathBuf)> = Vec::new();
    if ctx.include_user {
        roots.push((Level::User, ctx.home.join(".qoder")));
    }
    if let Some(p) = &ctx.project {
        roots.push((Level::Project, p.join(".qoder")));
    }

    for (level, root) in &roots {
        let dir = root.join("rules");
        for f in md_files(&dir, &["md"]) {
            let Some(text) = read(ctx, d, t, &f) else { continue };
            let (fm, body) = frontmatter::parse(&text);
            let (inclusion, note) = qoder_inclusion(&fm);
            emit(
                ctx,
                d,
                Draft {
                    tool: t,
                    level: *level,
                    path: &f,
                    name: name_from(&dir, &f, None),
                    description: fm.str("description").map(String::from),
                    inclusion,
                    body,
                    notes: note.into_iter().collect(),
                },
            );
        }
    }

    if ctx.include_user {
        let f = ctx.home.join(".qoder").join("AGENTS.md");
        if f.is_file() {
            if let Some(text) = read(ctx, d, t, &f) {
                emit(
                    ctx,
                    d,
                    Draft {
                        tool: t,
                        level: Level::User,
                        path: &f,
                        name: "agents".into(),
                        description: None,
                        inclusion: Inclusion::Always,
                        body: &text,
                        notes: vec![],
                    },
                );
            }
        }
    }

    if let Some(p) = &ctx.project {
        let f = p.join("AGENTS.local.md");
        if f.is_file() {
            if let Some(text) = read(ctx, d, t, &f) {
                emit(
                    ctx,
                    d,
                    Draft {
                        tool: t,
                        level: Level::Local,
                        path: &f,
                        name: "agents".into(),
                        description: None,
                        inclusion: Inclusion::Always,
                        body: &text,
                        notes: vec![],
                    },
                );
            }
        }
        native_agents_md(ctx, d, t);
    }
}

// ── CodeBuddy ────────────────────────────────────────────────────────────

/// `None` means CodeBuddy itself would not load the file.
fn codebuddy_inclusion(fm: &Front) -> Result<Inclusion, &'static str> {
    if fm.bool("enabled") == Some(false) {
        return Err("`enabled: false`, so CodeBuddy does not load it");
    }
    if fm.bool("alwaysApply").unwrap_or(true) {
        return Ok(Inclusion::Always);
    }
    let paths = fm.items("paths");
    if paths.is_empty() {
        return Err("`alwaysApply: false` without `paths`, so CodeBuddy does not load it");
    }
    Ok(Inclusion::FileMatch(paths))
}

fn codebuddy_file(ctx: &Ctx, d: &mut Inventory, level: Level, path: &Path, name: String, root_for_name: Option<&Path>) {
    let t = Tool::CodeBuddy;
    let Some(text) = read(ctx, d, t, path) else { return };
    let (fm, body) = frontmatter::parse(&text);
    match codebuddy_inclusion(&fm) {
        Ok(inclusion) => {
            let name = match root_for_name {
                Some(r) => name_from(r, path, None),
                None => name,
            };
            emit(
                ctx,
                d,
                Draft {
                    tool: t,
                    level,
                    path,
                    name,
                    description: fm.str("description").map(String::from),
                    inclusion,
                    body,
                    notes: vec![],
                },
            );
        }
        Err(why) => note(d, Severity::Skipped, t, Some(ctx.display(path)), why),
    }
}

pub fn codebuddy(ctx: &Ctx, d: &mut Inventory) {
    if ctx.include_user {
        let root = ctx.home.join(".codebuddy");
        let main = root.join("CODEBUDDY.md");
        if main.is_file() {
            codebuddy_file(ctx, d, Level::User, &main, "instructions".into(), None);
        }
        let dir = root.join("rules");
        for f in md_files(&dir, &["md"]) {
            codebuddy_file(ctx, d, Level::User, &f, String::new(), Some(&dir));
        }
    }
    if let Some(p) = &ctx.project {
        for main in [p.join("CODEBUDDY.md"), p.join(".codebuddy").join("CODEBUDDY.md")] {
            if main.is_file() {
                codebuddy_file(ctx, d, Level::Project, &main, "instructions".into(), None);
            }
        }
        let dir = p.join(".codebuddy").join("rules");
        for f in md_files(&dir, &["md"]) {
            codebuddy_file(ctx, d, Level::Project, &f, String::new(), Some(&dir));
        }
        let local = p.join("CODEBUDDY.local.md");
        if local.is_file() {
            codebuddy_file(ctx, d, Level::Local, &local, "instructions".into(), None);
        }
        native_agents_md(ctx, d, Tool::CodeBuddy);
    }
}

// ── Claude Code ──────────────────────────────────────────────────────────

/// Claude Code names a project's directory by replacing every non-alphanumeric
/// character of the absolute path with `-` (`/tmp/app` → `-tmp-app`).
pub fn claude_project_dir_name(project: &Path) -> String {
    project
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn claude_instruction(ctx: &Ctx, d: &mut Inventory, level: Level, path: &Path, name: &str) {
    let t = Tool::Claude;
    let Some(text) = read(ctx, d, t, path) else { return };
    let (fm, body) = frontmatter::parse(&text);
    let paths = fm.items("paths");
    let inclusion = if paths.is_empty() { Inclusion::Always } else { Inclusion::FileMatch(paths) };
    emit(
        ctx,
        d,
        Draft {
            tool: t,
            level,
            path,
            name: name.into(),
            description: fm.str("description").map(String::from),
            inclusion,
            body,
            notes: vec![],
        },
    );
}

pub fn claude(ctx: &Ctx, d: &mut Inventory) {
    let t = Tool::Claude;
    if ctx.include_user {
        let root = ctx.home.join(".claude");
        let main = root.join("CLAUDE.md");
        if main.is_file() {
            claude_instruction(ctx, d, Level::User, &main, "instructions");
        }
        let dir = root.join("rules");
        for f in md_files(&dir, &["md"]) {
            claude_instruction(ctx, d, Level::User, &f, &name_from(&dir, &f, None));
        }
    }

    if let Some(p) = &ctx.project {
        for main in [p.join("CLAUDE.md"), p.join(".claude").join("CLAUDE.md")] {
            if main.is_file() {
                claude_instruction(ctx, d, Level::Project, &main, "instructions");
            }
        }
        let dir = p.join(".claude").join("rules");
        for f in md_files(&dir, &["md"]) {
            claude_instruction(ctx, d, Level::Project, &f, &name_from(&dir, &f, None));
        }
        let local = p.join("CLAUDE.local.md");
        if local.is_file() {
            claude_instruction(ctx, d, Level::Local, &local, "instructions");
        }
        native_agents_md(ctx, d, t);

        if ctx.include_user {
            claude_auto_memory(ctx, d, p);
        }
    }
}

fn claude_auto_memory(ctx: &Ctx, d: &mut Inventory, project: &Path) {
    let t = Tool::Claude;
    let dir = ctx
        .home
        .join(".claude")
        .join("projects")
        .join(claude_project_dir_name(project))
        .join("memory");
    if !dir.is_dir() {
        note(d, 
            Severity::Info,
            t,
            None,
            format!("no Claude Code auto memory for this project at {}", ctx.display(&dir)),
        );
        return;
    }
    for f in md_files(&dir, &["md"]) {
        let fname = f.file_name().and_then(|x| x.to_str()).unwrap_or("");
        if fname == "MEMORY.md" {
            note(d, 
                Severity::Info,
                t,
                Some(ctx.display(&f)),
                "index file; its topic files are migrated one by one",
            );
            continue;
        }
        let Some(text) = read(ctx, d, t, &f) else { continue };
        let (fm, body) = frontmatter::parse(&text);
        // `user` and `feedback` memories describe how to work with this person;
        // `project` and `reference` memories are facts to look up when relevant.
        let inclusion = match fm.str("type") {
            Some("user") | Some("feedback") => Inclusion::Always,
            _ => Inclusion::Auto,
        };
        let stem = fm.str("name").map(slug).unwrap_or_else(|| name_from(&dir, &f, None));
        emit(
            ctx,
            d,
            Draft {
                tool: t,
                level: Level::Local,
                path: &f,
                name: format!("memory-{stem}"),
                description: fm.str("description").map(String::from),
                inclusion,
                body,
                notes: vec![],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_dir_name_matches_observed_layout() {
        assert_eq!(claude_project_dir_name(Path::new("/tmp/app")), "-tmp-app");
        assert_eq!(claude_project_dir_name(Path::new("/home/a.b/my_proj")), "-home-a-b-my-proj");
    }

    #[test]
    fn codebuddy_rules() {
        let (fm, _) = frontmatter::parse("---\nenabled: false\n---\nx");
        assert!(codebuddy_inclusion(&fm).is_err());
        let (fm, _) = frontmatter::parse("---\nalwaysApply: false\n---\nx");
        assert!(codebuddy_inclusion(&fm).is_err());
        let (fm, _) = frontmatter::parse("---\nalwaysApply: false\npaths: src/**/*.ts\n---\nx");
        assert_eq!(codebuddy_inclusion(&fm), Ok(Inclusion::FileMatch(vec!["src/**/*.ts".into()])));
        let (fm, _) = frontmatter::parse("no front matter");
        assert_eq!(codebuddy_inclusion(&fm), Ok(Inclusion::Always));
    }

    #[test]
    fn qoder_triggers() {
        let case = |s: &str| qoder_inclusion(&frontmatter::parse(s).0);
        assert_eq!(case("---\ntrigger: always_on\n---\nx").0, Inclusion::Always);
        assert_eq!(case("---\ntrigger: manual\n---\nx").0, Inclusion::Manual);
        assert_eq!(case("---\ntrigger: model_decision\ndescription: d\n---\nx").0, Inclusion::Auto);
        assert!(case("---\ntrigger: model_decision\n---\nx").1.is_some());
        assert_eq!(case("---\npaths:\n  - a/**\n---\nx").0, Inclusion::FileMatch(vec!["a/**".into()]));
        assert_eq!(case("---\nalwaysApply: false\n---\nx").0, Inclusion::Manual);
        assert_eq!(case("plain").0, Inclusion::Always);
    }
}
