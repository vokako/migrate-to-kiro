//! `migrate-to-kiro`: find what other AI coding assistants left on this machine and
//! import it into Kiro.

use anyhow::{anyhow, Result};
use clap::Parser;
use console::{style, Term};
use dialoguer::Confirm;
use std::path::PathBuf;
use migrate_to_kiro::model::Finding;
use migrate_to_kiro::{apply, build, scan, Action, Ctx, Kind, Severity, Tool};

/// Import Cursor, GitHub Copilot, Qoder, CodeBuddy and Claude Code rules, MCP
/// servers and skills into Kiro.
///
/// Run it with no arguments in a project folder: it looks for everything these
/// tools left behind, shows what would be imported, and asks once.
#[derive(Parser)]
#[command(name = "migrate-to-kiro", version)]
struct Cli {
    /// Tools to read: cursor, copilot, qoder, codebuddy, claude, or all
    #[arg(default_value = "all")]
    tools: String,

    /// What to import: steering (rules and memory), mcp, skills, or all
    #[arg(long, default_value = "all")]
    only: String,

    /// Project to read project-level files from and write .kiro/ into
    #[arg(long, default_value = ".")]
    project: PathBuf,

    /// Skip the project; import user-level files only
    #[arg(long)]
    no_project: bool,

    /// Skip user-level files (~/.cursor, ~/.claude, ...) and do not write to ~/.kiro
    #[arg(long)]
    no_user: bool,

    /// Import without asking
    #[arg(short, long)]
    yes: bool,

    /// Show the plan and stop
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Overwrite files that were edited since import or not created by migrate-to-kiro (a backup is kept)
    #[arg(long)]
    force: bool,

    /// Also list unchanged items and the Kiro file each item goes to
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}: {e:#}", style("error").red().bold());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let tools = Tool::parse_list(&cli.tools).map_err(|e| anyhow!(e))?;
    let kinds = Kind::parse_list(&cli.only).map_err(|e| anyhow!(e))?;
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine the home directory"))?;

    let mut project = if cli.no_project { None } else { Some(cli.project.clone()) };
    if let Some(p) = &project {
        if !p.is_dir() {
            return Err(anyhow!("project directory {} does not exist", p.display()));
        }
        // From $HOME, project-level and user-level would be the same folder.
        if p.canonicalize().ok() == home.canonicalize().ok() {
            eprintln!("{} the project is your home directory; reading user-level files only", style("note:").yellow());
            project = None;
        }
    }
    if project.is_none() && cli.no_user {
        return Err(anyhow!("nothing to read: --no-project and --no-user together exclude everything"));
    }

    let ctx = Ctx::new(home, project, !cli.no_user);
    let plan = build(scan(&tools, &kinds, &ctx), &ctx);

    let names: Vec<&str> = tools.iter().map(|t| t.slug()).collect();
    println!("{} {}", style("migrate-to-kiro").bold(), style(names.join(", ")).dim());
    match &ctx.project {
        Some(p) => println!("{}", style(format!("project  {}", p.display())).dim()),
        None => println!("{}", style("project  none").dim()),
    }
    println!("{}", style(format!("user     {}", if ctx.include_user { "included" } else { "skipped" })).dim());

    print_changes(&plan, &ctx, cli.verbose);
    print_findings(&plan.findings);

    let n_create = plan.count(|a| *a == Action::Create);
    let n_update = plan.count(|a| *a == Action::Update);
    let n_same = plan.count(|a| *a == Action::Unchanged);
    let n_conf = plan.count(|a| matches!(a, Action::Conflict(_)));
    let n_skip = plan.findings.iter().filter(|f| f.severity == Severity::Skipped).count();
    println!(
        "\n{} to import, {} to update, {} already imported, {} conflicts, {} not importable.",
        style(n_create).green().bold(),
        n_update,
        n_same,
        n_conf,
        n_skip
    );
    if n_conf > 0 && !cli.force {
        println!("{}", style("Conflicts are left alone. Add --force to overwrite them (a backup is kept).").yellow());
    }

    let pending = plan.pending(cli.force);
    if pending == 0 {
        return Ok(());
    }
    if cli.dry_run {
        println!("{}", style("Dry run, nothing written.").dim());
        return Ok(());
    }

    let interactive = Term::stdout().is_term();
    let go = if cli.yes {
        true
    } else if interactive {
        Confirm::new().with_prompt(format!("Import {pending} item(s) into Kiro?")).default(true).interact()?
    } else {
        println!("{}", style("Not a terminal and no --yes given, so nothing was written.").dim());
        false
    };
    if !go {
        return Ok(());
    }

    let report = apply(&plan, cli.force)?;
    for b in &report.backups {
        println!("{}", style(format!("backup   {}", ctx.display(b))).dim());
    }
    for f in &report.excludes_added {
        println!("{} added the personal-notes pattern to {}", style("✓").green(), f.display());
    }
    println!("{} {} item(s) written. Restart Kiro or open a new chat to pick them up.", style("✓").green(), report.written.len());
    Ok(())
}

fn print_changes(plan: &migrate_to_kiro::Plan, ctx: &Ctx, verbose: bool) {
    for kind in Kind::ALL {
        let items: Vec<_> = plan.changes.iter().filter(|c| c.kind == kind).collect();
        if items.is_empty() {
            continue;
        }
        let title = match kind {
            Kind::Steering => "Steering (rules and memory)",
            Kind::Mcp => "MCP servers",
            Kind::Skill => "Skills",
        };
        println!("\n{} {}", style(title).bold(), style(format!("({})", items.len())).dim());
        for c in items {
            if c.action == Action::Unchanged && !verbose {
                continue;
            }
            let sym = match &c.action {
                Action::Create => style("+").green(),
                Action::Update => style("~").cyan(),
                Action::Unchanged => style("=").dim(),
                Action::Conflict(_) => style("!").yellow(),
            };
            println!("  {sym} {:<9} {}  {}", c.tool.slug(), c.origin, style(format!("({})", c.label)).dim());
            if verbose {
                println!("      {} {}", style("→").dim(), c.target);
            }
            if let Action::Conflict(why) = &c.action {
                println!("      {}", style(format!("{why}; kept as is unless --force")).yellow());
            }
        }
    }
    let _ = ctx;
}

fn print_findings(findings: &[Finding]) {
    let mut shown = false;
    for sev in [Severity::Skipped, Severity::Lossy, Severity::Info] {
        for f in findings.iter().filter(|f| f.severity == sev) {
            if !shown {
                println!("\n{}", style("Notes").bold());
                shown = true;
            }
            let tag = match sev {
                Severity::Skipped => style("not imported").red().to_string(),
                Severity::Lossy => style("approximated").yellow().to_string(),
                Severity::Info => style("info").dim().to_string(),
            };
            let kind = f.kind.map(|k| k.label()).unwrap_or("-");
            let at = f.origin.as_deref().map(|o| format!(" {o}:")).unwrap_or_default();
            println!("  {tag} [{} {kind}]{at} {}", f.tool, f.message);
        }
    }
}
