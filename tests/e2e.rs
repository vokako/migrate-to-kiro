//! End-to-end: scan a fake machine with all five tools, build a plan, apply it,
//! and check the second run is a no-op and edits are protected.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use to_kiro::{apply, build, scan, Action, Ctx, Kind, Severity, Tool};

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

struct Machine {
    _h: tempfile::TempDir,
    _p: tempfile::TempDir,
    home: PathBuf,
    proj: PathBuf,
}

impl Machine {
    fn ctx(&self) -> Ctx {
        Ctx::new(self.home.clone(), Some(self.proj.clone()), true)
    }

    fn plan(&self) -> to_kiro::Plan {
        let ctx = self.ctx();
        build(scan(&Tool::ALL, &Kind::ALL, &ctx), &ctx)
    }
}

fn machine() -> Machine {
    let h = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let (home, proj) = (h.path().canonicalize().unwrap(), p.path().canonicalize().unwrap());
    fs::create_dir_all(proj.join(".git/info")).unwrap();

    // Steering.
    put(&proj, ".cursor/rules/react.mdc", "---\nglobs: **/*.tsx\nalwaysApply: false\n---\nUse hooks. See @docs/arch.md\n");
    put(&proj, ".github/copilot-instructions.md", "Be brief.\n");
    put(&proj, ".qoder/rules/db.md", "---\ntrigger: model_decision\ndescription: SQL work\n---\nIndex it.\n");
    put(&proj, ".codebuddy/rules/ts.md", "---\nalwaysApply: false\npaths: src/**/*.ts\n---\nStrict.\n");
    put(&proj, "CLAUDE.md", "Notes\n");
    put(&proj, "CLAUDE.local.md", "my sandbox\n");
    put(&proj, "docs/arch.md", "# Arch\n");
    put(&home, ".claude/CLAUDE.md", "Reply concisely.\n");

    // MCP.
    put(
        &proj,
        ".mcp.json",
        &json!({"mcpServers": {
            "github": {"command": "npx", "args": ["-y", "@github/mcp"], "env": {"GITHUB_TOKEN": "${GITHUB_TOKEN}"}},
            "docs": {"type": "http", "url": "https://docs.example.com/mcp"},
            "leaky": {"command": "x", "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_abcdefghijklmnopqrstuvwxyz0123456789"}},
            "socket": {"type": "ws", "url": "wss://x"}
        }})
        .to_string(),
    );
    put(
        &proj,
        ".cursor/mcp.json",
        &json!({"mcpServers": {
            "github": {"command": "npx", "args": ["-y", "@github/mcp"], "env": {"GITHUB_TOKEN": "${env:GITHUB_TOKEN}"}},
            "docs": {"url": "https://other.example.com/mcp"},
            "figma": {"url": "https://figma.example.com/mcp", "headers": {"X-Team": "a"}}
        }})
        .to_string(),
    );
    put(
        &proj,
        ".vscode/mcp.json",
        "{\n  // VS Code style\n  \"servers\": {\n    \"playwright\": {\"type\": \"stdio\", \"command\": \"npx\", \"args\": [\"@playwright/mcp\"],},\n    \"asks\": {\"command\": \"x\", \"args\": [\"${input:key}\"]},\n  },\n}\n",
    );
    put(
        &home,
        ".claude.json",
        &json!({
            "oauthAccount": {"emailAddress": "someone@example.com"},
            "mcpServers": {"user-srv": {"command": "uvx", "args": ["tool"]}},
            "projects": {
                proj.to_string_lossy(): {"mcpServers": {"local-srv": {"type": "http", "url": "https://local.example.com"}}},
                "/some/other/project": {"mcpServers": {"not-mine": {"command": "no"}}}
            }
        })
        .to_string(),
    );
    put(&home, ".qoder/settings.json", &json!({"model": "m", "mcpServers": {"q": {"command": "q", "alwaysAllow": ["t"]}}}).to_string());
    put(&home, ".codebuddy/mcp.json", &json!({"mcpServers": {"cb": {"command": "cb", "args": []}}}).to_string());
    // The user's own Kiro config must survive.
    put(&home, ".kiro/settings/mcp.json", &json!({"mcpServers": {"mine": {"command": "mine", "args": []}}, "other": 1}).to_string());

    // Skills.
    put(&proj, ".claude/skills/pdf/SKILL.md", "---\nname: pdf\ndescription: Work with PDFs\n---\nSee scripts/run.sh\n");
    put(&proj, ".claude/skills/pdf/scripts/run.sh", "#!/bin/sh\necho pdf\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(proj.join(".claude/skills/pdf/scripts/run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    put(&home, ".cursor/skills/review/SKILL.md", "---\nname: review\ndescription: Review code\n---\nbody\n");
    put(&proj, ".agents/skills/shared/SKILL.md", "---\nname: shared\ndescription: Shared by Cursor and Copilot\n---\nbody\n");

    Machine { _h: h, _p: p, home, proj }
}

