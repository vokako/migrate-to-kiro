//! Where each tool keeps its MCP servers, and how an entry maps to Kiro's shape.

use super::{jsonc, McpServer};
use crate::model::{Ctx, Inventory, Kind, Level, Severity, Tool};
use crate::text::secrets;
use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy)]
enum Layout {
    /// `{ "mcpServers": { … } }`
    McpServers,
    /// VS Code: `{ "servers": { … }, "inputs": [ … ] }`
    VsCode,
    /// Copilot CLI: `mcpServers`, or the server names as top-level keys.
    CopilotCli,
    /// `~/.claude.json`: user scope at the top level, local scope under `projects[<path>]`.
    ClaudeJson,
}

struct Src {
    level: Level,
    path: PathBuf,
    layout: Layout,
}

fn sources(tool: Tool, ctx: &Ctx) -> Vec<Src> {
    let mut v = Vec::new();
    let home = &ctx.home;
    let proj = ctx.project.as_ref();
    let mut user = |rel: &str, layout| {
        if ctx.include_user {
            v.push(Src { level: Level::User, path: home.join(rel), layout });
        }
    };
    match tool {
        Tool::Cursor => user(".cursor/mcp.json", Layout::McpServers),
        Tool::Copilot => user(".copilot/mcp-config.json", Layout::CopilotCli),
        Tool::Qoder => user(".qoder/settings.json", Layout::McpServers),
        Tool::CodeBuddy => user(".codebuddy/mcp.json", Layout::McpServers),
        Tool::Claude => user(".claude.json", Layout::ClaudeJson),
    }
    if let Some(p) = proj {
        let mut project = |rel: &str, level, layout| v.push(Src { level, path: p.join(rel), layout });
        match tool {
            Tool::Cursor => project(".cursor/mcp.json", Level::Project, Layout::McpServers),
            Tool::Copilot => project(".vscode/mcp.json", Level::Project, Layout::VsCode),
            Tool::Qoder => {
                project(".qoder/settings.json", Level::Project, Layout::McpServers);
                project(".mcp.json", Level::Project, Layout::McpServers);
                project(".qoder/settings.local.json", Level::Local, Layout::McpServers);
            }
            Tool::CodeBuddy | Tool::Claude => project(".mcp.json", Level::Project, Layout::McpServers),
        }
    }
    v
}

