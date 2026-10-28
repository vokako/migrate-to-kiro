//! Agent Skills → `~/.kiro/skills/` and `<project>/.kiro/skills/`.
//!
//! All five tools use the open Agent Skills layout (a folder with `SKILL.md`),
//! so a skill is copied as-is. Only the `name` field is touched, because Kiro
//! requires it to match the folder name.

use crate::model::{slug, Ctx, Inventory, Kind, Level, Severity, Tool};
use crate::text::frontmatter;
use crate::text::{fnv1a_parts, secrets};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const MARKER_FILE: &str = ".to-kiro.json";
const MAX_FILES: usize = 500;
const MAX_BYTES: u64 = 20 * 1024 * 1024;
const MAX_SCAN_BYTES: u64 = 512 * 1024;

/// Claude Code and CodeBuddy front matter fields Kiro does not act on.
const FOREIGN_FIELDS: [&str; 7] = [
    "allowed-tools", "context", "agent", "model", "hooks", "disable-model-invocation", "user-invocable",
];

#[derive(Debug, Clone)]
pub struct SkillFile {
    pub rel: String,
    pub src: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub tool: Tool,
    pub level: Level,
    /// Folder name in Kiro, equal to the `name` field.
    pub name: String,
    pub origin: String,
    /// Content of `SKILL.md` as it will be written (name field adjusted).
    pub skill_md: String,
    /// Every file except `SKILL.md`.
    pub files: Vec<SkillFile>,
    pub hash: u64,
}

impl Skill {
    pub fn file_count(&self) -> usize {
        self.files.len() + 1
    }
}

/// Hash of a skill folder. `entries` are `(relative path, bytes)`.
pub fn tree_hash(mut entries: Vec<(String, Vec<u8>)>) -> u64 {
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    fnv1a_parts(entries.iter().flat_map(|(p, b)| [p.as_bytes(), b.as_slice()]))
}

/// Hash of a folder on disk, ignoring our marker file.
pub fn dir_hash(dir: &Path) -> std::io::Result<u64> {
    let mut entries = Vec::new();
    for e in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()) {
        let rel = e.path().strip_prefix(dir).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        if rel == MARKER_FILE {
            continue;
        }
        entries.push((rel, fs::read(e.path())?));
    }
    Ok(tree_hash(entries))
}

fn roots(tool: Tool, ctx: &Ctx) -> Vec<(Level, PathBuf)> {
    let user: &[&str] = match tool {
        Tool::Cursor => &[".cursor/skills", ".agents/skills"],
        Tool::Copilot => &[".copilot/skills", ".agents/skills"],
        Tool::Qoder => &[".qoder/skills"],
        Tool::CodeBuddy => &[".codebuddy/skills"],
        Tool::Claude => &[".claude/skills"],
    };
    let project: &[&str] = match tool {
        Tool::Cursor => &[".cursor/skills", ".agents/skills"],
        Tool::Copilot => &[".github/skills", ".claude/skills", ".agents/skills"],
        Tool::Qoder => &[".qoder/skills"],
        Tool::CodeBuddy => &[".codebuddy/skills"],
        Tool::Claude => &[".claude/skills"],
    };
    let mut v = Vec::new();
    if ctx.include_user {
        v.extend(user.iter().map(|r| (Level::User, ctx.home.join(r))));
    }
    if let Some(p) = &ctx.project {
        v.extend(project.iter().map(|r| (Level::Project, p.join(r))));
    }
    v
}

/// Folders that contain a `SKILL.md`, outermost only, sorted.
fn skill_dirs(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.file_name() == "SKILL.md")
        .filter_map(|e| e.path().parent().map(Path::to_path_buf))
        .filter(|d| d != root)
        .collect();
    found.sort();
    let mut out: Vec<PathBuf> = Vec::new();
    for d in found {
        if !out.iter().any(|o| d.starts_with(o)) {
            out.push(d);
        }
    }
    out
}

/// Set the front matter `name`, adding the line when it is missing.
pub fn set_name(text: &str, name: &str) -> String {
    let (bom, t) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let mut lines: Vec<String> = t.split('\n').map(String::from).collect();
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return text.to_string();
    }
    let close = lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")).map(|i| i + 1);
    let Some(close) = close else { return text.to_string() };
    let eol = if lines[0].ends_with('\r') { "\r" } else { "" };
    let new_line = format!("name: {name}{eol}");
    match lines[1..close].iter().position(|l| l.starts_with("name:")) {
        Some(i) => lines[1 + i] = new_line,
        None => lines.insert(1, new_line),
    }
    format!("{bom}{}", lines.join("\n"))
}

