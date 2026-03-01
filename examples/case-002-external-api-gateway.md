# Case 002 - 外部仓库 API 网关全过程示例（无 gh）

## 目标

在不影响 IntentLoop 主仓库的前提下，完整展示一次“提交意图 -> 调用 agent -> 记录过程 -> 产生产物”的全流程。

## 运行环境

- IntentLoop 二进制：`/Users/mac-m4/github/IntentLoop/target/debug/intentloop`
- 外部仓库：`/tmp/intentloop-case-api-gateway`
- Agent 命令：`./fake-gateway-agent.sh`（模拟真实 agent 执行多步骤任务）

## 执行命令

```bash
cd /tmp/intentloop-case-api-gateway
/Users/mac-m4/github/IntentLoop/target/debug/intentloop run -- ./fake-gateway-agent.sh
```

Session ID：`019ca868-bfb9-7920-b26f-920eda10e03a`

## 过程记录了什么

- 输入意图：`INTENT.md`（id/title/背景/目标/约束）
- 执行命令：`./fake-gateway-agent.sh`
- 标准输出：每个步骤日志（inspect intent / create dirs / write config / write scripts / summary）
- 会话元数据：start/end/exit code/status
- 最终报告：`.intent/sessions/<id>/report.md`

## 本次关键输出（stdout 摘要）

- `[agent] step1: inspect intent`
- `[agent] step2: create directories`
- `[agent] step3: write gateway config`
- `[agent] step4: write startup and check scripts`
- `[agent] step5: write docs and test plan`
- `[agent] step6: delivery summary`
- `[agent] done`

## 最终产物（可直接查看）

```text
/tmp/intentloop-case-api-gateway/
  DELIVERY_SUMMARY.md
  gateway/config/gateway.yaml
  gateway/scripts/start.sh
  gateway/scripts/check.sh
  gateway/docs/README.md
  gateway/tests/smoke-plan.md
```

## 如何复查会话

```bash
/Users/mac-m4/github/IntentLoop/target/debug/intentloop show 019ca868-bfb9-7920-b26f-920eda10e03a
cat /tmp/intentloop-case-api-gateway/.intent/sessions/019ca868-bfb9-7920-b26f-920eda10e03a/terminal.raw.log
cat /tmp/intentloop-case-api-gateway/.intent/sessions/019ca868-bfb9-7920-b26f-920eda10e03a/report.md
```

## 结论

这个 case 展示的是“有工程动作”的完整过程，而不是只回显一句话：
- agent 根据意图完成多步骤任务
- 任务过程完整落日志
- 输出了结构化工程产物
- 全程发生在外部仓库，不污染 IntentLoop 代码库