fn as_servers(v: Option<&Value>) -> Vec<(String, Value)> {
    v.and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn extract(layout: Layout, root: &Value, ctx: &Ctx) -> Vec<(Level, String, Value)> {
    let tag = |lvl: Level, items: Vec<(String, Value)>| items.into_iter().map(move |(n, v)| (lvl, n, v)).collect::<Vec<_>>();
    match layout {
        Layout::McpServers => tag(Level::Project, as_servers(root.get("mcpServers"))),
        Layout::VsCode => {
            let key = if root.get("servers").is_some() { "servers" } else { "mcpServers" };
            tag(Level::Project, as_servers(root.get(key)))
        }
        Layout::CopilotCli => {
            if root.get("mcpServers").is_some() {
                tag(Level::Project, as_servers(root.get("mcpServers")))
            } else {
                // Bare format: every top-level object that looks like a server.
                let items = root
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .filter(|(_, v)| v.get("command").is_some() || v.get("url").is_some())
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                tag(Level::Project, items)
            }
        }
        Layout::ClaudeJson => {
            let mut out = tag(Level::User, as_servers(root.get("mcpServers")));
            if let Some(p) = &ctx.project {
                let local = root.get("projects").and_then(|m| m.get(p.to_string_lossy().as_ref()));
                out.extend(tag(Level::Local, as_servers(local.and_then(|l| l.get("mcpServers")))));
            }
            out
        }
    }
}

pub fn discover(tool: Tool, ctx: &Ctx, inv: &mut Inventory) {
    for src in sources(tool, ctx) {
        if !src.path.is_file() {
            continue;
        }
        let canon = src.path.canonicalize().unwrap_or_else(|_| src.path.clone());
        if !inv.seen.insert(canon) {
            continue; // `.mcp.json` is shared by several tools
        }
        let shown = ctx.display(&src.path);
        let text = match fs::read_to_string(&src.path) {
            Ok(t) => t,
            Err(e) => {
                inv.note(Severity::Skipped, tool, Kind::Mcp, Some(shown), format!("cannot read: {e}"));
                continue;
            }
        };
        let root = match jsonc::parse(&text) {
            Ok(v) => v,
            Err(e) => {
                inv.note(Severity::Skipped, tool, Kind::Mcp, Some(shown), format!("not valid JSON: {e}"));
                continue;
            }
        };

        for (level, name, raw) in extract(src.layout, &root, ctx) {
            // The layout reports Project for plain files; the source decides the real level.
            let level = match (src.level, level) {
                (Level::User, Level::Project) => Level::User,
                (Level::Local, _) => Level::Local,
                (_, l) => l,
            };
            let origin = format!("{shown} → {name}");
            match normalize(&raw) {
                Err(why) => inv.note(Severity::Skipped, tool, Kind::Mcp, Some(origin), why),
                Ok((config, mut notes)) => {
                    // Kiro has no private per-project scope; keep it out of the shared project file.
                    let level = if level == Level::Local {
                        notes.push(
                            "was private to this project in the source tool; Kiro has no private project scope, so it goes to your user-level mcp.json and is available in every workspace"
                                .into(),
                        );
                        Level::User
                    } else {
                        level
                    };
                    for n in notes {
                        inv.note(Severity::Lossy, tool, Kind::Mcp, Some(origin.clone()), n);
                    }
                    inv.mcp.push(McpServer { tool, level, name, config: Value::Object(config), origin });
                }
            }
        }
    }
}

// ── normalization ────────────────────────────────────────────────────────

const HANDLED: [&str; 14] = [
    "type", "command", "args", "env", "url", "headers", "disabled", "autoApprove", "alwaysAllow", "disabledTools",
    "excludeTools", "trust", "description", "name",
];
const SILENT: [&str; 1] = ["timeout"];
const CONTEXT_VARS: [&str; 6] = [
    "${workspaceFolder}", "${workspaceRoot}", "${userHome}", "${cwd}", "${CLAUDE_PROJECT_DIR", "${CODEBUDDY_PROJECT_DIR",
];

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| match x {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_map(v: Option<&Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(m) = v.and_then(Value::as_object) {
        for (k, val) in m {
            let s = match val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            out.insert(k.clone(), Value::String(s));
        }
    }
    out
}

fn visit_strings(v: &mut Value, f: &mut dyn FnMut(&mut String)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter_mut().for_each(|x| visit_strings(x, f)),
        Value::Object(m) => m.values_mut().for_each(|x| visit_strings(x, f)),
        _ => {}
    }
}

fn looks_secret(key: &str, value: &str) -> bool {
    if value.contains("${") || value.starts_with('$') {
        return false;
    }
    if !secrets::scan(&format!("{key}: {value}")).is_empty() {
        return true;
    }
    let k = key.to_ascii_lowercase();
    let bare = value.trim().trim_start_matches("Bearer ").trim_start_matches("bearer ");
    matches!(k.as_str(), "authorization" | "x-api-key" | "api-key" | "apikey" | "x-auth-token") && bare.len() >= 16
}

/// `Err` carries the reason the entry cannot be imported.
pub fn normalize(raw: &Value) -> Result<(Map<String, Value>, Vec<String>), String> {
    let obj = raw.as_object().ok_or("entry is not an object")?;
    let ty = obj.get("type").and_then(Value::as_str).map(str::to_ascii_lowercase);
    if let Some(t) = ty.as_deref() {
        if matches!(t, "ws" | "websocket" | "sdk") {
            return Err(format!("transport `{t}` is not supported by Kiro"));
        }
    }
    let mut notes: Vec<String> = Vec::new();
    let mut out = Map::new();

    if let Some(cmd) = obj.get("command").and_then(Value::as_str) {
        out.insert("command".into(), cmd.into());
        out.insert("args".into(), Value::Array(strings(obj.get("args")).into_iter().map(Value::String).collect()));
        let env = string_map(obj.get("env"));
        for (k, v) in &env {
            if v.as_str().map_or(false, |s| looks_secret(k, s)) {
                notes.push(format!("env `{k}` holds a hard-coded credential and was copied as-is; replace it with ${{{k}}} before sharing the file"));
            }
        }
        if !env.is_empty() {
            out.insert("env".into(), Value::Object(env));
        }
        if obj.contains_key("cwd") {
            notes.push("`cwd` has no documented Kiro equivalent and was dropped".into());
        }
    } else if let Some(url) = obj.get("url").and_then(Value::as_str) {
        out.insert("url".into(), url.into());
        let headers = string_map(obj.get("headers"));
        for (k, v) in &headers {
            if v.as_str().map_or(false, |s| looks_secret(k, s)) {
                notes.push(format!("header `{k}` holds a hard-coded credential and was copied as-is; replace it with a ${{VAR}} reference before sharing the file"));
            }
        }
        if !headers.is_empty() {
            out.insert("headers".into(), Value::Object(headers));
        }
        if ty.as_deref() == Some("sse") {
            notes.push("SSE transport; Kiro documents HTTP endpoints, so the connection may fail".into());
        }
    } else {
        return Err("has neither `command` nor `url`".into());
    }

    if obj.get("disabled").and_then(Value::as_bool) == Some(true) {
        out.insert("disabled".into(), Value::Bool(true));
    }
    let mut approve = strings(obj.get("autoApprove"));
    if approve.is_empty() {
        approve = strings(obj.get("alwaysAllow"));
    }
    if obj.get("trust").and_then(Value::as_bool) == Some(true) {
        approve = vec!["*".into()];
        notes.push("`trust: true` became autoApprove [\"*\"]: every tool of this server runs without asking".into());
    }
    if !approve.is_empty() {
        out.insert("autoApprove".into(), Value::Array(approve.into_iter().map(Value::String).collect()));
    }
    let mut denied = strings(obj.get("disabledTools"));
    if denied.is_empty() {
        denied = strings(obj.get("excludeTools"));
    }
    if !denied.is_empty() {
        out.insert("disabledTools".into(), Value::Array(denied.into_iter().map(Value::String).collect()));
    }

    // Allow-lists restrict which tools exist; Kiro's equivalent would have to list every other tool.
    for key in ["includeTools", "tools"] {
        let list = strings(obj.get(key));
        if !list.is_empty() && list != ["*"] {
            notes.push(format!("`{key}` (tool allow-list) has no Kiro equivalent and was dropped; all tools of the server are available"));
        }
    }

    let dropped: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !HANDLED.contains(k) && !SILENT.contains(k) && !matches!(*k, "includeTools" | "tools" | "cwd"))
        .collect();
    if !dropped.is_empty() {
        notes.push(format!("dropped fields with no Kiro equivalent: {}", dropped.join(", ")));
    }

    // Variable syntax.
    let mut needs_input = false;
    let mut context_var: Option<String> = None;
    let mut has_default = false;
    let mut value = Value::Object(out);
    visit_strings(&mut value, &mut |s| {
        if s.contains("${input:") {
            needs_input = true;
        }
        if s.contains("${env:") {
            *s = s.replace("${env:", "${");
        }
        for var in CONTEXT_VARS {
            if s.contains(var) {
                context_var = Some(var.trim_end_matches('}').to_string() + "}");
            }
        }
        if s.contains(":-") && s.contains("${") {
            has_default = true;
        }
    });
    if needs_input {
        return Err("asks for a value with `${input:…}` at startup; Kiro cannot prompt for it. Put the value in an environment variable and reference it as ${VAR}".into());
    }
    if let Some(v) = context_var {
        notes.push(format!("uses {v}, which Kiro does not document; check the path"));
    }
    if has_default {
        notes.push("uses `${VAR:-default}`; Kiro documents only ${VAR}, so make sure the variable is set".into());
    }

    let Value::Object(out) = value else { unreachable!() };
    Ok((out, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(v: Value) -> Result<(Map<String, Value>, Vec<String>), String> {
        normalize(&v)
    }

    #[test]
    fn local_server_gets_args_and_env() {
        let (c, notes) = n(json!({"command": "npx", "env": {"DEBUG": 1, "HOME_URL": "${env:HOME}/x"}})).unwrap();
        assert_eq!(c["command"], "npx");
        assert_eq!(c["args"], json!([]));
        assert_eq!(c["env"]["DEBUG"], "1");
        assert_eq!(c["env"]["HOME_URL"], "${HOME}/x");
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn remote_server_and_sse_note() {
        let (c, notes) = n(json!({"type": "sse", "url": "https://x/mcp", "headers": {"X": "y"}})).unwrap();
        assert_eq!(c["url"], "https://x/mcp");
        assert!(c.get("command").is_none());
        assert!(notes.iter().any(|x| x.contains("SSE")));
    }

    #[test]
    fn unsupported_and_incomplete_entries_are_rejected() {
        assert!(n(json!({"type": "ws", "url": "wss://x"})).unwrap_err().contains("not supported"));
        assert!(n(json!({"args": ["a"]})).unwrap_err().contains("neither"));
        assert!(n(json!({"command": "x", "args": ["${input:token}"]})).unwrap_err().contains("input"));
        assert!(n(json!("str")).is_err());
    }

    #[test]
    fn approvals_and_disabled_tools_map_over() {
        let (c, notes) = n(json!({
            "command": "x", "alwaysAllow": ["a"], "excludeTools": ["rm"], "disabled": true,
            "includeTools": ["a", "b"], "timeout": 5000, "oauth": {"enabled": true}
        }))
        .unwrap();
        assert_eq!(c["autoApprove"], json!(["a"]));
        assert_eq!(c["disabledTools"], json!(["rm"]));
        assert_eq!(c["disabled"], true);
        assert!(notes.iter().any(|x| x.contains("includeTools")));
        assert!(notes.iter().any(|x| x.contains("oauth")));
        assert!(!notes.iter().any(|x| x.contains("timeout")));

        let (c, notes) = n(json!({"command": "x", "trust": true})).unwrap();
        assert_eq!(c["autoApprove"], json!(["*"]));
        assert!(notes.iter().any(|x| x.contains("trust")));
    }

    #[test]
    fn copilot_wildcard_tools_are_not_noise() {
        let (_, notes) = n(json!({"type": "local", "command": "x", "args": [], "tools": ["*"]})).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn hard_coded_credentials_are_flagged_references_are_not() {
        let (_, notes) = n(json!({"command": "x", "env": {
            "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "OK": "${TOKEN}", "MODE": "prod"}}))
        .unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("GITHUB_PERSONAL_ACCESS_TOKEN"));
        assert!(!notes[0].contains("ghp_"), "the value must never be echoed");

        let (_, notes) = n(json!({"url": "https://x", "headers": {"Authorization": "Bearer abcdefghijklmnopqrstuvwxyz"}})).unwrap();
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn context_variables_and_defaults_are_noted() {
        let (_, notes) = n(json!({"command": "x", "args": ["${workspaceFolder}/a", "${PORT:-80}"]})).unwrap();
        assert_eq!(notes.len(), 2);
    }
}
