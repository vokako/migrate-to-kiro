//! Render a [`Rule`] as a Kiro steering file.
//!
//! Every file carries a marker comment with a content hash. A later run may
//! overwrite a file only while the hash still matches, so edits made after the
//! import and steering files written by hand are never clobbered without
//! `--force`.

use super::{Inclusion, Rule};
use crate::model::Level;
use crate::text::fnv1a;

pub const MARKER: &str = "<!-- to-kiro:imported ";

fn yaml_str(s: &str) -> String {
    // A JSON string is a valid YAML double-quoted scalar.
    serde_json::to_string(&s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_else(|_| "\"\"".into())
}

fn front_matter(rule: &Rule) -> String {
    let mut s = String::from("---\n");
    match &rule.inclusion {
        Inclusion::Always => s.push_str("inclusion: always\n"),
        Inclusion::Manual => s.push_str("inclusion: manual\n"),
        Inclusion::FileMatch(globs) => {
            s.push_str("inclusion: fileMatch\n");
            if globs.len() == 1 {
                s.push_str(&format!("fileMatchPattern: {}\n", yaml_str(&globs[0])));
            } else {
                let items: Vec<String> = globs.iter().map(|g| yaml_str(g)).collect();
                s.push_str(&format!("fileMatchPattern: [{}]\n", items.join(", ")));
            }
        }
        Inclusion::Auto => {
            s.push_str("inclusion: auto\n");
            s.push_str(&format!("name: {}\n", yaml_str(&rule.name)));
            s.push_str(&format!("description: {}\n", yaml_str(rule.description.as_deref().unwrap_or(""))));
        }
    }
    s.push_str("---\n");
    s
}

pub fn file_name(rule: &Rule) -> String {
    let local = if rule.level == Level::Local { "local-" } else { "" };
    format!("imported-{}-{}{}.md", rule.tool.slug(), local, rule.name)
}

pub fn render(rule: &Rule) -> String {
    let head = front_matter(rule);
    let mut body = rule.body.trim_start_matches(['\r', '\n']).trim_end().to_string();
    body.push('\n');
    let hash = fnv1a(format!("{head}{body}").as_bytes());
    let origin = rule.origin.replace("-->", "->");
    format!("{head}{MARKER}tool={} origin={origin} hash={hash:016x} -->\n\n{body}", rule.tool.slug())
}

/// `(recorded hash, front matter + body as originally hashed)`.
pub fn strip_marker(existing: &str) -> Option<(u64, String)> {
    let start = if existing.starts_with(MARKER) {
        0
    } else {
        existing.find(&format!("\n{MARKER}"))? + 1
    };
    let line_end = existing[start..].find('\n').map(|i| start + i)?;
    let line = &existing[start..line_end];
    let hash_at = line.rfind(" hash=")?;
    let hex = line[hash_at + 6..].trim_end_matches("-->").trim();
    let recorded = u64::from_str_radix(hex, 16).ok()?;
    let after = &existing[line_end + 1..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    Some((recorded, format!("{}{}", &existing[..start], after)))
}

/// Written by this tool and not edited since.
pub fn is_untouched(existing: &str) -> Option<bool> {
    let (recorded, payload) = strip_marker(existing)?;
    Some(fnv1a(payload.as_bytes()) == recorded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tool;

    fn rule(inclusion: Inclusion) -> Rule {
        Rule {
            tool: Tool::Cursor,
            level: Level::Project,
            name: "r".into(),
            description: Some("when editing \"x\"\nnow".into()),
            inclusion,
            body: "\n\nbody\n\n".into(),
            origin: ".cursor/rules/r.mdc".into(),
        }
    }

    #[test]
    fn front_matter_modes() {
        assert!(render(&rule(Inclusion::Always)).starts_with("---\ninclusion: always\n---\n<!-- to-kiro:imported "));
        let fm = render(&rule(Inclusion::FileMatch(vec!["**/*.ts".into(), "a/{b,c}".into()])));
        assert!(fm.contains("fileMatchPattern: [\"**/*.ts\", \"a/{b,c}\"]\n"), "{fm}");
        let one = render(&rule(Inclusion::FileMatch(vec!["*.md".into()])));
        assert!(one.contains("fileMatchPattern: \"*.md\"\n"));
        let auto = render(&rule(Inclusion::Auto));
        assert!(auto.contains("name: \"r\"\n"));
        assert!(auto.contains("description: \"when editing \\\"x\\\" now\"\n"), "{auto}");
        assert!(auto.ends_with("\n\nbody\n"));
    }

    #[test]
    fn marker_roundtrip_detects_edits() {
        let c = render(&rule(Inclusion::Manual));
        assert_eq!(is_untouched(&c), Some(true));
        assert_eq!(is_untouched(&c.replace("body", "body edited")), Some(false));
        assert_eq!(is_untouched("hand written\n"), None);
    }
}