pub fn discover(tool: Tool, ctx: &Ctx, inv: &mut Inventory) {
    for (level, root) in roots(tool, ctx) {
        if !root.is_dir() {
            continue;
        }
        let canon = root.canonicalize().unwrap_or_else(|_| root.clone());
        if !inv.seen.insert(canon) {
            continue; // `.agents/skills` and `.claude/skills` are shared by several tools
        }
        for dir in skill_dirs(&root) {
            read_skill(tool, level, ctx, &dir, inv);
        }
    }
}

fn skip(inv: &mut Inventory, tool: Tool, origin: String, why: impl Into<String>) {
    inv.note(Severity::Skipped, tool, Kind::Skill, Some(origin), why);
}

fn read_skill(tool: Tool, level: Level, ctx: &Ctx, dir: &Path, inv: &mut Inventory) {
    let origin = ctx.display(dir);
    let md_path = dir.join("SKILL.md");
    let text = match fs::read(&md_path).map(String::from_utf8) {
        Ok(Ok(t)) => t,
        Ok(Err(_)) => return skip(inv, tool, origin, "SKILL.md is not valid UTF-8"),
        Err(e) => return skip(inv, tool, origin, format!("cannot read SKILL.md: {e}")),
    };
    let (fm, body) = frontmatter::parse(&text);

    let Some(desc) = fm.str("description") else {
        return skip(inv, tool, origin, "SKILL.md has no `description`, which Kiro requires to decide when to use the skill");
    };
    if desc.chars().count() > 1024 {
        return skip(inv, tool, origin, "`description` is longer than the 1024 characters Kiro allows");
    }

    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("skill");
    let declared = fm.str("name").unwrap_or(dir_name);
    let mut name = slug(declared);
    name.truncate(64);
    let name = name.trim_end_matches('-').to_string();

    let mut notes: Vec<String> = Vec::new();
    if name != declared {
        notes.push(format!("name `{declared}` became `{name}` (Kiro allows lowercase letters, digits and hyphens)"));
    }

    // Files.
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut symlinks = 0usize;
    let mut leaks: Vec<String> = Vec::new();
    for e in WalkDir::new(dir).sort_by_file_name().into_iter().filter_entry(|e| e.file_name() != ".git").filter_map(|e| e.ok()) {
        if e.path_is_symlink() {
            symlinks += 1;
            continue;
        }
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e.path().strip_prefix(dir).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        if rel == "SKILL.md" || rel == MARKER_FILE {
            continue;
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        total += size;
        if total > MAX_BYTES || files.len() >= MAX_FILES {
            return skip(inv, tool, origin, format!("larger than {MAX_FILES} files or {} MiB", MAX_BYTES / 1024 / 1024));
        }
        if size <= MAX_SCAN_BYTES {
            if let Ok(Ok(t)) = fs::read(e.path()).map(String::from_utf8) {
                for (line, label) in secrets::scan(&t) {
                    leaks.push(format!("{rel} line {line} ({label})"));
                }
            }
        }
        files.push(SkillFile { rel, src: e.path().to_path_buf() });
    }
    for (line, label) in secrets::scan(&text) {
        leaks.push(format!("SKILL.md line {line} ({label})"));
    }
    if !leaks.is_empty() {
        leaks.truncate(5);
        return skip(inv, tool, origin, format!("looks like it contains a credential: {}. Remove it from the source and run again", leaks.join("; ")));
    }
    if symlinks > 0 {
        notes.push(format!("{symlinks} symbolic link(s) were not copied"));
    }

    let skill_md = if fm.str("name") == Some(name.as_str()) { text.clone() } else { set_name(&text, &name) };

    let foreign: Vec<&str> = FOREIGN_FIELDS.iter().copied().filter(|k| fm.has(k)).collect();
    if !foreign.is_empty() {
        notes.push(format!("front matter field(s) {} are not used by Kiro and are ignored", foreign.join(", ")));
    }
    let mut placeholders: Vec<String> = Vec::new();
    for prefix in ["${CLAUDE_", "${CODEBUDDY_"] {
        let mut rest = body;
        while let Some(i) = rest.find(prefix) {
            let tail = &rest[i..];
            let end = tail.find('}').map(|e| e + 1).unwrap_or(tail.len());
            let ph = tail[..end].to_string();
            if !placeholders.contains(&ph) {
                placeholders.push(ph);
            }
            rest = &tail[end..];
        }
    }
    if !placeholders.is_empty() {
        notes.push(format!("uses {}, which Kiro does not replace", placeholders.join(", ")));
    }
    if body.contains("!`") {
        notes.push("uses inline shell (!`command`), which Kiro does not run".into());
    }

    // Hash exactly what will be on disk.
    let mut entries = vec![("SKILL.md".to_string(), skill_md.clone().into_bytes())];
    for f in &files {
        match fs::read(&f.src) {
            Ok(b) => entries.push((f.rel.clone(), b)),
            Err(e) => return skip(inv, tool, origin, format!("cannot read {}: {e}", f.rel)),
        }
    }
    let hash = tree_hash(entries);

    if let Some(prev) = inv.skills.iter().find(|s| s.level == level && s.name == name) {
        if prev.hash != hash {
            let by = prev.origin.clone();
            skip(inv, tool, origin, format!("another skill named `{name}` was already taken from {by}; skipped to avoid a name clash"));
        }
        return;
    }

    for n in notes {
        inv.note(Severity::Lossy, tool, Kind::Skill, Some(origin.clone()), n);
    }
    inv.skills.push(Skill { tool, level, name, origin, skill_md, files, hash });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn set_name_replaces_or_inserts() {
        assert_eq!(set_name("---\nname: Old Name\ndescription: d\n---\nbody", "old-name"), "---\nname: old-name\ndescription: d\n---\nbody");
        assert_eq!(set_name("---\ndescription: d\n---\nbody", "x"), "---\nname: x\ndescription: d\n---\nbody");
        assert_eq!(set_name("---\r\nname: A\r\n---\r\nb", "a"), "---\r\nname: a\r\n---\r\nb");
        assert_eq!(set_name("no front matter", "x"), "no front matter");
    }

    #[test]
    fn nested_category_folders_and_name_fixes() {
        let h = tempfile::tempdir().unwrap();
        put(h.path(), ".codebuddy/skills/data/Analysis Tool/SKILL.md", "---\ndescription: Analyze data\nallowed-tools: Read\n---\nUse ${CODEBUDDY_SKILL_DIR}/x\n");
        put(h.path(), ".codebuddy/skills/data/Analysis Tool/scripts/run.sh", "echo hi\n");
        put(h.path(), ".codebuddy/skills/nodesc/SKILL.md", "---\nname: nodesc\n---\nbody\n");
        let ctx = Ctx::new(h.path().to_path_buf(), None, true);
        let mut inv = Inventory::default();
        discover(Tool::CodeBuddy, &ctx, &mut inv);

        assert_eq!(inv.skills.len(), 1);
        let s = &inv.skills[0];
        assert_eq!(s.name, "analysis-tool");
        assert!(s.skill_md.starts_with("---\nname: analysis-tool\n"), "{}", s.skill_md);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.file_count(), 2);
        let msgs: Vec<&str> = inv.findings.iter().map(|f| f.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("no `description`")));
        assert!(msgs.iter().any(|m| m.contains("allowed-tools")));
        assert!(msgs.iter().any(|m| m.contains("CODEBUDDY_SKILL_DIR")));
        assert!(msgs.iter().any(|m| m.contains("became `analysis-tool`")));
    }

    #[test]
    fn credential_blocks_the_whole_skill_and_shared_dirs_are_read_once() {
        let h = tempfile::tempdir().unwrap();
        put(h.path(), ".agents/skills/ok/SKILL.md", "---\nname: ok\ndescription: fine\n---\nhi\n");
        put(h.path(), ".agents/skills/leaky/SKILL.md", "---\nname: leaky\ndescription: d\n---\nhi\n");
        put(h.path(), ".agents/skills/leaky/scripts/deploy.sh", "export T=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n");
        let ctx = Ctx::new(h.path().to_path_buf(), None, true);
        let mut inv = Inventory::default();
        discover(Tool::Cursor, &ctx, &mut inv);
        discover(Tool::Copilot, &ctx, &mut inv); // same ~/.agents/skills
        assert_eq!(inv.skills.len(), 1);
        assert_eq!(inv.skills[0].name, "ok");
        let skipped: Vec<_> = inv.findings.iter().filter(|f| f.severity == Severity::Skipped).collect();
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        assert!(skipped[0].message.contains("scripts/deploy.sh line 1"));
        assert!(!skipped[0].message.contains("ghp_"));
    }

    #[test]
    fn same_name_different_content_is_a_clash_identical_is_merged() {
        let h = tempfile::tempdir().unwrap();
        put(h.path(), ".cursor/skills/pdf/SKILL.md", "---\nname: pdf\ndescription: a\n---\nA\n");
        put(h.path(), ".claude/skills/pdf/SKILL.md", "---\nname: pdf\ndescription: a\n---\nB\n");
        put(h.path(), ".qoder/skills/same/SKILL.md", "---\nname: same\ndescription: a\n---\nX\n");
        put(h.path(), ".claude/skills/same/SKILL.md", "---\nname: same\ndescription: a\n---\nX\n");
        let ctx = Ctx::new(h.path().to_path_buf(), None, true);
        let mut inv = Inventory::default();
        for t in [Tool::Claude, Tool::Cursor, Tool::Qoder] {
            discover(t, &ctx, &mut inv);
        }
        assert_eq!(inv.skills.iter().filter(|s| s.name == "pdf").count(), 1);
        assert_eq!(inv.skills.iter().filter(|s| s.name == "same").count(), 1);
        assert_eq!(inv.findings.iter().filter(|f| f.message.contains("name clash")).count(), 1);
    }
}
