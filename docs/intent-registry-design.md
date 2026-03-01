# IntentLoop 重构设计文档（个人版 Intent Registry）

> 项目代号：`intentloop`
>
> Slogan：它能让你敢于让 AI 进行大规模重构，因为你知道无论 AI 把代码改得多么面目全非，你总能一键带回它最清醒的时刻，并看到它当时为什么“疯了”。

---

## 1. 项目初衷、背景与目标

### 1.1 背景问题

当前 AI 编码流程的核心矛盾：
- Git 擅长记录“结果（代码 diff）”，不擅长记录“过程（推理轨迹）”。
- 当 Agent 大规模修改代码后跑偏，开发者很难“按意图”回退，只能按文件回退。
- 个人开发场景最需要的是“低摩擦的可恢复性”，而不是企业级复杂治理。

### 1.2 重构目标

将当前项目从 post-commit 自动部署工具，演进为**个人开发者的 Agent 会话记录与回退工具**：

1. **Wrapper First**：包装现有 AI CLI（Claude Code / Codex / Cursor CLI），不重造模型。
2. **Intent First**：`INTENT.md` 作为入口，驱动会话生命周期。
3. **Session as Unit**：以“意图会话”作为版本单位，代码只是副产物。
4. **Fast Rewind**：支持按会话一键回退（`rewind`）。
5. **Minimal Git Coupling**：仅通过 Git hooks 做弱集成，不重构 Git 工作流。
6. **Simple Local Storage**：元数据进 SQLite，大对象与日志走文件系统。

### 1.3 成功标准（MVP）

- 可以运行 `intentloop run -- claude` 并完整捕获一次会话。
- 可以从 `INTENT.md` 自动关联当前意图。
- 可以记录：会话元数据、终端流、关键动作、文件快照。
- 可以执行 `intentloop rewind --session <id>` 恢复工作区。
- 可以 `intentloop show <id>` 输出 Markdown 轨迹报告。

### 1.4 非目标（MVP 不做）

- 多人协作冲突解决。
- 分布式存储与云同步。
- 复杂策略引擎（Policy-as-Code）与企业合规。
- 存储膨胀优化（仅做基础压缩，不做高级去重）。

---

## 2. 系统架构与技术栈

### 2.1 总体架构

```text
Developer
  ├─ edits INTENT.md
  └─ runs intentloop run -- <agent-cli>

intentloop (Rust CLI)
  ├─ Intent Resolver      -> 解析 INTENT.md
  ├─ Session Manager      -> 创建/关闭会话
  ├─ PTY Wrapper          -> 运行并捕获 Agent CLI I/O
  ├─ Artifact Tracker     -> 检测文件改动、生成 diff/snapshot
  ├─ Git Hook Bridge      -> 可选注入 commit footer
  ├─ Rewind Engine        -> 按会话回退文件状态
  ├─ Markdown Reporter    -> 生成 history/*.md
  └─ Registry Store
       ├─ SQLite (metadata/index)
       └─ .intent/objects (snapshots/logs)
```

### 2.2 分层设计

- **Interface Layer**：CLI 命令入口与交互输出。
- **Application Layer**：会话编排、回退流程、报告生成。
- **Domain Layer**：实体与规则（Intent、Session、Artifact、ThoughtEvent）。
- **Infrastructure Layer**：PTY、SQLite、文件系统、Git hooks、压缩。

### 2.3 推荐技术栈（Rust）

- CLI：`clap`
- 异步运行时：`tokio`
- PTY：`portable-pty`（跨平台较稳）
- SQLite：`rusqlite`（MVP优先），后续可切 `sqlx`
- 文件监控：`notify`
- Hash：`blake3`
- 压缩：`zstd`
- 时间/序列化：`chrono` + `serde` + `serde_json`
- Markdown 处理：`pulldown-cmark`（解析）
- 日志：`tracing` + `tracing-subscriber`

### 2.4 可插拔检索层（Search Abstraction）

