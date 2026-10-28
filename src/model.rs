//! Types shared by every kind of resource.

use crate::mcp::McpServer;
use crate::rules::Rule;
use crate::skills::Skill;
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tool {
    Cursor,
    Copilot,
    Qoder,
    CodeBuddy,
    Claude,
}

impl Tool {
    pub const ALL: [Tool; 5] = [Tool::Cursor, Tool::Copilot, Tool::Qoder, Tool::CodeBuddy, Tool::Claude];

    pub fn slug(self) -> &'static str {
        match self {
            Tool::Cursor => "cursor",
            Tool::Copilot => "copilot",
            Tool::Qoder => "qoder",
            Tool::CodeBuddy => "codebuddy",
            Tool::Claude => "claude",
        }
    }

    pub fn parse(s: &str) -> Option<Tool> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cursor" => Some(Tool::Cursor),
            "copilot" | "github-copilot" => Some(Tool::Copilot),
            "qoder" => Some(Tool::Qoder),
            "codebuddy" | "code-buddy" => Some(Tool::CodeBuddy),
            "claude" | "claude-code" | "cc" => Some(Tool::Claude),
            _ => None,
        }
    }

    /// Comma separated list; `all` expands to every tool.
    pub fn parse_list(s: &str) -> Result<Vec<Tool>, String> {
        let mut out = Vec::new();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part.eq_ignore_ascii_case("all") {
                return Ok(Tool::ALL.to_vec());
            }
            match Tool::parse(part) {
                Some(t) if !out.contains(&t) => out.push(t),
                Some(_) => {}
                None => {
                    return Err(format!("unknown tool `{part}` (expected cursor, copilot, qoder, codebuddy, claude, all)"))
                }
            }
        }
        if out.is_empty() {
            return Err("no tool given".into());
        }
        Ok(out)
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// What kind of resource a change carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Steering,
    Mcp,
    Skill,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Steering, Kind::Mcp, Kind::Skill];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Steering => "steering",
            Kind::Mcp => "mcp",
            Kind::Skill => "skill",
        }
    }

    pub fn parse_list(s: &str) -> Result<Vec<Kind>, String> {
        let mut out = Vec::new();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let k = match part.to_ascii_lowercase().as_str() {
                "all" => return Ok(Kind::ALL.to_vec()),
                "steering" | "rules" | "rule" | "memory" => Kind::Steering,
                "mcp" => Kind::Mcp,
                "skills" | "skill" => Kind::Skill,
                other => return Err(format!("unknown resource kind `{other}` (expected steering, mcp, skills, all)")),
            };
            if !out.contains(&k) {
                out.push(k);
            }
        }
        if out.is_empty() {
            return Err("no resource kind given".into());
        }
        Ok(out)
    }
}

/// Where a resource applies in the source tool, which decides where it lands in Kiro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Personal, all projects → `~/.kiro/`.
    User,
    /// Shared with the team → `<project>/.kiro/`.
    Project,
    /// Personal and project specific (`*.local.md`, auto memory, Claude local MCP scope).
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Nothing to do, FYI.
    Info,
    /// Imported, but something was approximated or dropped.
    Lossy,
    /// Not imported.
    Skipped,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub tool: Tool,
    pub kind: Option<Kind>,
    pub origin: Option<String>,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Inventory {
    pub rules: Vec<Rule>,
    pub mcp: Vec<McpServer>,
    pub skills: Vec<Skill>,
    pub findings: Vec<Finding>,
    /// Shared files and folders already read (`.mcp.json`, `.agents/skills`, …),
    /// so a file used by several tools is imported once.
    pub seen: HashSet<PathBuf>,
}

impl Inventory {
    pub fn note(&mut self, severity: Severity, tool: Tool, kind: Kind, origin: Option<String>, message: impl Into<String>) {
        self.findings.push(Finding { severity, tool, kind: Some(kind), origin, message: message.into() });
    }

    /// A note about a tool as a whole, not tied to one resource kind.
    pub fn note_tool(&mut self, severity: Severity, tool: Tool, message: impl Into<String>) {
        self.findings.push(Finding { severity, tool, kind: None, origin: None, message: message.into() });
    }
}

#[derive(Debug, Clone)]
pub struct Ctx {
    pub home: PathBuf,
    /// Project directory to read project-level files from and write `.kiro/` into.
    /// `None` limits the run to user-level files.
    pub project: Option<PathBuf>,
    /// Read user-level files (`~/.cursor`, `~/.claude`, …) and write to `~/.kiro`.
    pub include_user: bool,
}

impl Ctx {
    pub fn new(home: PathBuf, project: Option<PathBuf>, include_user: bool) -> Self {
        let project = project.map(|p| p.canonicalize().unwrap_or(p));
        Ctx { home, project, include_user }
    }

    /// `~/.kiro` for user-level resources, `<project>/.kiro` otherwise.
    pub fn kiro_dir(&self, level: Level) -> Option<PathBuf> {
        match level {
            Level::User => Some(self.home.join(".kiro")),
            Level::Project | Level::Local => self.project.as_ref().map(|p| p.join(".kiro")),
        }
    }

    pub fn display(&self, path: &Path) -> String {
        if let Some(p) = &self.project {
            if let Ok(rel) = path.strip_prefix(p) {
                return rel.to_string_lossy().replace('\\', "/");
            }
        }
        if let Ok(rel) = path.strip_prefix(&self.home) {
            return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
        }
        path.to_string_lossy().into_owned()
    }
}

/// Lowercase ASCII slug: runs of non-alphanumerics collapse to a single `-`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "item".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        assert_eq!(slug("frontend/Components.mdc"), "frontend-components-mdc");
        assert_eq!(slug("  --  "), "item");
        assert_eq!(slug("React_Patterns"), "react-patterns");
    }

    #[test]
    fn tool_and_kind_lists() {
        assert_eq!(Tool::parse_list("cursor,qoder").unwrap(), vec![Tool::Cursor, Tool::Qoder]);
        assert_eq!(Tool::parse_list("all").unwrap().len(), 5);
        assert!(Tool::parse_list("windsurf").is_err());
        assert_eq!(Kind::parse_list("rules,mcp").unwrap(), vec![Kind::Steering, Kind::Mcp]);
        assert!(Kind::parse_list("hooks").is_err());
    }
}
