# Zene 集成需求（Issue 模板）

## 标题

feat(zene): 暴露可消费的完整 Agent 事件流（run_stream + session events）供 IntentLoop 记录全过程

---

## 背景

IntentLoop 当前已具备会话记录能力，但主要依赖命令级 stdout/stderr。
在 Agent 场景下，仅记录命令日志无法反映真实推理与决策过程，价值有限。

Zene 已具备 `AgentEvent` 和 `run_stream` 能力，因此需要将该能力对外稳定暴露，作为 IntentLoop 的标准事件输入源。

目标：将“日志级记录”升级为“事件级轨迹记录”。

---

## 目标（Goals）

1. 提供稳定的流式事件接口（不仅返回最终结果）。
2. 确保事件可重放、可审计、可断点续拉。
3. 向后兼容现有 `agent.run` 行为。
4. 让 IntentLoop 可无损记录完整会话轨迹。

---

## 非目标（Non-Goals）

1. 不要求暴露模型内部隐藏思维链（hidden CoT）。
2. 不在本需求中实现复杂 UI。
3. 不在本需求中引入企业级权限系统。

---

## P0 必做需求

### 1) 新增 JSON-RPC 方法：`agent.run_stream`

- 输入：
  - `instruction: string`
  - `session_id: string`
  - `run_id?: string`（若未传，服务端生成并回传）
  - `env_vars?: object`

- 输出模式：
  - 先返回 `accepted`（含 `run_id`）
  - 后续通过 notification 流式推送事件

> 要求：事件推送不能只依赖 stderr 文本，必须是结构化 JSON 事件。

### 2) 统一事件 Envelope（强约束）

每条事件必须包含：

```json
{
  "run_id": "string",
  "session_id": "string",
  "seq": 12,
  "ts": "2026-03-01T08:00:00.000Z",
  "event_type": "ToolCall",
  "payload": {}
}
```

字段约束：
- `seq`：单调递增、不可重复。
- `ts`：ISO-8601 UTC。
- `event_type`：枚举值必须稳定。

### 3) 终止语义（强约束）

每个 `run_id` 必须且只会有一个终止事件：
- `Finished`
- 或 `Error`

不可两者同时出现，不可缺失。

### 4) 事件类型覆盖（至少）

- `PlanningStarted`
- `PlanGenerated`
- `TaskStarted`
- `ThoughtDelta`
- `ToolCall`
- `ToolOutputDelta`
- `ToolResult`
- `FileStateChanged`
- `ReflectionStarted`
- `ReflectionResult`
- `Finished`
- `Error`

### 5) 取消能力：`agent.cancel(run_id)`

- 可取消运行中的任务。
- 取消后必须发终止事件：
  - `Error { code: "Cancelled", message: "..." }`

### 6) 会话事件查询：`session.get_events`

方法：`session.get_events(session_id, cursor, limit)`

- 支持断线续拉。
- `cursor` 语义清晰（建议用最后 `seq`）。
- 保证同一 session 重放顺序一致。

### 7) 事件持久化（服务端）

每个 session 至少落地：
- `events.jsonl`（完整事件流）
- `result.json`（最终输出与统计）

默认路径保持当前体系（如 `~/.zene/sessions`），并支持配置覆盖。

### 8) 脱敏规则（强约束）

在事件 payload 和日志中对以下信息统一脱敏：
- API Keys
- Bearer Token
- Authorization
- Cookie / Set-Cookie
- 其他高风险密钥字段

---

## P1 建议需求（增强）

1. `zene run --json-events`：stdout 直接输出结构化事件流。
2. `zene run --events-file <path>`：直接写 jsonl。
3. `zene run --session <id>`：显式复用会话。
4. 事件 schema 文档与版本号（如 `event_schema_version`）。

---

## 向后兼容要求

1. `agent.run` 现有行为保持不变。
2. 新能力通过新方法或显式参数开启。
3. 旧客户端无需修改即可继续使用。

---

## 验收标准（Definition of Done）

1. 提供最小 Demo：
   - 启动 server
   - 调用一次 `agent.run_stream`
   - 收到多条事件并以 `Finished/Error` 结束

2. 提供真实样例：
   - 一份 `events.jsonl`
   - 至少 20 条事件
   - 覆盖 tool/file/reflection 事件

3. 测试通过：
   - 事件顺序与终止语义
   - 断线重连后的 cursor 续拉
   - 取消语义（cancel）
   - 脱敏规则校验

---

## IntentLoop 对接预期（供双方对齐）

IntentLoop 侧将按事件 envelope 直接落盘为：
- `~/.intentloop/sessions/<session_id>/events.jsonl`
- `~/.intentloop/sessions/<session_id>/terminal.raw.log`（可选）
- `~/.intentloop/sessions/<session_id>/report.md`

