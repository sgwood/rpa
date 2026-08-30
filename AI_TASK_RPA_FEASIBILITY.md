# AI 工具任务完成汇总与通知 RPA 可行性分析

日期：2026-08-29

## 一、结论

项目可行，建议立项，综合可行性为 **高（8.5/10）**。

正确形态不是传统的“识别窗口、模拟点击”RPA，而是一个跨设备 Agent 控制面：在每台 macOS 或 Windows 设备上运行本机节点，优先接收 Codex、Claude、Cursor、Antigravity IDE 的生命周期 Hook、JSONL、SDK 或 Webhook，统一判断任务状态、汇总通知，并把后续任务投递回原会话。只有没有事件接口的场景才降级到 UI 自动化。

建议先用观察模式验证事件，再启用双向控制：

1. 接入 Codex、Claude、Cursor、Antigravity 四个工具；
2. 统一输出“运行中、等待人工、成功、失败、取消、状态不明”；
3. 失败和等待人工立即通知，成功任务按 15～30 分钟合并通知；
4. 对已登记会话支持“立即继续、当前回合结束后继续、恢复历史会话后继续”；
5. 本机通知作为默认通道，飞书作为推荐的远程通道；
6. 保留事件证据和原任务入口，禁止只凭 AI 文本声称成功。

技术可行性需要按能力分层：

- **状态采集：高（9.5/10）**。四个工具的 IDE/CLI 都有正式 Hook 或事件流；
- **回合结束时立即接续已排队任务：高（9/10）**。四个工具的 Stop Hook 都能让执行循环继续；
- **恢复由 RPA 管理的历史会话：高（9/10）**。四个工具均能保存稳定会话 ID；
- **任务结束后再唤醒任意手工创建的 GUI 会话：中（6/10）**。Cursor 本机 IDE 没有公开的任意会话唤醒 API；Antigravity IDE 更适合导入 CLI 或通过 Remote Control 继续；
- **向正在执行的任意 GUI 会话实时插话：中（6.5/10）**。Codex 有 `turn/steer`，Claude 有 Channels/跨会话消息；Cursor 本机 IDE 和 Antigravity IDE 更适合把新任务排队到当前回合结束后执行。

为了达到稳定的双向控制，任务需要分成两种运行模式：

| 模式 | 范围 | 状态采集 | 后续任务投递 |
|---|---|---:|---:|
| `OBSERVED` | 用户手工在任意 IDE 中创建的会话 | 高 | 回合结束前已排队的任务高；结束后重新唤醒因工具而异 |
| `MANAGED` | 由 RPA 通过 App Server、CLI、SDK 或 Cloud API 创建/登记 | 高 | 高，可恢复、继续、排队并审计 |

需要无人值守连续工作的任务应使用 `MANAGED`；`OBSERVED` 用于覆盖用户日常手工会话，但不承诺所有 IDE 都能在退出或结束后被无损唤醒。

## 二、当前环境核验

本机已发现：

| 工具 | 桌面端 | CLI | 当前完成事件配置 |
|---|---:|---:|---|
| ChatGPT / Codex | 已安装并运行 | 已安装 | 未发现 `~/.codex/hooks.json` |
| Claude | 已安装并运行 | Claude Code 2.1.177 | 只有 `UserPromptSubmit`，没有完成 Hook |
| Cursor | 已安装，版本 3.17.21 | 未在 PATH 中发现 | 未发现 `~/.cursor/hooks.json` |
| Antigravity IDE | 已安装并运行，版本 1.107.0 | `agy` 未在 PATH 中发现 | 尚未配置 |

Codex 当前任务状态可以被读取。核验时，除本分析任务外还有两个任务明确处于 `inProgress`：

- “开发飞书工作计划 Codex 插件”；
- “搜索 ManageBAC 操作手册”。

近期列表中还存在 `idle` 和 `notLoaded` 状态。验证表明 `idle` 任务可以拥有一个 `completed` 的最近回合，但 **`idle` 本身不能作为业务任务成功的证据**；`notLoaded` 更不能解释为完成。

以上是一次性快照，不是持续监控结果，也不覆盖所有历史任务。

## 三、各工具接入可行性

