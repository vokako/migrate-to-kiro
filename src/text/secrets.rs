//! Heuristic credential scan. Rule files occasionally contain pasted tokens; a
//! rule with a hit is never copied into Kiro steering.

/// Returns `(line number, label)` for each suspicious line. Matched values are
/// never returned.
pub fn scan(text: &str) -> Vec<(usize, &'static str)> {
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if let Some(label) = scan_line(line) {
            out.push((idx + 1, label));
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// `prefix` followed by at least `min_tail` chars satisfying `pred`, and not
/// preceded by a word character (so `task-...` does not match `sk-`).
fn has_prefixed(line: &str, prefix: &str, min_tail: usize, pred: impl Fn(char) -> bool) -> bool {
    let mut from = 0;
    while let Some(pos) = line[from..].find(prefix) {
        let at = from + pos;
        let boundary_ok = line[..at].chars().next_back().map_or(true, |c| !is_word(c));
        if boundary_ok {
            let tail = &line[at + prefix.len()..];
            if tail.chars().take_while(|&c| pred(c)).count() >= min_tail {
                return true;
            }
        }
        from = at + prefix.len();
    }
    false
}

fn scan_line(line: &str) -> Option<&'static str> {
    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
        return Some("private key");
    }
    let upper_alnum = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit();
    if has_prefixed(line, "AKIA", 16, upper_alnum) || has_prefixed(line, "ASIA", 16, upper_alnum) {
        return Some("AWS access key id");
    }
    let alnum = |c: char| c.is_ascii_alphanumeric();
    for p in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if has_prefixed(line, p, 30, alnum) {
            return Some("GitHub token");
        }
    }
    if has_prefixed(line, "github_pat_", 30, |c| c.is_ascii_alphanumeric() || c == '_') {
        return Some("GitHub token");
    }
    if has_prefixed(line, "sk-", 20, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Some("API key (sk-)");
    }
    for p in ["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"] {
        if has_prefixed(line, p, 10, |c| c.is_ascii_alphanumeric() || c == '-') {
            return Some("Slack token");
        }
    }
    if has_prefixed(line, "AIza", 35, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Some("Google API key");
    }
    generic_assignment(line)
}

const KEYWORDS: [&str; 7] = ["password", "passwd", "secret", "token", "api_key", "apikey", "api-key"];
const PLACEHOLDERS: [&str; 8] = ["xxx", "your", "example", "placeholder", "changeme", "<", "${", "$("];

fn generic_assignment(line: &str) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    for kw in KEYWORDS {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(kw) {
            let end = from + pos + kw.len();
            from = end;
            let rest = line[end..].trim_start_matches(['"', '\'', '`', ' ', '\t']);
            let Some(rest) = rest.strip_prefix([':', '=']) else { continue };
            let rest = rest.trim_start_matches(['"', '\'', '`', ' ', '\t']);
            let value: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || "/+=_.-".contains(*c))
                .collect();
            if value.len() < 16 {
                continue;
            }
            let v = value.to_ascii_lowercase();
            if PLACEHOLDERS.iter().any(|p| v.contains(p)) || rest.starts_with(PLACEHOLDERS[5]) || rest.starts_with("${") {
                continue;
            }
            return Some("credential assignment");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_common_tokens() {
        assert!(!scan("key AKIAABCDEFGHIJKLMNOP here").is_empty());
        assert!(!scan("token ghp_abcdefghijklmnopqrstuvwxyz0123456789").is_empty());
        assert!(!scan("OPENAI sk-abcdefghijklmnopqrstuvwx").is_empty());
        assert!(!scan("-----BEGIN RSA PRIVATE KEY-----").is_empty());
        assert!(!scan("password = \"Zk39dh2jf83hd92jdk\"").is_empty());
    }

    #[test]
    fn ignores_prose_and_placeholders() {
        assert!(scan("Use the task-management-system-for-everything").is_empty());
        assert!(scan("Set API_KEY: <your key here>").is_empty());
        assert!(scan("token: ${MY_TOKEN_VARIABLE_NAME}").is_empty());
        assert!(scan("Never commit a password or secret to git.").is_empty());
        assert!(scan("api_key: your-api-key-goes-here-xxxxxxxx").is_empty());
    }

    #[test]
    fn reports_line_numbers_not_values() {
        let hits = scan("ok\nghp_abcdefghijklmnopqrstuvwxyz0123456789\n");
        assert_eq!(hits, vec![(2, "GitHub token")]);
    }
}
