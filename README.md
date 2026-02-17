# ploop

**Post-commit Loop** - Local Git Auto-Deployment Tool

### Overview

ploop is a local Git auto-deployment tool written in Rust, designed for solo developers. The core concept is: after code commit, automatically complete the "build → deploy → sync to GitHub" cycle.

### Features

#### 1. Post-commit Hook Trigger
- Provides `ploop init` command to install post-commit hook in the target Git repository
- Hook script automatically triggers ploop deployment process after each `git commit`

#### 2. Auto Build
- Executes build commands based on `build.command` in the configuration file
- Supports any build command (e.g., `cargo build --release`, `npm run build`, `go build`)
- Logs and terminates process on build failure

#### 3. Auto Deploy
- Supports two deployment methods:
  - **Process Deployment**: Execute custom deployment command (e.g., `systemctl restart app.service`)
  - **File Deployment**: Copy build artifacts to target directory (`target_dir`)
- Supports rollback to previous version on deployment failure

#### 4. Sync to GitHub
- Automatically executes `git push` to sync code to remote GitHub repository after successful deployment
- Specify remote name and branch in configuration file
- Logs push failures but doesn't affect local deployment result

#### 5. Rollback Support
- Keeps the most recent N versions of build artifacts (configurable)
- Automatically rolls back to last successful version on deployment failure

#### 6. Logging
- All operations (build, deploy, sync, rollback) logged to local log file
- Logs include timestamp, commit hash, operation results

### Installation

```bash
cargo install --path .
```

Or build from source:

```bash
git clone https://github.com/lipish/ploop.git
cd ploop
cargo build --release
sudo cp target/release/ploop /usr/local/bin/
```

### Quick Start

1. Initialize ploop in your Git repository:
```bash
cd /path/to/your/repo
ploop init
```

2. Edit the generated `deploy.toml` configuration file:
```toml
[build]
command = "cargo build --release"

[deploy]
target_dir = "/opt/deploy"
artifacts = ["target/release/my-app"]
```

3. Commit your code, and ploop will automatically deploy:
```bash
git add .
git commit -m "Your changes"
# ploop runs automatically via post-commit hook
```

4. Or run manually:
```bash
ploop run
```

### CLI Commands

- `ploop init` — Initialize ploop in current Git repository (installs post-commit hook, generates default deploy.toml)
- `ploop run` — Manually trigger complete build→deploy→sync pipeline
- `ploop rollback` — Manually rollback to previous version
- `ploop rollback --version <hash>` — Rollback to specific version
- `ploop status` — View current deployment status and recent deployment history
- `ploop log` — View deployment logs
- `ploop log --lines 100` — View last 100 lines of logs

### Configuration File (deploy.toml)

See `deploy.toml.example` for a complete configuration example.

```toml
[watch]
repo_path = "."
branch = "main"

[build]
command = "cargo build --release"

[deploy]
# Option 1: Process deployment
command = "systemctl restart app.service"

# Option 2: File deployment
target_dir = "/opt/deploy"
artifacts = ["target/release/my-app"]

[sync]
enabled = true
remote = "origin"
branch = "main"

[rollback]
enabled = true
keep_versions = 3

[log]
file = "ploop.log"
level = "info"
```

### Requirements

- Git
- Rust 1.70+ (for building)

### Platform Support

- ✅ Linux
- ✅ macOS
- ✅ Windows

### License

MIT
