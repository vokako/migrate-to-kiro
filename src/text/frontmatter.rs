//! Tolerant front matter parser.
//!
//! Cursor writes `globs: **/*.ts`, which is not valid YAML (`*` starts an alias),
//! so `serde_yaml` cannot be used for rule files. This parser reads the small
//! subset the assistants actually emit: scalars, `- item` lists, `[a, b]` lists
//! and `|` / `>` block scalars.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Scalar(String),
    List(Vec<String>),
}

#[derive(Debug, Default)]
pub struct Front {
    map: BTreeMap<String, Val>,
}

impl Front {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Non-empty trimmed scalar.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.map.get(key)? {
            Val::Scalar(s) if !s.trim().is_empty() => Some(s.trim()),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.map.get(key)? {
            Val::Scalar(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" => Some(true),
                "false" | "no" => Some(false),
                _ => None,
            },
            Val::List(_) => None,
        }
    }

    /// List items. A scalar is split on commas that are outside `{}` groups.
    pub fn items(&self, key: &str) -> Vec<String> {
        match self.map.get(key) {
            Some(Val::List(v)) => v.iter().filter(|s| !s.is_empty()).cloned().collect(),
            Some(Val::Scalar(s)) => split_commas(s),
            None => vec![],
        }
    }
}

/// Split on commas outside `{...}` (glob alternatives such as `*.{ts,tsx}`).
pub fn split_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth <= 0 => {
                push_item(&mut out, &cur);
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    push_item(&mut out, &cur);
    out
}

fn push_item(out: &mut Vec<String>, raw: &str) {
    let t = unquote(raw.trim());
    if !t.is_empty() {
        out.push(t);
    }
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn strip_inline_comment(v: &str) -> &str {
    if v.starts_with('"') || v.starts_with('\'') {
        return v;
    }
    match v.find(" #") {
        Some(i) => v[..i].trim_end(),
        None => v,
    }
}

/// Split `text` into front matter and body. Without a terminated `---` block the
/// whole text is the body.
pub fn parse(text: &str) -> (Front, &str) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = t.split_inclusive('\n');
    match lines.next() {
        Some(first) if first.trim_end() == "---" => {}
        _ => return (Front::default(), t),
    }
    let mut offset = t.split_inclusive('\n').next().map(str::len).unwrap_or(0);
    let mut fm_lines: Vec<&str> = Vec::new();
    let mut closed = false;
    for line in lines {
        offset += line.len();
        let tr = line.trim_end();
        if tr == "---" || tr == "..." {
            closed = true;
            break;
        }
        fm_lines.push(line.trim_end_matches(['\r', '\n']));
    }
    if !closed {
        return (Front::default(), t);
    }
    let body = t[offset..].trim_start_matches(['\r', '\n']);
    (parse_lines(&fm_lines), body)
}

fn parse_lines(lines: &[&str]) -> Front {
    let mut map = BTreeMap::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((k, v)) = trimmed.split_once(':') else { continue };
        let key = k.trim().to_string();
        let v = v.trim();

        if v.is_empty() {
            let mut items = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if let Some(rest) = l.strip_prefix('-') {
                    items.push(unquote(strip_inline_comment(rest.trim())));
                    i += 1;
                } else if l.is_empty() {
                    i += 1;
                } else {
                    break;
                }
            }
            let val = if items.is_empty() { Val::Scalar(String::new()) } else { Val::List(items) };
            map.insert(key, val);
        } else if v.starts_with('|') || v.starts_with('>') {
            let mut parts = Vec::new();
            while i < lines.len() && (lines[i].starts_with(char::is_whitespace) || lines[i].trim().is_empty()) {
                let l = lines[i].trim();
                if !l.is_empty() {
                    parts.push(l.to_string());
                }
                i += 1;
            }
            map.insert(key, Val::Scalar(parts.join(" ")));
        } else if v.starts_with('[') && v.ends_with(']') {
            map.insert(key, Val::List(split_commas(&v[1..v.len() - 1])));
        } else {
            map.insert(key, Val::Scalar(unquote(strip_inline_comment(v))));
        }
    }
    Front { map }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquoted_glob_with_stars_is_accepted() {
        let (fm, body) = parse("---\nglobs: **/*.ts, src/**/*.tsx\nalwaysApply: false\n---\nhello\n");
        assert_eq!(fm.items("globs"), vec!["**/*.ts", "src/**/*.tsx"]);
        assert_eq!(fm.bool("alwaysApply"), Some(false));
        assert_eq!(body, "hello\n");
    }

    #[test]
    fn braces_are_not_split() {
        assert_eq!(split_commas("*.{ts,tsx}, docs/**"), vec!["*.{ts,tsx}", "docs/**"]);
    }

    #[test]
    fn dash_lists_and_inline_lists() {
        let (fm, _) = parse("---\nglob:\n  - src/api/**\n  - \"**/*.test.ts\"\npaths: [a/**, 'b/**']\n---\nx");
        assert_eq!(fm.items("glob"), vec!["src/api/**", "**/*.test.ts"]);
        assert_eq!(fm.items("paths"), vec!["a/**", "b/**"]);
    }

    #[test]
    fn no_front_matter_or_unterminated() {
        let (fm, body) = parse("# Title\nbody");
        assert!(fm.is_empty());
        assert_eq!(body, "# Title\nbody");
        let (fm, body) = parse("---\nkey: v\nno end");
        assert!(fm.is_empty());
        assert!(body.starts_with("---"));
    }

    #[test]
    fn crlf_bom_and_comments() {
        let (fm, body) = parse("\u{feff}---\r\ndescription: \"Use when X\" \r\nalwaysApply: true # note\r\n---\r\nbody\r\n");
        assert_eq!(fm.str("description"), Some("Use when X"));
        assert_eq!(fm.bool("alwaysApply"), Some(true));
        assert_eq!(body, "body\r\n");
    }

    #[test]
    fn block_scalar() {
        let (fm, _) = parse("---\ndescription: >\n  first line\n  second line\n---\nx");
        assert_eq!(fm.str("description"), Some("first line second line"));
    }
}
