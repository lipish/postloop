# IntentLoop 开发指南

## 项目定位

**IntentLoop 是一个 AI Agent 会话记录器**，专注于：
- 包装 AI CLI（Claude Code / Cursor / Copilot CLI 等），透明捕获交互过程
- 记录完整的终端 I/O 流（stdin/stdout）
- 提取结构化对话内容（user prompt / agent response）
- 提供可检索的会话历史

**Slogan**：AI Agent 的黑匣子 —— 完整记录每一次 AI 编码会话的过程与推理轨迹。

## 边界定义

### IntentLoop 负责

| 功能 | 说明 |
|------|------|
| PTY 包装 | 启动 Agent CLI，透明捕获 stdin/stdout |
| 会话管理 | 创建、关闭、列表、查询会话 |
| 对话提取 | 从终端日志中提取 user prompt / agent response |
| 报告生成 | 生成 Markdown 格式的会话报告内容并写入 `memmap_fs` |
| CLI 命令 | `il run`, `il list`, `il show`, `il attach`, `il search`, `il dump` |

### IntentLoop 不负责

| 功能 | 应使用的工具 |
|------|-------------|
| 代码版本控制 | Git |
| 文件快照与回退 | Git |
| 代码 diff 与 patch | Git diff |
| 分支管理 | Git |

---

## memmap_fs 集成边界

IntentLoop 使用 `memmap_fs` crate 作为存储层。

原则：`memmap_fs` 是会话数据的唯一默认持久化来源；IntentLoop 不再为会话维护并行的
`sessions/{id}/*.jsonl` / `report.md` / `terminal.*.raw` 文件。文件只在用户显式 `il dump --output`
导出时生成。

### memmap_fs 负责（存储层）

| 功能 | API |
|------|-----|
| KV 存储 | `set_kv()`, `get_kv()`, `delete_kv()` |
| 流式大对象 | `append_stream()`, `open_read()` |
| 全文检索 | `index()`, `search()` |
| WAL 持久化 | 自动 |
| 崩溃恢复 | `init()` 时自动重放 WAL |

### IntentLoop 负责（应用层）

| 功能 | 说明 |
|------|------|
| 数据结构定义 | `SessionSummary` 等业务实体 |
| 序列化/反序列化 | JSON/bincode 编码 |
| Key 命名约定 | `sessions/{id}`, `sessions/{id}/stdout` 等 |
| PTY 捕获适配 | 将 PTY stdin/stdout 写入抽象为 `Write` sink |
| 会话状态管理 | `running`/`succeeded`/`failed`/`interrupted` |
| 僵尸会话检测 | 启动时检查 `running` 状态但进程已死的会话 |

### Key 命名约定

```
sessions/{session_id}              # SessionSummary JSON
sessions/{session_id}/stdout       # 终端 stdout 流
sessions/{session_id}/stdin        # 终端 stdin 流
sessions/{session_id}/stderr       # 非交互执行 stderr 流
sessions/{session_id}/ring         # VT100 ring buffer
sessions/{session_id}/events       # 结构化 PTY events JSONL
sessions/{session_id}/normalized   # VT100 snapshots JSONL
sessions/{session_id}/conversation # 对话 turns JSONL
sessions/{session_id}/thoughts     # thought events JSONL
sessions/{session_id}/report       # Markdown report
```

### 边界约束

- `memmap_fs` 只提供通用 KV、stream、search，不引入 IntentLoop 的业务结构体。
- IntentLoop 只通过 `src/storage.rs` 封装访问 `memmap_fs`，避免业务代码散落直接调用存储 API。
- PTY 层只依赖 `std::io::Write`，不关心底层是文件、内存还是 `memmap_fs` stream。
- 默认情况下不创建 `~/.intentloop/sessions/{id}` 业务文件目录；所有查看、搜索、导出通过 `il` 命令完成。

---

## 开发规范

### 代码风格
- 使用 `cargo fmt` 格式化
- 使用 `cargo clippy` 检查
- 所有公开 API 需要文档注释

### 测试
- 单元测试放在模块内 `#[cfg(test)]`
- 集成测试放在 `tests/` 目录
- 运行 `cargo test` 确保所有测试通过

### 依赖管理
- 使用 `cargo add/remove` 管理依赖
- 不要手动编辑 `Cargo.toml` 的依赖部分