| 工具/场景 | 推荐接入方式 | 可行性 | 说明 |
|---|---|---:|---|
| Codex 交互任务 | `Stop`、`SessionEnd` Hook | 高 | 能捕获回合停止和会话结束，但仍需根据最终结果判断成功、失败或等待人工 |
| Codex 脚本任务 | `codex exec --json` | 高 | JSONL 包含 `turn.completed`、`turn.failed`、`error`，最适合稳定自动化 |
| Codex 定时巡检 | ChatGPT/Codex Scheduled task | 高 | 可定时汇总，并在 Scheduled 中形成收件箱；访问本地项目时电脑需开机且桌面应用运行 |
| Claude Code | `TaskCompleted`、`Stop`、`StopFailure`、`SessionEnd` Hook | 高 | 支持命令、HTTP、MCP 等 Handler，事件覆盖较完整 |
| Cursor 本机 Agent | `stop`、`sessionEnd`、`afterAgentResponse` Hook | 高 | 用户级或项目级脚本均可接入 |
| Cursor Cloud Agent | v1 API、SSE/轮询 | 高 | 可读取 Run 状态并向原 Agent 创建后续 Run；执行中重复投递会返回 `409 agent_busy`；v1 Webhook 尚未开放 |
| Antigravity IDE / CLI | `Stop` Hook、CLI `stream-json`、SDK Lifecycle | 高 | Hook 带 `conversationId`、`terminationReason`、`fullyIdle`；CLI 和 SDK 可恢复会话 |
| 只有网页/封闭客户端的 AI 工具 | 浏览器或辅助功能 UI RPA | 中低 | 页面结构、登录、弹窗和文案变化会导致失效，只应作为兜底 |

官方依据：

