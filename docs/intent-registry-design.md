# IntentLoop 设计文档

> **文档状态**：设计稿，持续迭代中。
>
> **当前实现**：命令行 `il`，会话统一持久化到 `memmap_fs`，默认根目录为 `~/.intentloop`。
>
> **存储实现**：使用 **memmap_fs**（内存映射文件系统）提供：
> - 会话元数据高效存储
> - 大日志流式读写（避免 OOM）
> - 全文检索（可选）

---

## 1. 项目定位

### 1.1 核心定位

**IntentLoop 是一个 AI Agent 会话记录器**，专注于：

- 包装 AI CLI（Claude Code / Cursor / Copilot CLI 等），透明捕获交互过程
- 记录完整的终端 I/O 流（stdin/stdout）
- 提取结构化对话内容（user prompt / agent response）
- 提供可检索的会话历史

**Slogan**：AI Agent 的黑匣子 —— 完整记录每一次 AI 编码会话的过程与推理轨迹。

### 1.2 边界（不做什么）

以下功能**不属于 IntentLoop 范围**，应由其他工具完成：

| 功能 | 应使用的工具 |
|------|-------------|
| 代码版本控制 | Git |
| 文件快照与回退 | Git |
| 代码 diff 与 patch | Git diff |
| 分支管理 | Git |

**原则**：IntentLoop 记录"过程"，Git 记录"结果"，各司其职。

### 1.3 成功标准

- ✅ `il run claude` 完整捕获一次会话
- ✅ `il list` 查看历史会话
- ✅ `il show <id>` 输出会话报告
- ✅ 会话崩溃时数据不丢失

### 1.4 明确不做

- ❌ 文件快照（snapshot）
- ❌ 代码回退（rewind）
- ❌ 文件变化追踪（artifact tracking）
- ❌ Git Hook 集成
- ❌ 多人协作 / 云同步

---

## 2. 系统架构

### 2.1 模块结构

```text
il (Rust CLI)
  ├─ Session Manager      # 创建/关闭会话
  ├─ PTY Wrapper          # 运行并捕获 Agent CLI I/O
  ├─ Conversation Extract # 提取对话内容
  ├─ Markdown Reporter    # 生成报告
  └─ Registry Store       # 会话存储
       └─ memmap_fs
```

### 2.2 技术栈

| 组件 | 选型 |
|------|------|
| CLI | `clap` |
| PTY | `portable-pty` |
| Terminal | `crossterm` (raw mode) |
| VT100 解析 | `vt100` |
| 序列化 | `serde` + `serde_json` |
| 时间 | `chrono` |
| 日志 | `tracing` (计划中) |

---

## 3. 数据模型

### 3.1 存储布局

```text
~/.intentloop/
  memmap_fs files            # KV + stream + search index + WAL

memmap_fs keys:
  sessions/{session_id}              # SessionSummary JSON
  sessions/{session_id}/stdout       # 原始 stdout stream
  sessions/{session_id}/stdin        # 原始 stdin stream
  sessions/{session_id}/stderr       # 非交互 stderr stream
  sessions/{session_id}/ring         # VT100 ring buffer
  sessions/{session_id}/events       # 结构化事件 JSONL
  sessions/{session_id}/normalized   # 归一化终端 JSONL
  sessions/{session_id}/conversation # 对话 JSONL
  sessions/{session_id}/thoughts     # 思考事件 JSONL
  sessions/{session_id}/report       # Markdown 报告
```

### 3.2 核心实体

```rust
struct SessionSummary {
    id: String,              // UUID v7
    agent_cmd: String,
    cwd: String,
    status: String,          // running/succeeded/failed/interrupted
    start_at: String,        // RFC3339
    end_at: Option<String>,
    exit_code: Option<i32>,
}

struct ConversationTurn {
    role: String,            // user/agent
    text: String,
    ts: String,
}
```

---

## 4. CLI 命令

```bash
il run <agent-cli> [args...]      # 包装执行并记录会话
il list [--limit N]               # 列出历史会话（最新在前）
il last                           # 查看最近一次会话（推荐，无需 ID）
il show [session-id]              # 显示会话详情（默认最新）
il dump [session-id] <stream>     # 查看或导出 memmap_fs stream（默认最新）
```

---

## 5. memmapFS 需求

### 5.1 会话元数据

- 按 session_id 快速查询
- 按时间范围列表

### 5.2 大日志流式读写

```rust
// 会话进行中实时追加
fn append_stdout(&self, session_id: &str, data: &[u8]);

// 回放时流式读取（不全量加载到内存）
fn stream_stdout(&self, session_id: &str) -> impl Read;
```

### 5.3 全文检索（可选）

```rust
fn search(&self, query: &str) -> Vec<SearchHit>;
```

---

## 6. 迭代计划

| 阶段 | 目标 | 状态 |
|------|------|------|
| MVP | run/list/show | ✅ 已完成 |
| P1 | memmap_fs 存储层 | ✅ 已完成 |
| P2 | 全文检索 | ✅ 已完成 |
| P2 | 结构化日志 (tracing) | 📋 计划中 |
