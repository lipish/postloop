# intent

Intent - Interactive Agent Session Recorder

## Overview

`intent` is a lightweight Rust CLI for personal AI-assisted development.
It wraps any agent (Cursor, Claude, Copilot, etc.), records the full interactive session, and stores:

- Session metadata
- Intent reference from `INTENT.md`
- Raw terminal output
- Minimal Markdown report

This is the Phase 1-Lite scope: `run` + `copilot` + `show` with full PTY interaction and structured event capture.

## Install

```bash
cargo install --path .
```

Or build locally:

```bash
cargo build --release
./target/release/intent --help
```

## Quick Start

1) (Optional) Create `INTENT.md` in repo root:

```md
id: auth-jwt-001
title: Login refactor to JWT
```

2) Run an agent interactively:

```bash
intent run --agent cursor
```

This launches Cursor (or any agent defined in `.intent/agents.toml`) in full PTY interactive mode and records everything.

You can also pass extra args:

```bash
intent run --agent cursor -- --some-flag
```

Or run any command:

```bash
intent run -- echo "hello"
```

Copy `.intent/agents.toml.example` to `.intent/agents.toml` (current dir or `~/.intent/`) to add Cursor, Claude, Codex, etc.

Or run GitHub Copilot CLI directly:

```bash
intent copilot
```

3) Inspect a session:

```bash
intent show <session-id>
```


## Commands

- `intentloop run -- <agent-cli> [args...]`
  - Runs the command and records a session in `~/.intentloop/` (or `$INTENTLOOP_HOME`).
- `intentloop copilot [--mode auto|copilot|agent-task] [--prompt "..."] [-- <gh args...>]`
  - Runs GitHub CLI agent command in a recorded session.
  - Uses PTY interactive mode by default (full terminal interaction + transcript capture).
  - Add `--non-interactive` to use one-shot capture mode.
  - Add `--wait` to wait for final result when using `gh agent-task` (`create --follow`).
  - If no args are provided, it builds a prompt from `INTENT.md` and runs:
    - `gh copilot suggest <prompt>` (copilot mode)
    - `gh agent-task create <prompt>` (agent-task mode)

Wait for final result example:

```bash
intent copilot --mode agent-task --wait
```
- `intent show <session-id>`
  - Shows session metadata and log/report paths.

## Storage Layout

```text
~/.intent/                  # or $INTENT_HOME
  db.sqlite
  sessions/
    <session_id>/
      terminal.raw.log
      events.jsonl
      report.md
```

You can override the storage root with:

```bash
export INTENT_HOME=/path/to/your/session-store
```

## Current Scope

Included:
- Interactive PTY session recording (`intent run --agent xxx`)
- SQLite + raw log + structured events
- Minimal Markdown report
- Agent profiles via `.intent/agents.toml`

Not included yet:
- Rewind / snapshot restore
- Git hooks
- Semantic search

## Environment Variables (.env)

- Auto-loads `.env` from current directory (via `dotenvy`).
- Quick start: `cp .env.example .env` and fill your API key/model settings.

Example:

```bash
LLM_PROVIDER=openai
LLM_MODEL=gpt-4o
LLM_API_KEY=your_key
```

## Copilot CLI Prerequisites

- Install GitHub CLI (`gh`)
- Install/enable Copilot CLI support for `gh` (or use `gh agent-task` preview commands)
- Run `gh auth login` and ensure `gh copilot` works in your shell

## License

MIT