- [OpenAI Codex Hooks](https://learn.chatgpt.com/docs/hooks)
- [OpenAI Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [OpenAI Codex SDK](https://learn.chatgpt.com/docs/codex-sdk)
- [OpenAI Codex 非交互模式](https://learn.chatgpt.com/docs/non-interactive-mode)
- [OpenAI Windows App](https://learn.chatgpt.com/docs/windows/windows-app)
- [OpenAI Scheduled tasks](https://learn.chatgpt.com/docs/automations)
- [Claude Code Hooks](https://code.claude.com/docs/en/hooks)
- [Claude Agent SDK 会话](https://code.claude.com/docs/en/agent-sdk/sessions)
- [Claude 跨会话消息](https://code.claude.com/docs/en/cross-session-messaging)
- [Claude Channels](https://code.claude.com/docs/en/channels)
- [Cursor Hooks](https://cursor.com/docs/hooks)
- [Cursor Cloud Agents API](https://cursor.com/docs/cloud-agent/api/endpoints)
- Cursor Cloud v1 当前为 Public Beta，官方端点文档标注 v1 Webhook 尚未开放；旧版 v0 Webhook 只能作为独立兼容能力
- [Antigravity Hooks](https://antigravity.google/docs/hooks)
- [Antigravity CLI Headless](https://antigravity.google/docs/cli/headless)
- [Antigravity SDK Lifecycle](https://antigravity.google/docs/sdk/lifecycle)
- [Antigravity Remote Control](https://antigravity.google/docs/remote-control)

## 四、不能把“停止”直接当成“完成”

RPA 必须统一使用下面的状态模型：

| 状态 | 判定要求 | 是否通知 |
|---|---|---|
| `RUNNING` | 有开始事件，尚无终止事件 | 通常不通知 |
| `WAITING_USER` | 明确需要授权、选择、凭据或人工操作 | 立即通知 |
| `SUCCEEDED` | 有终止事件，且最终结构化结果明确成功，并附验证证据 | 合并通知 |
| `FAILED` | 失败事件、非零退出、异常，或明确验证失败 | 立即通知 |
| `CANCELLED` | 用户取消、进程中断或会话被终止 | 立即或合并通知 |
| `UNKNOWN` | 只有空闲、窗口关闭、进程消失、超时或无法解析的文本 | 到期提醒，不报成功 |

推荐让所有可控任务最终输出统一 JSON：

```json
{
  "outcome": "succeeded | failed | waiting_user | cancelled",
  "summary": "完成了什么",
  "evidence": ["测试、产物、链接、提交或其他可核验证据"],
  "next_action": "没有则为空",
  "confidence": "high | medium | low"
}
```

Hook 负责告诉系统“回合结束”，结构化结果和证据负责告诉系统“任务是否真的成功”。

## 五、推荐架构

```text
Codex Hooks / JSONL ─┐
Claude Hooks ────────┼─> 本机 Collector ─> SQLite 事件库 ─> 状态规则 ─> 汇总器 ─> 通知器
Cursor Hooks/API ─────┤         │                 │                    ├─ macOS / Windows
Antigravity Hooks ────┘         │                 └─ 去重/重试/超时     ├─ 飞书
                                │                                      └─ Codex Scheduled
                                └<── Command Queue <── 中央控制台/通知卡片
```

建议的最小技术实现：

- 本机节点：推荐 Go 单文件程序，macOS 和 Windows 共用一套业务代码；
- Collector：仅监听 `127.0.0.1` 的本机 HTTP 服务，同时支持从 stdin 接收 Hook JSON；
- 存储：SQLite，保存任务、回合、事件、通知和最后确认状态；
- 任务主键：`source + task_id + turn_id`，防止不同工具 ID 冲突；
- 去重键：`source + task_id + turn_id + event_type`；
- 汇总：先执行确定性状态规则，再让模型生成面向人的中文摘要；
- 运行方式：macOS 使用当前登录用户的 LaunchAgent；Windows 使用登录时启动的每用户后台程序或托盘程序；
- 远程连接：本机节点主动建立出站 TLS WebSocket，不在电脑上开放公网入站端口；
- 安全：默认不保存完整 Prompt、密钥、命令输出和个人数据，只保存必要摘要和证据引用。

### 双向控制适配矩阵

| 工具 | 获取执行情况 | 回合结束后继续 | 恢复历史会话 | 执行中实时追加 | macOS / Windows |
|---|---|---|---|---|---|
| Codex | Hook、App Server 事件、`exec --json` | `Stop` 返回 `decision: block` | SDK/App Server `thread/resume` + `turn/start` | App Server `turn/steer` | 原生支持 |
| Claude | Hook、Agent SDK 流、结果消息 | `Stop` 返回 `decision: block` | Agent SDK/CLI `resume` | Channels（预览）或同机跨会话消息 | 原生支持；Windows 也支持命名管道 |
| Cursor 本机 | Hook 的 `conversation_id`、`generation_id`、`status` | `stop` 返回 `followup_message` | RPA 管理的 CLI 会话可用 `--resume <chatId>`；未确认能直接恢复任意 IDE Chat | 无同等级公开本机 Steer API，建议排队 | 原生支持 |
| Cursor Cloud | API/SSE 的 Run 状态 | `POST /v1/agents/{id}/runs` | Durable Agent 保留会话和工作区 | 运行中返回 `409 agent_busy`，排队后重试 | 与控制端 OS 无关 |
| Antigravity | Hook、CLI NDJSON、SDK Hook | `Stop` 返回 `decision: continue` | CLI/SDK 会话使用 `conversation_id`；IDE 会话可导入 CLI 后克隆上下文继续 | RPA 管理的常驻 CLI 可在上一轮 `result` 后写入 stdin；IDE 建议排队 | 原生支持 |

因此，“继续发任务”可以统一为三个命令语义：

- `SEND_NEXT`：当前回合结束时自动作为下一条指令执行；四个工具全部支持，但命令必须在 Stop Hook 返回前进入队列；
- `RESUME_AND_SEND`：恢复已结束的指定会话并发新任务；四个工具的 `MANAGED` 会话全部支持；
- `STEER_ACTIVE`：向正在运行的回合追加信息；Codex 正式支持，Claude 可选预览通道，Cursor/Antigravity 默认降级为 `SEND_NEXT`。

这里的 `RESUME_AND_SEND` 对 Cursor 本机只承诺 RPA 管理的 CLI 会话。对于已经结束、且完全由用户在 Cursor IDE 手工创建的 Chat，第一版只提供“打开原会话并预填任务”或 UI 自动化兜底，不把它列为稳定接口能力。Cursor Cloud Agent 不存在这个限制。

### Hook 投递协议

所有 Hook 都调用同一个跨平台命令：

```text
ai-rpa hook --source codex|claude|cursor|antigravity
```

命令从 stdin 读取厂商 JSON，向本机 Collector 上报事件，并查询该会话是否存在待投递命令。若存在，按厂商协议返回：

| 工具 | Hook 返回值 |
|---|---|
| Codex | `{"decision":"block","reason":"<下一任务>"}` |
| Claude | `{"decision":"block","reason":"<下一任务>"}` |
| Cursor | `{"followup_message":"<下一任务>"}` |
| Antigravity | `{"decision":"continue","reason":"<下一任务>"}` |

Hook 查询必须在 1～3 秒内结束。Collector 不可用时返回空结果，让 AI 工具正常停止；监控系统故障不能阻断开发工作。

### 跨平台运行边界

- Hook 配置放在各工具的用户级目录，Windows 使用 `%USERPROFILE%` 对应目录；安装器必须合并现有配置，不能覆盖用户已有 Hook；
- Windows 后台节点应运行在当前登录用户会话中。Windows Service 的 Session 0 不适合直接控制用户打开的 IDE；
- WSL 与 Windows Native 视为两台逻辑设备。Codex、Claude 等在 WSL 和 Windows Native 的会话目录、Socket/Named Pipe、认证缓存默认不共享；
- macOS 睡眠或退出登录后本机 IDE 任务不可继续。Windows 每用户节点退出登录后也不能控制 GUI IDE；真正无人值守任务应通过 CLI、SDK 或云 Agent 运行；
- 设备间只同步状态、命令和必要摘要，不直接同步整个厂商会话数据库。

### 核心数据模型

```text
Device(id, os, hostname, online_at)
AgentSession(id, source, vendor_session_id, device_id, workspace, status, control_mode)
AgentTurn(id, session_id, vendor_turn_id, status, started_at, ended_at, evidence)
Event(idempotency_key, session_id, turn_id, type, payload_hash, occurred_at)
Command(id, session_id, mode, prompt_ciphertext, state, expires_at, delivered_at)
Notification(id, event_id, channel, state, sent_at)
```

`Command` 必须经过会话级租约控制：同一会话同一时刻只能有一个 Controller 投递任务；命令以 UUID 保证幂等，失败重试不能重复执行。

`control_mode` 必须明确记录 `OBSERVED` 或 `MANAGED`。控制台不能在 `OBSERVED` 会话上展示一个实际上无法保证执行的“继续任务”成功状态；应显示“已排队等待 Stop”“需要打开原会话”或“已转入托管 CLI/Cloud 会话”。

## 六、通知策略

推荐默认策略：

- `FAILED`、`WAITING_USER`：立即通知；
- `SUCCEEDED`：15～30 分钟窗口内合并，减少通知轰炸；
- `UNKNOWN`：超过设定时限后提醒“状态不明”，不宣称失败或成功；
- 同一任务状态不变时不重复通知；
- 通知必须包含工具、任务标题、结论、证据摘要、耗时、下一步和可打开的任务入口。

本机通知实现最省事，但离开电脑后不可见。若需要手机收到，推荐接入飞书机器人或飞书应用；Webhook 密钥应放入系统钥匙串或受限环境变量，不能写入仓库或 Hook 日志。

## 七、实施投入

| 阶段 | 范围 | 预计投入（单人） | 产出 |
|---|---|---:|---|
| POC | macOS 四类 Hook 入库、排队继续、控制台汇总 | 1～2 天 | 证明状态采集和 `SEND_NEXT` |
| MVP | macOS/Windows 节点、状态机、SQLite、命令队列、飞书通知 | 5～8 天 | 双平台可日常使用版本 |
| 稳定版 | 中央控制台、设备长连接、签名校验、升级、健康检查、测试与审计 | 2～4 周 | 多设备长期运行版本 |

## 八、主要风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| 把 `Stop`、`idle` 当成功 | 高 | 终止事件与结构化结果、验证证据同时满足才判成功 |
| Hook 重复或乱序 | 中 | 幂等键、事件时间与状态迁移约束 |
| 电脑休眠或应用退出 | 中 | 本地队列补发；云任务使用可公网访问的 HTTPS Receiver |
| UI 自动化随版本失效 | 高 | 原生 Hook/API 优先；UI 兜底必须有健康检测 |
| 同一会话被 IDE 和 RPA 同时恢复 | 高 | 会话租约；只有 `IDLE` 且持有控制权时允许 `RESUME_AND_SEND` |
| Hook 自动继续形成死循环 | 高 | 每命令只投递一次，设置续跑上限、TTL 和全局熔断 |
| Windows Service 无法访问用户 IDE | 高 | 使用每用户后台进程；Service 只承载中央同步，不直接做 GUI 控制 |
| Prompt、日志或密钥泄露 | 高 | 最小采集、字段脱敏、钥匙串、日志轮转和访问权限 |
| 通知过多 | 中 | 即时告警与批量成功汇总分流 |
| AI 最终文本夸大完成度 | 高 | 由确定性事件和测试/产物证据判定，模型只负责摘要 |

## 九、验收标准

MVP 达到以下条件才算可用：

1. macOS 和 Windows 上，Codex、Claude、Cursor、Antigravity 各完成成功、失败、等待人工、取消、继续任务测试；
2. 事件不漏报，重复通知率低于 1%；
3. 没有把 `idle`、窗口关闭或 `Stop` 单独判成成功；
4. 本机离线后恢复，未发送通知可以补发且不重复；
5. 通知能一键回到原任务或明确给出任务 ID；
6. 日志中不出现令牌、API Key、完整 Prompt 或非必要个人数据；
7. Collector 关闭时不能阻断四个 AI 工具的正常工作，Hook 应快速返回并采用 fail-open；
8. 连续投递 100 条带 UUID 的命令，不发生重复执行；
9. Cursor/Antigravity 在执行中收到新任务时进入队列，并在当前回合结束后只执行一次；
10. macOS、Windows Native、WSL 的同名会话不会被错误合并。

## 十、建议决策

**建议 Go。** 首期做 Codex、Claude、Cursor、Antigravity 的原生事件和 Hook 双向接入，不做跨产品的纯 UI 自动化，不直接修改各产品内部数据库。

推荐默认落地组合是：

- 四工具 Hook + macOS/Windows 本机节点 + SQLite；
- 第一阶段只观察，第二阶段启用 `SEND_NEXT`，第三阶段再启用 `RESUME_AND_SEND`；
- 失败/等待人工立即飞书通知；
- 成功任务每 30 分钟汇总一次；
- 每 10 分钟做一次只读对账，发现“开始后长期无终止事件”的任务就标记 `UNKNOWN`；
- 先观察两天再启用正式通知规则。

## 十一、macOS 四工具真实投递 PoC

2026-08-29 在本机向 Codex、Claude、Cursor、Antigravity IDE 实际投递同一问题“当前使用大模型是哪个？”，并通过界面与本地会话记录交叉回读。结果为 **4/4 投递成功、4/4 获得终态回复**：

| 工具 | 实测回复或元数据 | 终态证据 | 观测耗时 |
|---|---|---|---:|
| Codex Desktop | `gpt-5.6-sol`，`xhigh` | 当前 rollout 的 `turn_context` | 当前任务内实时读取 |
| Claude Desktop | Opus 5，模型 ID `claude-opus-5` | 会话 JSONL 的 `stop_reason=end_turn` | 消息落盘至回复约 4.1 秒 |
| Cursor Agents | Auto（Cursor Agent Router），不固定披露底层模型 | Agent transcript 的 `turn_ended=success` | 不超过 10 秒 |
| Antigravity IDE | Gemini 3.7 Flash；界面选择器为 Gemini 3.7 Flash Medium | transcript 的 `PLANNER_RESPONSE=DONE` | 日志内不足 1 秒；界面轮询不超过 13 秒 |

这次实测同时验证了三件事：

1. macOS 辅助功能可以完成跨 IDE 的焦点切换、输入和发送，但坐标与窗口结构会随版本变化；
2. Cursor、Claude、Antigravity 均产生可解析的本地终态会话记录，稳定性明显高于截图 OCR；
3. 最佳产品形态仍是“GUI 负责触发或兜底，本地会话记录、Hook、App Server、CLI/API 负责确认、继续和审计”。

本次只完成 macOS 真机验证；Windows 的结论来自接口与架构分析，仍需要 Windows Native 与 WSL 各做一轮相同验收。详细证据见 `runs/20260829-model-probe/summary.md`。
