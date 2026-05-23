# intent

Intent - Interactive Agent Session Recorder

## Overview

`intent` is a lightweight Rust CLI for personal AI-assisted development.
It wraps any agent (Cursor, Claude, Copilot, etc.), records the full interactive session, and stores:

- Session metadata (`meta.json`)
- Intent reference from `INTENT.md`
- Raw terminal output
- Structured events, conversation, and VT100 snapshots
- Ring buffer for tail replay
- Minimal Markdown report

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
intent list
intent attach <session-id>   # replay ring buffer tail
```

## Commands

- `intent run [--agent <name>] [--non-interactive] [-- <command...>]`
  - Runs the command and records a session.
- `intent copilot [--mode auto|copilot|agent-task] [--prompt "..."] [--wait] [--non-interactive] [-- <gh args...>]`
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

- `intent show <session-id>` — show session metadata and artifact paths
- `intent list` — list recent sessions
- `intent attach <session-id>` — print the saved ring buffer tail for a completed PTY session

## Storage Layout

```text
~/.intentloop/              # or $INTENTLOOP_HOME
  sessions/
    <session_id>/
      meta.json
      terminal.raw.log
      thought_events.jsonl
      events.jsonl
      conversation.jsonl
      terminal.normalized.jsonl
      terminal.ring.bin
      report.md
```

You can override the storage root with:

```bash
export INTENTLOOP_HOME=/path/to/your/session-store
```

Agent profiles (config) are loaded from `.intent/agents.toml` (walk up), `~/.intent/agents.toml`, or legacy `.intentloop/agents.toml`. Session data (recordings) always defaults to `~/.intentloop` (or `$INTENTLOOP_HOME`), keeping repos clean; set the env var for per-repo storage and ignore it in git.

## Current Scope

Included:

- Interactive PTY session recording (`intent run --agent xxx`)
- JSON session store + raw log + structured events
- Conversation extraction via vt100 replay
- Ring buffer tail replay via `intent attach`
- Minimal Markdown report
- Agent profiles via `.intent/agents.toml` (`env_whitelist`, `shell_setup`, etc.)

Not included yet:

- Live attach to running sessions
- Rewind / snapshot restore
- Git hooks
- Semantic search

## Environment Variables (.env)

- Auto-loads `.env` from current directory (via `dotenvy`).
- Agent `env_whitelist` in `agents.toml` passes selected vars into the spawned process.

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