目标：
- MVP 不强依赖向量数据库；默认使用 SQLite。
- 通过统一接口隔离“关键词检索”和“语义检索”实现。
- 后续可无缝接入 LanceDB，不影响上层命令与数据模型。

抽象接口（建议）：

```rust
pub trait SearchRepository {
    fn index_session_text(&self, session_id: &str, chunks: Vec<TextChunk>) -> anyhow::Result<()>;
    fn keyword_search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>>;
    fn semantic_search(
        &self,
        query: &str,
        embedding: Option<Vec<f32>>,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchHit>>;
}

pub struct TextChunk {
    pub id: String,
    pub session_id: String,
    pub source: String,   // intent|thought|artifact|report
    pub content: String,
    pub ts: String,
}

pub struct SearchHit {
    pub chunk_id: String,
    pub session_id: String,
    pub score: f32,
    pub snippet: String,
}
```

实现策略：
- `SqliteSearchRepository`（默认）
  - `keyword_search`: SQLite FTS5。
  - `semantic_search`: MVP 返回“未启用语义检索”或退化为关键词检索。
- `LanceDbSearchRepository`（P1/P2 可选）
  - 存储 chunk embedding 与 metadata。
  - 支持 ANN 向量检索和混合检索（vector + keyword）。

配置开关（`intentloop.toml`）：

```toml
[search]
backend = "sqlite" # sqlite | lancedb
enable_semantic = false
embedding_provider = "none" # none | openai | ollama | custom
top_k = 20
```

兼容性要求：
- CLI 层只调用 `SearchRepository`，禁止直接依赖具体后端。
- 切换后端不改变 `show/list/status` 的输出协议。
- 检索失败不影响会话记录与回退主链路（降级策略）。

---

## 3. 数据库设计与文档系统设计

### 3.1 本地目录布局

```text
.intent/
  db.sqlite
  sessions/
    <session_id>/
      terminal.raw.log
      terminal.normalized.jsonl
      report.md
  snapshots/
    <snapshot_id>.tar.zst
  patches/
    <artifact_id>.patch
  hooks/
    prepare-commit-msg
  state/
    active_session.json
```

### 3.2 SQLite Schema（MVP）

```sql
PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS intents (
  id TEXT PRIMARY KEY,                -- from INTENT.md frontmatter id
  title TEXT NOT NULL,
  body_markdown TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,                -- uuid v7
  intent_id TEXT,
  agent_name TEXT NOT NULL,           -- claude/codex/cursor/unknown
  agent_cmd TEXT NOT NULL,
  cwd TEXT NOT NULL,
  git_head_before TEXT,
  git_head_after TEXT,
  start_at TEXT NOT NULL,
  end_at TEXT,
  status TEXT NOT NULL,               -- running/succeeded/failed/interrupted
  exit_code INTEGER,
  summary TEXT,
  FOREIGN KEY(intent_id) REFERENCES intents(id)
);

CREATE TABLE IF NOT EXISTS thought_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  ts TEXT NOT NULL,
  event_type TEXT NOT NULL,           -- stdout/stderr/tool_call/system
  content TEXT NOT NULL,
  signature TEXT,                     -- reserved for future
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS artifacts (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  path TEXT NOT NULL,
  change_type TEXT NOT NULL,          -- added/modified/deleted/renamed
  hash_before TEXT,
  hash_after TEXT,
  patch_path TEXT,
  snapshot_before_id TEXT,
  snapshot_after_id TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS commands (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  ts TEXT NOT NULL,
  command_text TEXT NOT NULL,
  exit_code INTEGER,
  stdout_tail TEXT,
  stderr_tail TEXT,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS git_links (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  commit_hash TEXT NOT NULL,
  branch TEXT,
  linked_at TEXT NOT NULL,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE INDEX IF NOT EXISTS idx_sessions_intent ON sessions(intent_id);
CREATE INDEX IF NOT EXISTS idx_thought_session_seq ON thought_events(session_id, seq);
CREATE INDEX IF NOT EXISTS idx_artifacts_session ON artifacts(session_id);
```

