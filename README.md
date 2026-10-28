# to-kiro

Import what other AI coding assistants left on your machine into [Kiro](https://kiro.dev): rules and memory, MCP servers, and skills.

```
cd your-project
to-kiro            # scan, show the plan, ask once
to-kiro --yes      # no question
to-kiro -n         # plan only
```

It reads Cursor, GitHub Copilot, Qoder, CodeBuddy and Claude Code. Nothing is written until you confirm. Outside a terminal it only prints the plan unless you pass `--yes`.

## What goes where

| Resource | Kiro destination |
|---|---|
| Rules, instructions, memory | `.kiro/steering/imported-<tool>-<name>.md`, with the matching inclusion mode (`always`, `fileMatch`, `auto`, `manual`) |
| MCP servers | `.kiro/settings/mcp.json` (project) or `~/.kiro/settings/mcp.json` (user), merged into what is already there |
| Skills | `.kiro/skills/<name>/` or `~/.kiro/skills/<name>/`, copied whole |

Project-level files go to the project's `.kiro/`, user-level files (`~/.cursor`, `~/.claude`, ...) go to `~/.kiro/`.

## Where each tool is read from

| Tool | Rules and memory | MCP | Skills |
|---|---|---|---|
| Cursor | `.cursor/rules/*.mdc`, `.cursorrules` | `.cursor/mcp.json`, `~/.cursor/mcp.json` | `.cursor/skills`, `.agents/skills`, `~/.cursor/skills`, `~/.agents/skills` |
| Copilot | `.github/copilot-instructions.md`, `.github/instructions/*.instructions.md`, `.github/prompts/*.prompt.md` | `.vscode/mcp.json`, `~/.copilot/mcp-config.json` | `.github/skills`, `.claude/skills`, `.agents/skills`, `~/.copilot/skills`, `~/.agents/skills` |
| Qoder | `.qoder/rules`, `AGENTS.local.md`, `~/.qoder/rules`, `~/.qoder/AGENTS.md` | `.qoder/settings.json`, `.qoder/settings.local.json`, `.mcp.json`, `~/.qoder/settings.json` | `.qoder/skills`, `~/.qoder/skills` |
| CodeBuddy | `CODEBUDDY.md`, `.codebuddy/rules`, `CODEBUDDY.local.md`, `~/.codebuddy/CODEBUDDY.md`, `~/.codebuddy/rules` | `.mcp.json`, `~/.codebuddy/mcp.json` | `.codebuddy/skills`, `~/.codebuddy/skills` |
| Claude Code | `CLAUDE.md`, `CLAUDE.local.md`, `.claude/rules`, `~/.claude/CLAUDE.md`, `~/.claude/rules`, auto memory in `~/.claude/projects/<project>/memory` | `.mcp.json`, `~/.claude.json` | `.claude/skills`, `~/.claude/skills` |

`AGENTS.md` is read by Kiro itself and is left alone.

## What it cannot read

Cursor User Rules and Memories live in Cursor's settings, and Copilot personal and organization instructions and Copilot Memory live on github.com. Neither is a file on disk, so the tool says so and you copy them by hand.

## Safety

- Files written by `to-kiro` carry a content hash. They are updated only while unmodified; your edits are never overwritten.
- Anything else that already exists is a conflict and is left alone. `--force` overwrites it and keeps a backup (`*.md.bak`, `mcp.json.bak`, `~/.kiro/to-kiro-backup/`).
- Your existing `mcp.json` entries and unrelated keys are preserved. An `mcp.json` that is not valid JSON is never touched.
- A rule or skill that looks like it contains a credential is not imported; the report shows the file and line, never the value.
- Personal notes (`*.local.md`, Claude auto memory) are added to `.git/info/exclude` so they are not committed.
- A Claude Code project-private MCP server goes to your user-level `mcp.json`, because Kiro has no private project scope.

Entries that cannot be converted exactly (tool allow-lists, `${input:...}` prompts, WebSocket transports, tool-specific front matter) are reported with the reason instead of being guessed.

## Options

```
to-kiro [TOOLS] [--only steering,mcp,skills] [--project DIR] [--no-project] [--no-user]
        [-y|--yes] [-n|--dry-run] [--force] [-v|--verbose]
```

`TOOLS` is a comma separated list of `cursor`, `copilot`, `qoder`, `codebuddy`, `claude`, or `all` (default).

## Try it without touching your setup

```
python3 tests/mkfixture.py /tmp/sandbox
cd /tmp/sandbox/proj && HOME=/tmp/sandbox/home to-kiro
```

## Build

```
cargo build --release     # target/release/to-kiro
cargo test
```
