# Intent Result: case-gh-agent-task-001

**Intent Title:** 示例：为 IntentLoop 提供重构建议

---

## 重构建议清单

以下建议基于对仓库所有源文件的分析，聚焦于 `src/` 目录下的 Rust 模块：`builder.rs`、`deployer.rs`、`rollback.rs`、`syncer.rs`、`hook.rs`、`logger.rs`、`config.rs` 与 `main.rs`。

---

### 建议 1：提取公共命令执行工具函数

**Why（为什么）**

`builder.rs` 的 `build()` 函数和 `deployer.rs` 的 `deploy_with_command()` 函数各自包含几乎完全相同的逻辑：将命令字符串分割为程序名和参数、使用 `Command::new` 执行、检查退出状态、将 stderr 包装为错误返回。代码重复违反了 DRY 原则，任何对命令执行逻辑的修改（如增加超时、环境变量传递、stdout 捕获方式）都需要在两处同步维护，容易产生不一致。

**How（如何改动）**

- 新建 `src/utils.rs`，提供公共函数：
  ```rust
  pub fn run_command(command: &str, dir: &str) -> Result<String, Box<dyn std::error::Error>>
  ```
  该函数封装命令解析、`Command` 执行、错误格式化，并返回 stdout 字符串。
- `builder::build()` 和 `deployer::deploy_with_command()` 改为调用 `utils::run_command()`，删除各自的重复实现。
- 在 `lib.rs` 中 `pub mod utils;` 暴露该模块。

**改动范围：** `src/utils.rs`（新增）、`src/builder.rs`、`src/deployer.rs`、`src/lib.rs`

**Risk（风险）**

- 低风险。函数签名与现有逻辑等价，现有测试 `test_build_with_echo` 和 `test_deploy_with_echo` 可直接用于回归验证。
- 需注意 `build` 和 `deploy_with_command` 的日志前缀略有差异（`"Build failed"` vs `"Deployment failed"`），提取时需保留上下文相关的错误提示，可通过额外的 `context` 参数传入错误前缀实现。

---

### 建议 2：提取重复的符号链接管理逻辑

**Why（为什么）**

`deployer.rs` 的 `deploy_with_files()` 和 `rollback.rs` 的 `rollback_to_previous()` / `rollback_to_version()` 三处均包含完全相同的平台相关符号链接操作代码：删除旧 `current` 符号链接（Unix/Windows 分支），再创建新的 `current` 符号链接。这段代码共约 20 行，出现了 3 次，跨越两个模块，是明显的重复代码热点。一旦需要支持新平台或修改链接策略，必须在三处同步修改。

**How（如何改动）**

- 在 `src/utils.rs`（或单独的 `src/symlink.rs`）中新增：
  ```rust
  pub fn update_current_symlink(target_dir: &str, version_path: &str) -> Result<(), Box<dyn std::error::Error>>
  ```
  该函数封装"删除旧 current + 创建新 current"的完整平台分支逻辑。
- `deployer::deploy_with_files()`、`rollback::rollback_to_previous()`、`rollback::rollback_to_version()` 均改为调用该函数。

**改动范围：** `src/utils.rs`（或 `src/symlink.rs`，新增）、`src/deployer.rs`、`src/rollback.rs`

**Risk（风险）**

- 低风险。逻辑完全等价，仅是代码位置的迁移。
- 需保持 `#[cfg(unix)]` / `#[cfg(windows)]` 的条件编译标记，跨平台测试覆盖率较低时应增加平台特定的集成测试作为保障。

---

### 建议 3：拆分 `cmd_run` 中的单体流水线为结构化步骤

**Why（为什么）**

`main.rs` 中的 `cmd_run()` 函数约 100 行，将"加载配置 → 构建 → 制品验证 → 部署 → 回滚 → 清理 → 同步"
七个阶段全部内联在一个函数体内，混合了业务流程控制、用户输出和错误处理。这导致：

- 单函数职责过多，难以单独测试某一阶段；
- 新增流水线步骤必须改动同一函数，增加合并冲突风险；
- 业务逻辑与 CLI 呈现（`println!`）紧耦合，不利于未来提供非 CLI 入口（如 HTTP API 或 daemon 模式）。

**How（如何改动）**

- 定义 `PipelineContext` 结构体，持有 `Config`、`repo_path`、`commit_hash` 等执行期状态。
- 将各阶段提取为独立函数（可置于 `main.rs` 内或专用 `pipeline.rs` 模块）：
  `run_build_step(ctx)`、`run_deploy_step(ctx)`、`run_sync_step(ctx)` 等。
- `cmd_run()` 只负责初始化 `PipelineContext` 并按顺序调用各步骤函数，保持函数体在 20 行以内。
- 各步骤函数接收 `&PipelineContext` 并返回 `Result<(), ...>`，可独立进行单元测试。

**改动范围：** `src/main.rs`（重构）；可选新增 `src/pipeline.rs`

**Risk（风险）**

- 中等风险。这是行为等价的纯重构，但涉及 `main.rs` 的大范围结构调整，需要完整的端到端测试（`postloop run`）验证流水线各分支（构建失败触发回滚、同步失败不阻断部署结果等）仍然正确。
- 重构后 `cmd_rollback`、`cmd_status`、`cmd_log` 可按同样模式跟进，但不属于必要步骤。
