# il

**One command to record everything.**

```bash
il run cursor
```

`il` (also available as `intentloop` and `intent`) is the simplest way to record your full interactive sessions with Cursor, Claude, Copilot, Kimi, Aider — or any terminal tool.

It captures every keystroke, the complete PTY output, conversation turns, and gives you a ring buffer to replay the tail later. All privately on your machine.

No config file required for agents you have already installed and logged into.

## Install (Mac / Linux)

```bash
# 1. Build from source (Homebrew + prebuilt binaries coming soon)
git clone https://github.com/lipish/intent.git
cd intent
cargo build --release

# 2. Put the binary somewhere in your PATH (three names are identical)
mkdir -p ~/.local/bin
cp target/release/il ~/.local/bin/
ln -sf ~/.local/bin/il ~/.local/bin/intentloop
ln -sf ~/.local/bin/il ~/.local/bin/intent

# 3. Make sure ~/.local/bin is in PATH, then:
il --help
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
- Saves everything under `~/.intentloop/sessions/<id>/`

No `agents.toml` is needed unless you want custom shell activation (conda, venv, etc.).

## Inspect what happened

```bash
il list
il show <session-id>
il attach <session-id>     # replay the last ~2000 characters of the session
```

## Optional: INTENT.md

If you put an `INTENT.md` file with `id:` and `title:` at the top in your project root, the session is automatically tagged with that intent. Great for later searching or grouping related work.

Example:

```md
id: payment-refactor
title: Migrate billing to new payment provider
```

## Optional: .intent/agents.toml (advanced)

Only create this file when you need:

- Custom shell setup (`source .venv/bin/activate`, `conda activate`, nvm, etc.)
- Always pass the same extra flags
- Give a short alias to a long command

See `.intentloop/agents.toml.example` for the format. Most users never need this file.

## Commands

| Command                  | What it does                                      |
|--------------------------|---------------------------------------------------|
| `il run <name>`          | Launch `<name>` (from PATH or agents.toml) and record the full PTY session |
| `il list`                | List recent recorded sessions                     |
| `il show <id>`           | Show metadata and report location for a session   |
| `il attach <id>`         | Replay the tail of the ring buffer (last ~2000 chars) |
| `il copilot ...`         | Run GitHub Copilot CLI inside a recorded session (advanced) |

All `il`, `intentloop`, and `intent` are the exact same binary.

## Storage

All recordings go to `~/.intentloop` (or `$INTENTLOOP_HOME` if set). This keeps your git repos clean.

```text
~/.intentloop/
  sessions/
    <session-id>/
      meta.json
      terminal.raw.log          # original PTY stream
      conversation.jsonl
      terminal.ring.bin         # circular buffer for fast tail replay
      report.md
```

```bash
export INTENTLOOP_HOME=/some/other/place   # optional, per-project storage
```

The only thing you ever put in your repo (optional) is `.intent/agents.toml` when you need custom launch profiles.

## What works today

- Zero-config recording of any agent already in your PATH (`il run cursor`, etc.)
- Full PTY capture (even TUI, arrow keys, multi-line prompts)
- Conversation extraction + ring buffer for instant tail replay (`il attach`)
- Automatic tagging via optional `INTENT.md`
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
