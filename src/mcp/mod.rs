//! MCP servers → Kiro `mcp.json`.
//!
//! Every tool's server entry is normalized to the shape Kiro documents:
//! `command`/`args`/`env` for local servers, `url`/`headers` for remote ones,
//! plus `disabled`, `autoApprove` and `disabledTools`.

mod jsonc;
mod sources;

use crate::model::{Ctx, Inventory, Level, Tool};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct McpServer {
    pub tool: Tool,
    pub level: Level,
    pub name: String,
    /// Already in Kiro's shape.
    pub config: Value,
    /// Source file as shown to the user.
    pub origin: String,
}

impl McpServer {
    pub fn transport(&self) -> &'static str {
        if self.config.get("url").is_some() {
            "remote"
        } else {
            "stdio"
        }
    }
}

/// Parse JSON that may contain comments and trailing commas (VS Code style).
pub fn parse_json(text: &str) -> Result<Value, String> {
    jsonc::parse(text)
}

pub fn discover(tool: Tool, ctx: &Ctx, inv: &mut Inventory) {
    sources::discover(tool, ctx, inv)
}

/// Canonical JSON text (keys sorted) for hashing and comparison.
pub fn canonical(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    out.insert(k.clone(), sort(&m[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort(v)).unwrap_or_default()
}