fn find<'a>(plan: &'a to_kiro::Plan, kind: Kind, name: &str) -> &'a to_kiro::plan::Change {
    plan.changes
        .iter()
        .find(|c| c.kind == kind && c.name == name)
        .unwrap_or_else(|| panic!("no {kind:?} change named {name}: {:?}", plan.changes.iter().map(|c| (c.kind, &c.name)).collect::<Vec<_>>()))
}

#[test]
fn full_run_is_idempotent_and_protects_edits() {
    let m = machine();
    let plan = m.plan();

    // ── what was found ──
    let mcp: Vec<&str> = plan.changes.iter().filter(|c| c.kind == Kind::Mcp).map(|c| c.name.as_str()).collect();
    for want in ["github", "docs", "leaky", "figma", "playwright", "user-srv", "local-srv", "q", "cb"] {
        assert!(mcp.contains(&want), "missing {want} in {mcp:?}");
    }
    assert!(!mcp.contains(&"socket"), "ws transport must be skipped");
    assert!(!mcp.contains(&"asks"), "${{input:}} server must be skipped");
    assert!(!mcp.contains(&"not-mine"), "other projects' servers must not leak in");
    assert_eq!(mcp.iter().filter(|n| **n == "github").count(), 1, "identical server from two tools is imported once");

    let skills: Vec<&str> = plan.changes.iter().filter(|c| c.kind == Kind::Skill).map(|c| c.name.as_str()).collect();
    assert_eq!(skills.iter().filter(|n| **n == "shared").count(), 1, "shared folder read once");
    for want in ["pdf", "review", "shared"] {
        assert!(skills.contains(&want));
    }

    // ── notes ──
    let has = |sev: Severity, needle: &str| plan.findings.iter().any(|f| f.severity == sev && (f.message.contains(needle) || f.origin.as_deref().map_or(false, |o| o.contains(needle))));
    assert!(has(Severity::Skipped, "not supported by Kiro"));
    assert!(has(Severity::Skipped, "${input:"));
    assert!(has(Severity::Skipped, "name clash"));
    assert!(has(Severity::Lossy, "hard-coded credential"));
    assert!(has(Severity::Lossy, "private to this project"));
    assert!(plan.findings.iter().all(|f| !f.message.contains("ghp_")), "credential values must never be printed");

    // ── targets ──
    let local = find(&plan, Kind::Mcp, "local-srv");
    assert_eq!(local.level, to_kiro::Level::User);
    assert!(local.target.starts_with("~/.kiro/settings/mcp.json"), "{}", local.target);
    assert!(find(&plan, Kind::Mcp, "github").target.starts_with(".kiro/settings/mcp.json"));
    assert!(plan.changes.iter().all(|c| c.action == Action::Create));

    // ── apply ──
    let rep = apply(&plan, false).unwrap();
    assert!(!rep.written.is_empty());

    let project_cfg = read_json(&m.proj.join(".kiro/settings/mcp.json"));
    let servers = project_cfg["mcpServers"].as_object().unwrap();
    assert_eq!(servers["github"]["env"]["GITHUB_TOKEN"], "${GITHUB_TOKEN}");
    assert_eq!(servers["docs"]["url"], "https://docs.example.com/mcp");
    assert_eq!(servers["playwright"]["args"], json!(["@playwright/mcp"]));
    assert!(servers.get("local-srv").is_none(), "private servers stay out of the project file");

    let user_cfg = read_json(&m.home.join(".kiro/settings/mcp.json"));
    assert_eq!(user_cfg["other"], 1, "unrelated keys survive");
    let user_servers = user_cfg["mcpServers"].as_object().unwrap();
    assert!(user_servers.contains_key("mine"), "the user's own server survives");
    assert_eq!(user_servers["q"]["autoApprove"], json!(["t"]));
    assert!(user_servers.contains_key("user-srv") && user_servers.contains_key("local-srv") && user_servers.contains_key("cb"));

    let skill_dir = m.proj.join(".kiro/skills/pdf");
    assert!(skill_dir.join("SKILL.md").is_file() && skill_dir.join(".to-kiro.json").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(skill_dir.join("scripts/run.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "script keeps its executable bit");
    }
    assert!(m.home.join(".kiro/skills/review/SKILL.md").is_file());

    let steering = fs::read_to_string(m.proj.join(".kiro/steering/imported-cursor-react.md")).unwrap();
    assert!(steering.contains("#[[file:docs/arch.md]]"));
    assert!(fs::read_to_string(m.proj.join(".git/info/exclude")).unwrap().contains(".kiro/steering/imported-*-local-*.md"));

    // ── second run changes nothing ──
    let again = m.plan();
    assert!(
        again.changes.iter().all(|c| c.action == Action::Unchanged),
        "{:?}",
        again.changes.iter().filter(|c| c.action != Action::Unchanged).map(|c| (&c.name, &c.action)).collect::<Vec<_>>()
    );
    assert_eq!(again.pending(false), 0);
    let rep2 = apply(&again, false).unwrap();
    assert!(rep2.written.is_empty());

    // ── the source changes → unmodified imports update ──
    put(
        &m.proj,
        ".mcp.json",
        &json!({"mcpServers": {"github": {"command": "npx", "args": ["-y", "@github/mcp@2"], "env": {"GITHUB_TOKEN": "${GITHUB_TOKEN}"}}}}).to_string(),
    );
    put(&m.proj, ".claude/skills/pdf/SKILL.md", "---\nname: pdf\ndescription: Work with PDFs v2\n---\nnew\n");
    let p3 = m.plan();
    assert_eq!(find(&p3, Kind::Mcp, "github").action, Action::Update);
    assert_eq!(find(&p3, Kind::Skill, "pdf").action, Action::Update);

    // ── the user edits imports → they become conflicts ──
    let mut cfg = read_json(&m.proj.join(".kiro/settings/mcp.json"));
    cfg["mcpServers"]["docs"]["url"] = json!("https://my-own.example.com");
    fs::write(m.proj.join(".kiro/settings/mcp.json"), cfg.to_string()).unwrap();
    fs::write(m.home.join(".kiro/skills/review/notes.txt"), "mine").unwrap();
    let react = m.proj.join(".kiro/steering/imported-cursor-react.md");
    fs::write(&react, format!("{}my addition\n", fs::read_to_string(&react).unwrap())).unwrap();

    let p4 = m.plan();
    assert!(matches!(find(&p4, Kind::Mcp, "docs").action, Action::Conflict(_)));
    assert!(matches!(find(&p4, Kind::Skill, "review").action, Action::Conflict(_)));
    assert!(matches!(find(&p4, Kind::Steering, "react").action, Action::Conflict(_)));
    // …while the unmodified neighbours still update.
    assert_eq!(find(&p4, Kind::Mcp, "github").action, Action::Update);

    let safe = apply(&p4, false).unwrap();
    assert_eq!(safe.skipped_conflicts, 3);
    assert_eq!(read_json(&m.proj.join(".kiro/settings/mcp.json"))["mcpServers"]["docs"]["url"], "https://my-own.example.com");
    assert!(m.home.join(".kiro/skills/review/notes.txt").is_file());
    assert!(fs::read_to_string(&react).unwrap().contains("my addition"));
    assert_eq!(read_json(&m.proj.join(".kiro/settings/mcp.json"))["mcpServers"]["github"]["args"][1], "@github/mcp@2");

    // ── --force overwrites and keeps backups ──
    let p5 = m.plan();
    let forced = apply(&p5, true).unwrap();
    assert!(!forced.backups.is_empty());
    // `.mcp.json` no longer defines `docs` (rewritten above), so Cursor's definition is the source now.
    assert_eq!(read_json(&m.proj.join(".kiro/settings/mcp.json"))["mcpServers"]["docs"]["url"], "https://other.example.com/mcp");
    assert!(!m.home.join(".kiro/skills/review/notes.txt").exists());
    assert!(m.home.join(".kiro/to-kiro-backup/skills/review/notes.txt").is_file());
    assert!(fs::read_to_string(react.with_extension("md.bak")).unwrap().contains("my addition"));
    // No stray temp folders next to the skills.
    assert!(!m.proj.join(".kiro/to-kiro-tmp").exists());
}

#[test]
fn invalid_existing_mcp_json_is_never_overwritten() {
    let m = machine();
    put(&m.proj, ".kiro/settings/mcp.json", "{ this is not json");
    let plan = m.plan();
    let github = find(&plan, Kind::Mcp, "github");
    assert!(matches!(&github.action, Action::Conflict(why) if why.contains("not valid JSON")), "{:?}", github.action);
    apply(&plan, false).unwrap();
    assert_eq!(fs::read_to_string(m.proj.join(".kiro/settings/mcp.json")).unwrap(), "{ this is not json");
}

#[test]
fn restricting_tools_and_kinds_limits_what_is_read() {
    let m = machine();
    let ctx = m.ctx();
    let plan = build(scan(&[Tool::Cursor], &[Kind::Mcp], &ctx), &ctx);
    assert!(plan.changes.iter().all(|c| c.kind == Kind::Mcp && c.tool == Tool::Cursor));
    let names: Vec<_> = plan.changes.iter().map(|c| c.name.as_str()).collect();
    // `.mcp.json` belongs to Claude/CodeBuddy/Qoder, so Cursor alone only sees its own file.
    assert!(names.contains(&"figma") && names.contains(&"github"));
    assert!(!names.contains(&"leaky"));
}

#[test]
fn without_user_nothing_under_home_is_read_or_written() {
    let m = machine();
    let ctx = Ctx::new(m.home.clone(), Some(m.proj.clone()), false);
    let plan = build(scan(&Tool::ALL, &Kind::ALL, &ctx), &ctx);
    assert!(plan.changes.iter().all(|c| c.level != to_kiro::Level::User), "{:?}", plan.changes.iter().map(|c| &c.target).collect::<Vec<_>>());
    let before = read_json(&m.home.join(".kiro/settings/mcp.json"));
    apply(&plan, false).unwrap();
    assert_eq!(read_json(&m.home.join(".kiro/settings/mcp.json")), before);
}
