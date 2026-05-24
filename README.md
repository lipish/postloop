# il

**One command to record everything.**

```bash
il run cursor
```

`il` is the simplest way to record your full interactive sessions with Cursor, Claude, Copilot, Kimi, Aider — or any terminal tool.

It captures every keystroke, the complete PTY output, conversation turns, and gives you a ring buffer to replay the tail later. All privately on your machine.

No config file required for agents you have already installed and logged into.

## Install

### macOS (recommended)

Download the `.pkg` from [GitHub Releases](https://github.com/EeroEternal/IntentLoop/releases) and install:

```bash
sudo installer -pkg IntentLoop-*.pkg -target /
```

The package installs `il` to `/usr/local/bin/il`.

### Linux / build from source

Requires Rust:

```bash
cargo install --path .
```

Or:

```bash
git clone https://github.com/EeroEternal/IntentLoop.git
cd IntentLoop
cargo build --release
# copy target/release/il to a directory in $PATH (e.g. ~/.local/bin)
```

## The only command you will use

```bash
il run cursor
il run claude
il run kimi
# any agent or tool that already works in your terminal
```

- Launches the program exactly as you normally would (full environment, API keys, login state, everything).
- Records the complete interactive PTY session.
- Persists session data through `memmap_fs` under `~/.intentloop`.

No `agents.toml` is needed unless you want custom shell activation (conda, venv, etc.).

## Inspect what happened

```bash
il list
il show <session-id>
il attach <session-id>     # replay the last ~2000 characters of the session
il dump <session-id> stdout
```

## Optional: .intent/agents.toml (advanced)

Only create this file when you need:

- Custom shell setup (`source .venv/bin/activate`, `conda activate`, nvm, etc.)
- Always pass the same extra flags
- Give a short alias to a long command

Most users never need this file. If you do, create `.intent/agents.toml` manually with an
`[agents.<name>]` entry that specifies `command`, optional `args`, optional `shell_setup`,
and optional `env_whitelist`.

## Commands

| Command                  | What it does                                      |
|--------------------------|---------------------------------------------------|
| `il run <name>`          | Launch `<name>` (from PATH or agents.toml) and record the full PTY session |
| `il list`                | List recent recorded sessions                     |
| `il show <id>`           | Show metadata and stored stream references        |
| `il attach <id>`         | Replay the tail of the ring buffer (last ~2000 chars) |
| `il search <query>`      | Search recorded session conversations             |
| `il dump <id> <stream>`  | Dump a stored stream (`stdout`, `conversation`, `report`, etc.) |
| `il copilot ...`         | Run GitHub Copilot CLI inside a recorded session (advanced) |

The official macOS package only ships the `il` binary.

## Storage

All recordings go to `~/.intentloop` (or `$INTENTLOOP_HOME` if set). This keeps your git repos clean.
Session metadata, raw terminal streams, derived artifacts, and search indexes are persisted through `memmap_fs`.

```text
~/.intentloop/
  memmap_fs files              # KV + streams + search index + WAL
```

Default session data is not exposed as `sessions/<id>/*.jsonl` files. Use `il` to inspect or export it:

```bash
il dump <session-id> stdout
il dump <session-id> conversation
il dump <session-id> report --output report.md
```

```bash
export INTENTLOOP_HOME=/some/other/place   # optional, per-project storage
```

The only thing you ever put in your repo (optional) is `.intent/agents.toml` when you need custom launch profiles.

## What works today

- Zero-config recording of any agent already in your PATH (`il run cursor`, etc.)
- Full PTY capture (even TUI, arrow keys, multi-line prompts)
- Conversation extraction + ring buffer for instant tail replay (`il attach`)
- Full-text search across extracted conversations (`il search`)
- Optional advanced profiles in `.intent/agents.toml`

Roadmap (soon):

- Live attach to a running session
- One-command rewind to the state before the session
- Git commit message footer injection
- Cross-session semantic search over your history

## Environment & Secrets

`il` never touches your secrets. Whatever environment variables and login state (Keychain, `~/.config/...`, etc.) your agent normally sees, `il run <agent>` will see exactly the same thing.

No `env_whitelist` gymnastics needed for normal use.

## GitHub Copilot CLI

If you use `gh copilot`, you can record those sessions too:

```bash
il copilot -- suggest "fix the auth bug"
```

See `il copilot --help` for the full set of options.

## License

MIT
