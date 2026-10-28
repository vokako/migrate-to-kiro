//! Import the rules, MCP servers and skills of other AI coding assistants into Kiro.
//!
//! Flow: [`scan`] reads the other tools' files into an [`model::Inventory`],
//! [`build`] turns it into a [`plan::Plan`] that says what would change, and
//! [`apply`] writes it. Only `apply` touches the disk.

pub mod mcp;
pub mod model;
pub mod plan;
pub mod rules;
pub mod skills;
pub mod text;

pub use model::{Ctx, Kind, Level, Severity, Tool};
pub use plan::{apply, build, scan, Action, ApplyReport, Plan};
