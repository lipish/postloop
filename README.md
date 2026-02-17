# ploop

**Post-commit Loop** - Local Git Auto-Deployment Tool

[English](#english) | [中文](#中文)

---

## English

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

---

## 中文

### 项目概述

ploop 是一个用 Rust 实现的本地 Git 自动部署工具，面向单人开发者。核心理念是：代码提交后自动完成"构建 → 部署 → 同步到 GitHub"的完整闭环。

### 核心功能

#### 1. Post-commit Hook 触发
- 提供 `ploop init` 命令，在目标 Git 仓库中安装 post-commit hook
- hook 脚本在每次 `git commit` 后自动触发 ploop 执行部署流程

#### 2. 自动构建
- 根据配置文件中的 `build.command` 执行构建命令
- 支持任意构建命令（如 `cargo build --release`、`npm run build`、`go build` 等）
- 构建失败时记录日志并终止流程

#### 3. 自动部署
- 支持两种部署方式：
  - **进程部署**：执行自定义部署命令（如 `systemctl restart app.service`）
  - **文件部署**：将构建产物复制到目标目录（`target_dir`）
- 部署失败时支持回滚到上一个版本

#### 4. 同步到 GitHub
- 部署成功后自动执行 `git push` 将代码同步到远端 GitHub 仓库
- 在配置文件中指定 remote 名称和分支
- push 失败时记录日志但不影响本地部署结果

#### 5. 回滚支持
- 保留最近 N 个版本的构建产物（可配置）
- 部署失败时自动回滚到上一个成功版本

#### 6. 日志记录
- 所有操作（构建、部署、同步、回滚）记录到本地日志文件
- 日志包含时间戳、commit hash、操作结果

### 安装

```bash
cargo install --path .
```

或从源码构建：

```bash
git clone https://github.com/lipish/ploop.git
cd ploop
cargo build --release
sudo cp target/release/ploop /usr/local/bin/
```

### 快速开始

1. 在你的 Git 仓库中初始化 ploop：
```bash
cd /path/to/your/repo
ploop init
```

2. 编辑生成的 `deploy.toml` 配置文件：
```toml
[build]
command = "cargo build --release"

[deploy]
target_dir = "/opt/deploy"
artifacts = ["target/release/my-app"]
```

3. 提交代码，ploop 会自动部署：
```bash
git add .
git commit -m "你的修改"
# ploop 通过 post-commit hook 自动运行
```

4. 或手动运行：
```bash
ploop run
```

### CLI 命令

- `ploop init` — 在当前 Git 仓库中初始化 ploop（安装 post-commit hook，生成默认 deploy.toml）
- `ploop run` — 手动触发一次完整的 构建→部署→同步 流程
- `ploop rollback` — 手动回滚到上一个版本
- `ploop rollback --version <hash>` — 回滚到指定版本
- `ploop status` — 查看当前部署状态和最近的部署历史
- `ploop log` — 查看部署日志
- `ploop log --lines 100` — 查看最后 100 行日志

### 配置文件 (deploy.toml)

查看 `deploy.toml.example` 获取完整配置示例。

```toml
[watch]
repo_path = "."
branch = "main"

[build]
command = "cargo build --release"

[deploy]
# 方式一：进程部署
command = "systemctl restart app.service"

# 方式二：文件部署
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

### 系统要求

- Git
- Rust 1.70+ (用于构建)

### 平台支持

- ✅ Linux
- ✅ macOS
- ✅ Windows

### 开源协议

MIT