### 3.2 检索索引表（SQLite 默认后端）

用于关键词检索与后续混合检索的基础索引：

```sql
CREATE TABLE IF NOT EXISTS search_chunks (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  source TEXT NOT NULL,               -- intent|thought|artifact|report
  content TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS search_chunks_fts
USING fts5(id, content, tokenize='unicode61');

CREATE INDEX IF NOT EXISTS idx_search_chunks_session ON search_chunks(session_id);
```

写入约定：
- 会话结束时将 `INTENT.md`、`thought_events` 摘要、`report.md` 分块写入 `search_chunks`。
- 同步写入 `search_chunks_fts`，保证 `list/search` 秒级响应。
- 若启用 LanceDB，可继续保留 FTS 作为本地兜底。

### 3.3 `INTENT.md` 规范

MVP 固定文件名：仓库根目录 `INTENT.md`。

推荐格式：

```md
---
id: auth-jwt-001
title: 登录模块改造为 JWT
status: active
focus:
  - src/auth/
  - src/middleware/
constraints:
  - 保持现有 API 路径不变
  - 不引入额外数据库
---

## 背景
...

## 目标
...

## 验收标准
...
```

解析规则：
- frontmatter 缺失时：自动生成 `intent-{date}-{short_hash}`。
- `id` 必须稳定；同 `id` 允许多次会话。
- 每次 `run` 都在会话开始时快照 `INTENT.md` 原文存入 `intents.body_markdown`。

### 3.4 Markdown 历史报告

每个会话输出 `.intent/sessions/<session_id>/report.md`，结构固定：

1. Header（session id / intent id / 时间 / agent）
2. Intent Snapshot（当时的 INTENT）
3. Timeline（关键事件）
4. File Changes（语义摘要 + patch 引用）
5. Rejected/Failed Attempts（从日志中提取）
6. Rewind Hints（建议回退点）

---

## 4. 系统模块设计

### 4.1 CLI 模块

命令集（MVP）：

```bash
intentloop init
intentloop run -- <agent-cli> [args...]
intentloop status
intentloop show <session-id>
intentloop list [--intent <id>] [--limit 20]
intentloop rewind [--session <id> | --last]
intentloop hook install
intentloop hook uninstall
```

约束：
- `run` 必须透传参数，不篡改用户输入。
- `rewind` 默认只恢复当前工作区可追踪文件，不主动 `git reset --hard`。

### 4.2 Session Manager

职责：
- 创建会话记录（`running`）。
- 记录起止时间与状态迁移。
- 保存 `git_head_before/after`。

状态机：
- `running -> succeeded`
- `running -> failed`
- `running -> interrupted`

### 4.3 PTY Wrapper

职责：
- 启动 Agent CLI 子进程。
- 实时捕获 stdout/stderr。
- 将原始流写入 `terminal.raw.log`。
- 归一化为 `thought_events`（jsonl + sqlite）。

MVP 规则：
- 不假设模型输出格式；先按“时间片 + 流类型”持久化。
- 可选对已知标记（如 `tool:`、`exec:`）做轻量提取。

### 4.4 Artifact Tracker

职责：
- 会话开始时对工作区做快照（baseline）。
- 会话结束后扫描变化文件。
- 生成 patch 与 hash before/after。

实现策略（MVP）：
1. baseline：遍历工作区（排除 `.git`, `.intent`, `target`）。
2. compare：按路径与 blake3 进行差异计算。
3. patch：优先调用 `git diff --no-index`（若可用），否则仅保存 before/after。

### 4.5 Snapshot & Rewind Engine

职责：
- 快照归档到 `.intent/snapshots/*.tar.zst`。
- 支持按 session 回退。

回退算法（MVP）：
1. 加载目标 session 的 baseline 清单。
2. 对“该会话影响的文件集合”执行恢复。
3. 若当前文件在目标会话后被人工修改，默认提示冲突并支持 `--force`。

### 4.6 Git Hook Bridge（弱集成）