并基于事件流生成：
- 决策时间线
- 工具调用证据
- 文件变更证据
- 失败路径与恢复建议

---

## 风险与处理

1. **事件风暴导致吞吐压力**
   - 处理：批量 flush + backpressure + 限流。

2. **不同 provider 事件粒度不一致**
   - 处理：服务端归一化为统一 event schema。

3. **隐私泄漏风险**
   - 处理：严格脱敏 + 可配置字段黑名单。

---

## 建议里程碑

### M1（1 周）
- `agent.run_stream` + 统一事件 envelope + 终止语义

### M2（1 周）
- `session.get_events` + cursor 重放 + 取消能力

### M3（1 周）
- 持久化 + 脱敏 + 文档 + 示例

---

## 当前依赖冲突对齐清单（可执行）

背景：IntentLoop 启用 `--features zene` 时，`zene` 在 `tree-sitter` 上出现双版本冲突：
- `zene` 直依赖：`tree-sitter = 0.20.10`
- `tree-sitter-rust = 0.20.3` 实际拉入：`tree-sitter = 0.26.6`

这会导致 `tree_sitter::Language` 类型来自不同版本，编译报 `E0308 mismatched types`。

### P0（必须）

1. 统一 `zene` 的 `tree-sitter` 主版本到单一版本（建议 `0.26.x`）。
2. 同步检查并对齐以下依赖，避免混用：
   - `tree-sitter`
   - `tree-sitter-rust`
   - `tree-sitter-typescript`
3. 修改 `zene/src/engine/context.rs` 到同一 API 语义（`Parser::set_language`、`Query::new` 的参数类型必须来自同一 crate 版本）。
4. 执行验证：
   - `cargo clean`
   - `cargo build`
   - `cargo tree -d | grep -i tree-sitter`
   - 结果应只剩单一 `tree-sitter` 版本。

### P1（建议）

1. 在 `zene` 仓库增加 CI 检查：检测重复 `tree-sitter` 版本并 fail。
2. 在 `zene` 发布说明中固定 parser 依赖策略，避免下游二次冲突。
3. 对 `context` 模块补最小单测（Rust/TS 解析初始化 + query 构建）。

### IntentLoop 侧临时策略（已完成）

1. `zene` 依赖已改为 optional feature。
2. 默认构建不启用 `zene`，保证 `intentloop` 主流程可用。
3. 仅在 `cargo run --features zene -- zene ...` 时启用内嵌集成。

---

## Zene 模型配置位置

`zene` 的模型配置在 `AgentConfig::from_env()`，代码位置：
- `zene/src/config/mod.rs`

### 1) 全局默认（所有角色共用）

- `LLM_PROVIDER`（默认 `openai`）
- `LLM_MODEL`（默认 `gpt-4o`）
- `LLM_API_KEY`（或回退 `OPENAI_API_KEY`）
- `LLM_BASE_URL`（或回退 `OPENAI_BASE_URL`）
- `LLM_REGION`

### 2) 角色级覆盖（planner / executor / reflector）

- `ZENE_PLANNER_PROVIDER` / `ZENE_PLANNER_MODEL` / `ZENE_PLANNER_API_KEY` / `ZENE_PLANNER_BASE_URL` / `ZENE_PLANNER_REGION`
- `ZENE_EXECUTOR_PROVIDER` / `ZENE_EXECUTOR_MODEL` / `ZENE_EXECUTOR_API_KEY` / `ZENE_EXECUTOR_BASE_URL` / `ZENE_EXECUTOR_REGION`
- `ZENE_REFLECTOR_PROVIDER` / `ZENE_REFLECTOR_MODEL` / `ZENE_REFLECTOR_API_KEY` / `ZENE_REFLECTOR_BASE_URL` / `ZENE_REFLECTOR_REGION`

### 3) 其他相关配置

- `ZENE_SIMPLE_MODE`
- `ZENE_USE_SEMANTIC_MEMORY`
- `ZENE_XTRACE_ENDPOINT`
- `ZENE_XTRACE_TOKEN`

### 4) 文件配置（仅 MCP）

- `zene_config.toml` 目前用于加载 MCP servers，不用于 LLM 模型字段。

### 5) 在 IntentLoop 内嵌场景如何生效

IntentLoop 的 `zene` 子命令会调用 `ZeneConfig::from_env()`，因此模型配置同样通过环境变量传入当前进程。

注意：
- `zene` 独立 CLI 会 `dotenv().ok()` 自动读取 `.env`。
- IntentLoop 已支持自动加载 `.env`（启动时加载），因此内嵌 `zene` 场景可直接复用同一份环境变量配置。
