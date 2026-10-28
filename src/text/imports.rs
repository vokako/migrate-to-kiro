//! Rewrite `@path` file imports (Claude Code, CodeBuddy, Cursor) to Kiro's
//! `#[[file:<workspace-relative path>]]` references.
//!
//! A token is only treated as an import when it resolves to an existing file, so
//! handles, emails and package names such as `@scope/pkg` are left alone.

use std::path::{Path, PathBuf};

pub struct Rewrite {
    pub body: String,
    pub notes: Vec<String>,
}

/// `bases`: directories tried for relative tokens, in order.
/// `workspace`: when set, imports that resolve inside it become `#[[file:..]]`.
pub fn rewrite(body: &str, bases: &[PathBuf], workspace: Option<&Path>, home: &Path) -> Rewrite {
    let ws_canon = workspace.and_then(|w| w.canonicalize().ok());
    let mut notes = Vec::new();
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;

    for line in body.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        out.push_str(&rewrite_line(line, bases, ws_canon.as_deref(), home, &mut notes));
    }
    notes.sort();
    notes.dedup();
    Rewrite { body: out, notes }
}

fn is_token_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, ')' | ']' | ',' | '>' | '"' | '\'' | '`')
}

fn rewrite_line(line: &str, bases: &[PathBuf], ws: Option<&Path>, home: &Path, notes: &mut Vec<String>) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            in_code = !in_code;
            out.push(c);
            i += 1;
            continue;
        }
        let at_boundary = i == 0 || chars[i - 1].is_whitespace() || matches!(chars[i - 1], '(' | '[');
        if c == '@' && !in_code && at_boundary {
            let mut j = i + 1;
            while j < chars.len() && !is_token_end(chars[j]) {
                j += 1;
            }
            let raw: String = chars[i + 1..j].iter().collect();
            let token = raw.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let trailing = &raw[token.len()..];
            if let Some(found) = resolve(token, bases, home) {
                match inside(ws, &found) {
                    Some(rel) => {
                        out.push_str(&format!("#[[file:{rel}]]"));
                        out.push_str(trailing);
                    }
                    None => {
                        notes.push(format!(
                            "import `@{token}` points outside the workspace and was left as text (Kiro file references are workspace-relative)"
                        ));
                        out.push('@');
                        out.push_str(&raw);
                    }
                }
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn resolve(token: &str, bases: &[PathBuf], home: &Path) -> Option<PathBuf> {
    if token.is_empty() {
        return None;
    }
    let candidates: Vec<PathBuf> = if let Some(rest) = token.strip_prefix("~/") {
        vec![home.join(rest)]
    } else if Path::new(token).is_absolute() {
        vec![PathBuf::from(token)]
    } else {
        bases.iter().map(|b| b.join(token)).collect()
    };
    candidates.into_iter().find(|p| p.is_file())
}

fn inside(ws: Option<&Path>, file: &Path) -> Option<String> {
    let ws = ws?;
    let canon = file.canonicalize().ok()?;
    let rel = canon.strip_prefix(ws).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rewrites_existing_files_only() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path();
        fs::create_dir_all(ws.join("docs")).unwrap();
        fs::write(ws.join("docs/git.md"), "x").unwrap();
        let body = "See @docs/git.md, ping @alice and `@docs/git.md` and @scope/pkg.\n```\n@docs/git.md\n```\n";
        let r = rewrite(body, &[ws.to_path_buf()], Some(ws), d.path());
        assert!(r.body.contains("See #[[file:docs/git.md]], ping @alice"), "{}", r.body);
        assert!(r.body.contains("`@docs/git.md`"));
        assert!(r.body.contains("```\n@docs/git.md\n```"));
        assert!(r.body.contains("@scope/pkg"));
        assert!(r.notes.is_empty());
    }

    #[test]
    fn outside_workspace_is_noted_not_rewritten() {
        let ws = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("mine.md"), "x").unwrap();
        let r = rewrite("- @~/mine.md\n", &[ws.path().to_path_buf()], Some(ws.path()), home.path());
        assert_eq!(r.body, "- @~/mine.md\n");
        assert_eq!(r.notes.len(), 1);
    }
}