只实现 `prepare-commit-msg`：
- 若存在 active session，向提交信息附加 footer：
  - `Intent-Session: <id>`
  - `Intent-ID: <intent_id>`
- 若用户禁用：`intentloop hook uninstall`。

不接管：
- 不自动 commit。
- 不改分支策略。
- 不阻断 commit。

### 4.7 Reporter

- 从 SQLite + 文件系统拼装会话报告。
- `show <id>` 走 pager 输出。
- `export --format md/json`（P1 功能，可预留）。

### 4.8 Search 模块（新增）

职责：
- 接收 `search` 命令请求，路由到 `SearchRepository`。
- 负责 chunk 切分、索引构建和查询结果重排。

建议命令：

```bash
intentloop search "jwt refresh token"
intentloop search "为什么当时放弃方案B" --intent auth-jwt-001 --limit 10
```

分层边界：
- `application/search_service.rs`：查询编排、fallback 策略。
- `infra/search_sqlite.rs`：FTS 实现。
- `infra/search_lancedb.rs`：向量检索实现（可后置）。
- `domain/search.rs`：`SearchRepository` trait 与实体。

---

## 5. 测试策略（可直接执行）

### 5.1 测试分层

1. **单元测试（Rust `#[test]`）**
   - `intent_parser`：frontmatter 解析与默认值。
   - `hash_index`：文件哈希与变更识别。
   - `session_state`：状态迁移合法性。
   - `rewind_planner`：恢复计划生成。

2. **集成测试（`tests/`）**
   - `run_with_fake_agent`：用 mock CLI 产生日志与文件修改，验证会话完整落库。
   - `rewind_last_session`：执行 run -> 修改 -> rewind，验证文件恢复。
   - `hook_footer_injection`：验证 commit message footer 注入。

3. **端到端测试（脚本）**
   - 新建临时 git repo，运行完整流程：`init -> run -> commit -> rewind`。

### 5.2 关键测试用例

- **Case A: 基础会话捕获**
  - 输入：`INTENT.md` + fake agent 输出。
  - 断言：`sessions=1`，`thought_events>0`，`artifacts` 匹配修改文件。

- **Case B: Agent 失败场景**
  - fake agent 返回非 0。
  - 断言：session=`failed`，仍有日志和局部 artifact 记录。

- **Case C: Rewind 安全恢复**
  - 会话修改 `a.rs`，之后人工再改 `a.rs`。
  - 断言：默认提示冲突，不覆盖；`--force` 可覆盖。

- **Case D: Git 弱集成**
  - 安装 hook 后 commit。
  - 断言：提交信息包含 footer，不影响 commit exit code。

### 5.3 质量门禁

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- 覆盖率目标（MVP）：核心模块行覆盖 >= 70%

---

## 6. 迭代路线图（AI 可直接按阶段开发）

### Milestone 1（MVP-0，1~2 周）

目标：跑通最短闭环。
- 命令：`init/run/list/show`。
- SQLite 初始化 + sessions/thought_events。
- PTY 包装与日志持久化。
- 基础 Markdown 报告。

交付验收：
- 能完整记录一次会话并查看报告。

### Milestone 1-Lite（建议首发范围，3~5 天）

如果第一阶段还要进一步精简，建议将范围压缩为“单次会话可追溯”。

必须保留（Only Must-Have）：
- 命令仅保留：`run`、`show`。
- `INTENT.md` 读取（仅取 `id/title`，无 frontmatter 复杂校验）。
- SQLite 仅建两张表：`sessions`、`thought_events`。
- PTY 透传执行 + 原始终端日志落盘。
- 会话结束时生成最小 `report.md`（Header + 原始日志引用）。

第一阶段可砍掉（Move to Next Phase）：
- `list`、`status`、`search` 命令。
- `artifacts` 表与文件级 hash/diff 跟踪。
- snapshot 压缩与 `rewind`（整体延期到 Milestone 2）。
- Git hooks 与 commit footer 注入。
- LanceDB 及任何 embedding 相关配置。

