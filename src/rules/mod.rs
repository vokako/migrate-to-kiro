//! Rules and memory → Kiro steering files.
//!
//! One adapter per source tool reads only the locations the tool's own
//! documentation names, and maps the tool's activation mode to the closest Kiro
//! steering inclusion mode.

pub mod adapters;
pub mod render;

use crate::model::{Ctx, Inventory, Level, Tool};

/// Kiro steering inclusion mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inclusion {
    Always,
    FileMatch(Vec<String>),
    Manual,
    Auto,
}

impl Inclusion {
    pub fn label(&self) -> String {
        match self {
            Inclusion::Always => "always".into(),
            Inclusion::FileMatch(g) => format!("fileMatch {}", g.join(", ")),
            Inclusion::Manual => "manual".into(),
            Inclusion::Auto => "auto".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub tool: Tool,
    pub level: Level,
    /// File name stem, already slugged and unique within (tool, level).
    pub name: String,
    pub description: Option<String>,
    pub inclusion: Inclusion,
    pub body: String,
    /// Source path as shown to the user and written into the marker
    /// (`~/…` for user files, project-relative otherwise).
    pub origin: String,
}

pub fn discover(tool: Tool, ctx: &Ctx, inv: &mut Inventory) {
    match tool {
        Tool::Cursor => adapters::cursor(ctx, inv),
        Tool::Copilot => adapters::copilot(ctx, inv),
        Tool::Qoder => adapters::qoder(ctx, inv),
        Tool::CodeBuddy => adapters::codebuddy(ctx, inv),
        Tool::Claude => adapters::claude(ctx, inv),
    }
}
