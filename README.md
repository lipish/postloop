# intentloop

IntentLoop Lite - Local Agent Session Recorder

## Overview

IntentLoop Lite is a Rust CLI for personal AI-assisted development.
It wraps an agent command, records one session end-to-end, and stores:

- Session metadata
- Intent reference from `INTENT.md`
- Raw terminal output
- Minimal Markdown report

This is the Phase 1-Lite scope: `run` + `copilot` + `show`, plus optional `zene` integration.

## Install

```bash
cargo install --path .
```

Or build locally:

```bash
cargo build --release
./target/release/intentloop --help
```

## Quick Start

1) (Optional) Create `INTENT.md` in repo root:

```md
id: auth-jwt-001
title: Login refactor to JWT
```

2) Run an agent command through IntentLoop:

```bash
intentloop run -- echo "hello-intentloop"
```

Or run a Copilot CLI session directly:

```bash
intentloop copilot
```

`intentloop copilot` defaults to `--mode auto`:
- uses `gh copilot` when available
- otherwise falls back to `gh agent-task`

Or pass raw args to `gh copilot`:

```bash
intentloop copilot -- suggest "refactor auth module with safer token handling"
```

Force backend explicitly:

```bash
intentloop copilot --mode copilot -- suggest "analyze auth flow"
intentloop copilot --mode agent-task -- create "Refactor auth module"
```

3) Inspect the session:

```bash
intentloop show <session-id>
```

Run with embedded Zene event stream:

```bash
cargo run --features zene -- zene
```

Or provide a prompt explicitly:

```bash
cargo run --features zene -- zene --prompt "Analyze this repository and propose refactoring plan"
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
intentloop copilot --mode agent-task --wait
```
- `intentloop show <session-id>`
  - Shows session metadata and log/report paths.
- `intentloop zene [--prompt "..."] [--zene-session-id "..."]`
  - Requires `--features zene` at build/run time.
  - Runs Zene as embedded Rust library.
  - Consumes `run_envelope_stream` and records all event envelopes to `events.jsonl`.

## Storage Layout

```text
~/.intentloop/                  # or $INTENTLOOP_HOME
  db.sqlite
  sessions/
    <session_id>/
      terminal.raw.log
      events.jsonl
      report.md
```

You can override the storage root with:

```bash
export INTENTLOOP_HOME=/path/to/your/session-store
```

## Current Scope (Lite)

Included:
- `sessions` + `thought_events` in SQLite
- Raw terminal log capture
- Minimal report generation

Not included yet:
- Rewind
- Artifacts hash/diff
- Git hooks
- Search / LanceDB

## Environment Variables (.env)

- IntentLoop now auto-loads `.env` from the current working directory (and parent chain, via `dotenvy`).
- This means `intentloop zene` can read the same model/provider variables used by `zene` directly.
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