技术上再降一档（可选）：
- 不上 `tokio`，先用同步进程模型，减少异步复杂度。
- `terminal.normalized.jsonl` 先不做，仅保留 `terminal.raw.log`。
- 配置文件 `intentloop.toml` 先不引入，使用硬编码默认路径。

Lite 验收标准：
- `intentloop run -- <agent-cli>` 可以稳定执行并创建一条 session。
- `intentloop show <session-id>` 可以查看会话基本信息和日志位置。
- Agent 非 0 退出码场景下，仍会保留完整会话记录。

### Milestone 2（MVP-1，1~2 周）

目标：可恢复。
- artifact tracker（hash + patch）。
- snapshot 存储。
- `rewind --last`。
- `status`。

交付验收：
- AI 跑偏后 1 命令恢复到会话前状态。

### Milestone 3（MVP-2，1 周）

目标：与 Git 轻联动。
- hook install/uninstall。
- commit footer 注入。
- list 过滤与 show 优化。

交付验收：
- commit 能自动关联 session id，且工作流零侵入。

### Milestone 4（P1，可选）

目标：增强检索能力（不影响主链路稳定性）。
- 增加 `search` 命令与 SQLite FTS5。
- 引入 `SearchRepository` 抽象和后端切换配置。
- 可选接入 LanceDB（当语义检索成为核心需求时开启）。

交付验收：
- 默认 SQLite 可用且稳定。
- 切换 LanceDB 不改上层命令协议。
- 后端故障时可自动降级到 SQLite 关键词检索。

---

## 7. 与现有仓库的重构映射

当前代码可复用：
- `src/config.rs`：保留“配置加载”模式，改为 `intentloop.toml`。
- `src/hook.rs`：复用 hook 安装能力，改为 `prepare-commit-msg`。
- `src/logger.rs`：可升级为 `tracing` 但保留文件日志思路。
- `src/main.rs`：沿用 clap 框架，替换命令域。

建议废弃或降级：
- `builder.rs`、`deployer.rs`、`syncer.rs`、`rollback.rs`（部署语义与新产品无关）。

建议新增模块：
- `intent.rs`（INTENT.md 解析）
- `session.rs`（生命周期）
- `pty.rs`（包装执行）
- `registry.rs`（SQLite 访问）
- `artifact.rs`（文件变化追踪）
- `snapshot.rs`（快照压缩与恢复）
- `rewind.rs`（恢复编排）
- `report.rs`（Markdown 生成）

---

## 8. 风险与应对

1. **PTY 兼容性风险**
   - 应对：先支持 macOS/Linux；Windows 延后。

2. **Agent 输出结构不稳定**
   - 应对：MVP 不做强语义解析，先保真记录原始流。

3. **回退覆盖误伤人工修改**
   - 应对：默认保护 + 冲突提示 + `--force`。

4. **快照体积增长快**
   - 应对：先接受增长；后续再做 TTL 与增量去重。

---

## 9. AI 开发启动清单（可直接执行）

### 9.1 首批任务（按优先级）

1. 重命名命令与产品文案（`postloop -> intentloop`）。
2. 新建 `registry` 模块并实现 SQLite 初始化迁移。
3. 新建 `intent` 模块并解析 `INTENT.md` frontmatter。
4. 实现 `run -- <cmd>`：启动子进程并记录会话。
5. 写入 `show/list/status` 基础查询命令。
6. 实现 artifact 基线扫描 + 差异记录。
7. 实现 `rewind --last`。
8. 实现 hook 注入 commit footer。

### 9.2 Definition of Done

- 所有核心命令有帮助文档与示例。
- 核心流程均有集成测试。
- 出错信息可读，且不会破坏用户仓库。
- `.intent/` 可被安全删除（不影响源码仓库健康）。

---

## 10. 产品宣言（对外）

IntentLoop 不是另一个“更快写代码”的工具。

它是你的 Agent 开发黑匣子：
- 记录意图，
- 记录推理，
- 记录副作用，
- 并且在一切失控时，
- 带你回到最清醒的那一刻。
