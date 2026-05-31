# il

**One command to record everything.**

```bash
il run cursor
```

`il` is the simplest way to record your full interactive sessions with Cursor, Claude, Copilot, Kimi, Aider — or any terminal tool.

It captures every keystroke, the complete PTY output, conversation turns, and gives you a ring buffer to replay the tail later. All privately on your machine.

No config file required for agents you have already installed and logged into.

## Install

### One-line install (recommended)

```bash
curl -fsSL https://intentloop.dev/install.sh | sh
```

- Installs to `~/.local/bin/il` (no sudo, no password prompt)
- Downloads a prebuilt binary when available (fast, no compilation)
- Falls back to building from source only when necessary (with a clear warning)

### Advanced options

**Specific version**

```bash
INTENTLOOP_VERSION=0.5.0 curl -fsSL https://intentloop.dev/install.sh | sh
```

**System-wide installation** (`/usr/local/bin`, requires sudo on macOS)

```bash
curl -fsSL https://intentloop.dev/install.sh | sh -s -- --system
```

Or manually download the `.pkg` from [GitHub Releases](https://github.com/EeroEternal/IntentLoop/releases) and run:

```bash
sudo installer -pkg IntentLoop-*.pkg -target /
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

**No `agents.toml` or any launch configuration file is supported or needed.** All shell activation, environment, and login state come from your normal terminal setup.

## Inspect what happened

After a session, you rarely need the ID anymore:

```bash
il list                 # recent sessions, newest first
il last                 # the most recent session (recommended)
il show                 # same as `il last` (defaults to latest)
il dump chat            # view the conversation (recommended)
il dump report          # export the report of the latest session
il dump stdout -o out.txt
```

You can still pass an explicit ID when needed: `il show <id>`.

## Commands

| Command                  | What it does                                      |
|--------------------------|---------------------------------------------------|
| `il run <name>`          | Launch `<name>` (exactly as it works in your normal shell) and record the full PTY session |
| `il list [--limit N]`    | List recent sessions (newest first, default 10)   |
| `il last`                | Show the most recent session                        |
| `il show [id]`           | Show metadata/streams (defaults to latest)        |
| `il search <query>`      | Full-text search across conversations             |
| `il dump [id] <stream>`  | Dump any stream of latest (or given) session      |

The official macOS package only ships the `il` binary.

## Storage

All recordings go to `~/.intentloop` (or `$INTENTLOOP_HOME` if set). This keeps your git repos clean.
Session metadata, raw terminal streams, derived artifacts, and search indexes are persisted through `memmap_fs`.

```text
~/.intentloop/
  memmap_fs files              # KV + streams + search index + WAL
```

Default session data is not exposed as files. Use `il` to inspect or export (most commands default to the latest session):

```bash
il dump stdout
il dump chat
il dump conversation     # raw JSONL if needed
il dump report --output report.md
# or with explicit id when needed: il dump <id> report
```

```bash
export INTENTLOOP_HOME=/some/other/place   # optional, per-project storage
```

You never put any IntentLoop-specific launch configuration in your repo. The recorder is completely decoupled from how your agents are activated.

## What works today

- Zero-config recording of any agent already in your PATH (`il run cursor`, etc.)
- Full PTY capture (even TUI, arrow keys, multi-line prompts)
- Full-text search across extracted conversations (`il search`)

Roadmap (soon):

- One-command rewind to the state before the session
- Git commit message footer injection
- Cross-session semantic search over your history

## Environment & Secrets

`il` never touches your secrets. Whatever environment variables and login state (Keychain, `~/.config/...`, etc.) your agent normally sees, `il run <agent>` will see exactly the same thing.

No `env_whitelist` gymnastics needed for normal use.

## GitHub Copilot CLI

Just record it like any other tool:

```bash
il run gh copilot -- suggest "fix the auth bug"
il run gh agent-task create "Add login feature"
```

## License

MIT

---

## For Maintainers: Deploying intentloop.dev

The website lives in the `web/` directory and is deployed to Cloudflare Pages.

We use **Wrangler** for configuration (so everything is in the repo).

### One-time setup

```bash
# Install Wrangler (if you haven't already)
npm install -g wrangler

# Login to Cloudflare
wrangler login
```

### Deploy the site

```bash
# From the root of the repo
wrangler pages deploy . --project-name=intentloop
```

This uses `wrangler.toml` (which points to `web/` as the output directory).

After deployment:
- `https://intentloop.dev/` → `web/index.html`
- `https://intentloop.dev/install.sh` → `web/install.sh` (the one-line installer)

### Custom headers

We also have `web/_headers` which ensures `install.sh` is served with the correct `Content-Type: text/plain`.

### Alternative (explicit directory)

You can also deploy just the web folder directly:

```bash
wrangler pages deploy web --project-name=intentloop
```

Both commands are equivalent thanks to the `wrangler.toml` configuration.
