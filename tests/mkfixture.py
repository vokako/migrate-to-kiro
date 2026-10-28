#!/usr/bin/env python3
"""Build a sandbox that looks like a machine used with every supported tool.

Usage: mkfixture.py <root>   (creates <root>/home and <root>/proj)
"""
import json
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
home, proj = root / "home", root / "proj"
proj_abs = str(proj.resolve())

files = {
    # Cursor
    proj / ".cursor/rules/react.mdc": "---\nglobs: **/*.tsx, src/ui/**\nalwaysApply: false\n---\nUse hooks. See @docs/arch.md\n",
    proj / ".cursor/rules/style.mdc": "---\nalwaysApply: true\n---\nUse tabs.\n",
    proj / ".cursor/rules/api.mdc": "---\ndescription: REST API design conventions\n---\nUse nouns for resources.\n",
    proj / ".cursorrules": "Legacy rule: keep functions short.\n",
    proj / ".cursor/mcp.json": json.dumps({"mcpServers": {"figma": {"url": "https://figma.example.com/mcp"}}}),
    home / ".cursor/skills/review/SKILL.md": "---\nname: review\ndescription: Review code for bugs\n---\nLook for bugs.\n",
    # Copilot
    proj / ".github/copilot-instructions.md": "Be brief.\n",
    proj / ".github/instructions/py.instructions.md": "---\napplyTo: \"**/*.py\"\n---\nFollow PEP 8.\n",
    proj / ".github/prompts/review.prompt.md": "---\nmode: agent\ndescription: Review a diff\n---\nReview the staged diff.\n",
    proj / ".vscode/mcp.json": '{\n  // VS Code\n  "servers": {"playwright": {"type": "stdio", "command": "npx", "args": ["@playwright/mcp"]}}\n}\n',
    # Qoder
    proj / ".qoder/rules/db.md": "---\ntrigger: model_decision\ndescription: Use when writing SQL\n---\nAlways add an index.\n",
    home / ".qoder/AGENTS.md": "Personal Qoder preferences: answer in Chinese.\n",
    home / ".qoder/settings.json": json.dumps({"mcpServers": {"qoder-docs": {"command": "npx", "args": ["qoder-docs"], "alwaysAllow": ["search"]}}}),
    # CodeBuddy
    proj / ".codebuddy/rules/ts.md": "---\nalwaysApply: false\npaths: src/**/*.ts\n---\nStrict TypeScript.\n",
    proj / ".codebuddy/rules/off.md": "---\nenabled: false\n---\nNot loaded.\n",
    home / ".codebuddy/CODEBUDDY.md": "Prefer pnpm over npm.\n",
    home / ".codebuddy/skills/data/Analysis Tool/SKILL.md": "---\ndescription: Analyze CSV data\nallowed-tools: Read, Bash\n---\nRun ${CODEBUDDY_SKILL_DIR}/scripts/stats.py\n",
    home / ".codebuddy/skills/data/Analysis Tool/scripts/stats.py": "print('stats')\n",
    # Claude Code
    proj / "CLAUDE.md": "Architecture notes: @docs/arch.md and @~/personal.md\n",
    proj / "CLAUDE.local.md": "My sandbox URL is http://localhost:4000\n",
    proj / ".claude/rules/api.md": "---\npaths:\n  - api/**\n---\nValidate all input.\n",
    proj / ".mcp.json": json.dumps({"mcpServers": {
        "github": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"}},
        "docs": {"type": "http", "url": "https://docs.example.com/mcp"},
        "socket": {"type": "ws", "url": "wss://example.com/socket"},
    }}),
    proj / ".claude/skills/pdf/SKILL.md": "---\nname: pdf\ndescription: Read and fill PDF forms\ncontext: fork\n---\nUse scripts/fill.sh\n",
    proj / ".claude/skills/pdf/scripts/fill.sh": "#!/bin/sh\necho fill\n",
    proj / "docs/arch.md": "# Architecture\n",
    proj / "AGENTS.md": "Native Kiro file.\n",
    home / "personal.md": "personal\n",
    home / ".claude/CLAUDE.md": "Reply concisely.\n",
    home / ".claude.json": json.dumps({
        "oauthAccount": {"emailAddress": "someone@example.com"},
        "mcpServers": {"notes": {"command": "uvx", "args": ["notes-mcp"]}},
        "projects": {proj_abs: {"mcpServers": {"scratch-db": {"type": "http", "url": "https://db.example.com"}}}},
    }),
    # Something that must not be imported.
    proj / ".cursor/rules/leak.mdc": "---\nalwaysApply: true\n---\nStaging key is AKIAABCDEFGHIJKLMNOP\n",
    # A Kiro setup that already exists.
    home / ".kiro/settings/mcp.json": json.dumps({"mcpServers": {"mine": {"command": "mine", "args": []}}}),
}

# Claude Code keeps auto memory under a directory named after the project path.
name = "".join(c if (c.isalnum() and c.isascii()) else "-" for c in proj_abs)
mem = home / ".claude/projects" / name / "memory"
files[mem / "MEMORY.md"] = "- [Testing](feedback_testing.md)\n"
files[mem / "feedback_testing.md"] = "---\nname: Testing\ndescription: how to test\ntype: feedback\n---\nUse a real database, no mocks.\n"

for p, text in files.items():
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)
os.chmod(proj / ".claude/skills/pdf/scripts/fill.sh", 0o755)
(proj / ".git/info").mkdir(parents=True, exist_ok=True)
print(f"HOME={home}\nPROJECT={proj}")
