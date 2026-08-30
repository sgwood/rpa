# AI 任务中控台技术实施方案

> 版本：v1.0
> 日期：2026-08-29
> 对应产品文档：[PRODUCT_PRD.md](PRODUCT_PRD.md)
> 对应测试计划：[TEST_PLAN.md](TEST_PLAN.md)

## 1. 技术决策摘要

首版采用“**本机节点 + 可选中央控制面 + Web 控制台**”架构。

- 本机节点使用 **Go**：编译为单文件，适合 macOS/Windows 常驻、Hook 快速启动、并发采集和跨平台分发；
- Web 控制台使用 **TypeScript + React + Vite**：组件生态成熟，便于制作任务时间线、筛选和飞书卡片跳转页；
- 本地状态使用 **SQLite**：离线可用、事务与幂等能力足够，不要求用户安装数据库；
- 中央服务使用 **Go + PostgreSQL**：复用领域模型，同时支持多设备汇总、审计和远程命令；
- 节点只建立出站 **HTTPS/WSS** 连接，避免给个人电脑开放入站端口；
- MVP 使用数据库事务 Outbox，不引入消息队列；吞吐或团队规模达到阈值后再评估 NATS JetStream；
- 状态由确定性状态机判断，模型只生成摘要，不能决定任务是否成功；
- 厂商正式 Hook、CLI、SDK、App Server、Cloud API 优先，GUI 自动化只做显式降级。

Go、Node.js、依赖和安装器工具均选择开发时的最新稳定版，并通过 `go.mod`、锁文件和构建镜像固定；不在架构文档中硬编码容易过时的小版本。

## 2. 为什么选择 Go

| 关注点 | Go | Electron/纯 Node.js | Rust | 结论 |
|---|---|---|---|---|
| macOS/Windows 单文件分发 | 好 | 通常需要较大运行包 | 好 | Go/Rust 更合适 |
| Hook 冷启动和资源占用 | 低 | 中 | 最低 | Go 足够 |
| HTTP、WSS、SQLite、并发 | 成熟 | 成熟 | 成熟但开发成本较高 | Go 平衡最好 |
| 系统服务与 CLI | 成熟 | 一般 | 成熟 | Go |
| 团队学习和迭代速度 | 较低门槛 | 最低门槛 | 较高门槛 | Go |
| 内存安全 | GC + 类型安全 | GC + 动态边界较多 | 最强 | 首版 Go，安全敏感模块加强测试 |

不建议用 Python 作为常驻核心：它适合 PoC 和 UI 辅助脚本，但跨平台打包、解释器/依赖管理和长期后台运行的交付成本更高。Python 可保留为可选的 UI 自动化插件进程，不进入可信状态判定链。

## 3. 总体架构

```text
Codex Hook/App Server ─┐
Claude Hook/CLI ───────┤
Cursor Hook/Cloud API ─┼─> Hook Shim / Adapter ─> Normalizer ─> State Engine
Antigravity Hook/CLI ──┘          │                    │              │
                                  │                    v              v
                                  │               Evidence       SQLite + Outbox
                                  │                                     │
                                  └──── Command Runner <──── Queue/Lease │
                                                                        │
                                                    outbound HTTPS/WSS  │
                                                                        v
                                                    Central Control Plane
                                                    PostgreSQL / API / Feishu
                                                                        │
                                                                        v
                                                            React Web Console
```

### 3.1 进程划分

1. `ai-rpa-node`：每个登录用户一个常驻进程，负责事件、状态、命令和本地存储；
2. `ai-rpa hook <provider> <event>`：极轻量 Hook 入口，把标准输入转发给节点后立即退出；
3. `ai-rpa-server`：可选中央服务，负责设备同步、用户权限、飞书通知和 Web API；
4. `ai-rpa-ui`：React 控制台；本地版静态资源可嵌入节点，团队版由中央服务托管；
5. `ui-driver-*`：可选、隔离的 GUI 辅助进程，默认不安装，不参与成功判定。

### 3.2 部署模式

| 模式 | 组件 | 适用场景 |
|---|---|---|
| Local Only | Node + SQLite + 本地 Web UI | 单机试用、数据不出设备 |
| Personal Sync | 多 Node + Server + PostgreSQL + 飞书 | 用户的 Mac、Windows 多设备 |
| Team | 多 Node + HA Server + PostgreSQL + SSO/RBAC | 团队统一监控和审计 |

## 4. 本机节点模块

推荐目录结构：

```text
cmd/ai-rpa/                 CLI、Hook Shim、服务入口
internal/adapter/           codex、claude、cursor、antigravity
internal/discovery/         安装、进程、版本和配置发现
internal/event/             统一事件模型、校验、去重
internal/state/             确定性状态机
internal/evidence/          结果与证据判定
internal/command/           队列、租约、投递、重试
internal/store/             SQLite/PostgreSQL repository
internal/outbox/            可靠同步与通知
internal/redact/            凭据和隐私脱敏
internal/transport/         IPC、HTTPS、WSS、mTLS
internal/notify/            飞书与本机通知
web/                        React 控制台
testdata/providers/         脱敏事件 Golden Fixtures
```

### 4.1 IPC

- macOS：`Unix Domain Socket`，目录权限 `0700`、Socket 权限 `0600`；
- Windows：当前用户 ACL 限制的 `Named Pipe`；
- 调试降级：仅监听 `127.0.0.1` 的随机端口，并要求短期令牌；
- Hook 总时限默认 800 ms，转发超过 500 ms 即落本机 spool 或 fail-open；
- Hook 的标准输出必须严格符合对应厂商协议，诊断日志只写标准错误或节点日志。

### 4.2 事件入口

每个厂商适配器负责：

1. 校验事件大小、编码和必要字段；
2. 保留原厂商事件类型与脱敏后的最小原文；
3. 映射为统一事件；
4. 生成稳定幂等键；
5. 标记解析版本，允许未来重新对账；
6. 在需要时返回 Hook 控制结果，例如下一条任务。

统一事件示例：

```json
{
  "schemaVersion": 1,
  "eventId": "01K...",
  "idempotencyKey": "sha256:...",
  "provider": "codex",
  "deviceId": "dev_...",
  "sessionId": "provider-session-id",
  "turnId": "provider-turn-id",
  "occurredAt": "2026-08-29T10:20:30.123Z",
  "receivedAt": "2026-08-29T10:20:30.231Z",
  "type": "TURN_STOPPED",
  "outcome": "unknown",
  "controlMode": "OBSERVED",
  "capabilities": ["SEND_NEXT"],
  "evidenceRefs": ["evt_..."],
  "attributes": {
    "terminationReason": "end_turn"
  }
}
```

禁止把令牌、完整环境变量、完整 Prompt、完整代码 diff 或原始桌面截图放入统一事件。需要留存的原始证据加密存储在本机，中央端仅保存摘要、哈希和受控链接。

## 5. 厂商适配方案

### 5.1 能力矩阵

| 工具 | 首选状态入口 | `SEND_NEXT` | 历史会话恢复 | 执行中追加 | 关键边界 |
|---|---|---|---|---|---|
| Codex | Hooks；托管模式用 App Server | Stop Hook 或 `turn/start` | `thread/resume` | `turn/steer` | `turn/steer` 必须匹配当前活动 `turnId` |
| Claude | Hooks；托管模式用 CLI/SDK | Stop Hook 返回 block/reason 后继续，或 CLI 新消息 | `--resume` / session ID | 默认排队；可选能力单独探测 | Stop Hook 必须处理重复触发保护 |
| Cursor IDE | 本机 Hooks | `stop` 的 `followup_message` | 仅承诺产品登记的可恢复会话 | 默认排队 | 不承诺任意手工 IDE Chat 的稳定恢复 |
| Cursor Cloud | v1 API 状态查询/SSE | 新建后续 Run | Durable Agent 上下文 | busy 时排队重试 | v1 为 Public Beta；v1 Webhook 尚未开放 |
| Antigravity | Hooks；托管模式用 `agy --output-format stream-json` | Stop Hook `decision: continue` 或持久 stdin | `--conversation` / conversation ID | 默认排队 | `fullyIdle` 仍需结合结果证据，不能直接当成功 |

适配器启动时执行能力探测，将 `capabilities` 固化到会话快照。控制台只根据快照显示按钮，不能只根据产品名称假设能力。

### 5.2 Codex

- 观察模式：接收 Stop、SessionEnd 等 Hook；
- 托管模式：通过 App Server 创建/恢复 thread，使用 `turn/start` 下发任务；
- 活动回合追加：使用 `turn/steer`，携带当前预期 `turnId`；不匹配则返回冲突并转为 `SEND_NEXT`；
- 状态判定：Hook 只表示生命周期节点；结果、工具错误和项目验证证据共同决定终态；
- 会话入口和项目工作区映射必须由正式返回值登记，不解析或改写 Codex 内部数据库。

### 5.3 Claude

- 使用 Stop、TaskCompleted、StopFailure 等 Hook 采集生命周期；
- Stop Hook 通过结构化返回控制是否继续，并检查 `stop_hook_active` 防止无限循环；
- 产品托管的 CLI/SDK 会话保存 session ID，使用 `--resume` 或 SDK 对应能力继续；
- 用户手工会话若没有可靠的可恢复标识，只开放 `OPEN_AND_PREFILL`；
- 权限询问映射到 `WAITING_USER`，不得自动批准敏感操作。

### 5.4 Cursor

- 本机 IDE 使用 Hooks 采集会话/Agent 状态；
- 当前回合结束时，可通过 `stop` Hook 的 `followup_message` 执行 `SEND_NEXT`；
- 必须记录并限制连续 follow-up 次数，达到产品阈值或厂商阈值时停止并通知；
- 仅对产品创建或登记、且已验证可恢复的会话提供 `RESUME_AND_SEND`；
- Cursor Cloud Agents v1 使用公开 API 创建和查询 Run；遇到 busy 冲突进入带抖动的指数退避；
- v1 Webhook 未开放前，以有界轮询/SSE 为正式方案；若需要 Webhook，只能把旧版 v0 作为独立兼容适配器，不能把它描述成 v1 能力。

### 5.5 Antigravity

- Hook Stop 事件提供 conversation ID、终止原因、`fullyIdle` 和模型信息，统一映射后仍进入证据判定；
- 托管模式使用 `agy` 的 `stream-json`，保持 stdin 或用 conversation ID 恢复；
- Stop Hook 可返回 `decision: continue` 投递队列中的下一条任务；
- IDE 手工会话只有在已捕获可靠 conversation ID 且完成恢复验证后才开放恢复；
- Remote Control 可用于人工接管入口，但不能作为状态真值源。

## 6. 状态机与成功判定

### 6.1 判定顺序

```text
明确取消事件                         => CANCELLED
明确错误/非零退出/验证失败           => FAILED
等待授权、登录、输入、选择           => WAITING_USER
有活动开始或工具执行事件             => RUNNING
终止事件 + 结构化成功 + 必要证据      => SUCCEEDED
只有 Stop/idle/窗口关闭/超时          => UNKNOWN
```

同一事件批次同时包含多个信号时，优先级为 `CANCELLED > FAILED > WAITING_USER > SUCCEEDED > RUNNING > UNKNOWN`。晚到事件可以纠正 `UNKNOWN`，但对已经通知的终态变更必须产生审计事件和更正通知。

### 6.2 证据等级

| 等级 | 证据 | 允许结论 |
|---|---|---|
| E0 | 窗口存在、进程存在、idle | 仅能确认已发现/状态不明 |
| E1 | Stop/SessionEnd Hook | 确认回合停止，不能确认成功 |
| E2 | 结构化结果、无工具错误、正常退出 | 一般问答可判成功 |
| E3 | 测试报告、文件哈希、命令退出码、部署回读等 | 有交付要求的任务可判成功 |

创建任务时可配置 `requiredEvidenceLevel`。例如“回答一个问题”为 E2，“修改代码并通过测试”为 E3。产品不得使用 LLM 对文本“看起来完成了”的判断代替 E2/E3。

### 6.3 幂等与乱序

- `idempotency_key` 唯一索引阻止重复事件；
- 同一会话按 `occurred_at + provider_sequence + received_at` 归并；
- Provider 无序号时，不依赖接收顺序覆盖高优先级终态；
- 状态变更、Outbox 写入和命令 ACK 在一个数据库事务中完成；
- 原始事件进入不可变表，派生状态可重放重建；
- 状态机版本写入快照，升级后可对限定时间窗重算。

## 7. 命令队列与会话续跑

### 7.1 命令状态

```text
CREATED -> QUEUED -> LEASED -> DELIVERED -> ACCEPTED -> COMPLETED
                      |             |            |
                      +----------> RETRY_WAIT ---+
                                    |
                                    +----------> EXPIRED | FAILED | CANCELLED
```

### 7.2 只执行一次的工程语义

跨进程/网络严格 exactly-once 不现实，采用“**至少一次传输 + 幂等消费**”：

- 用户或服务端生成全局 `commandId`；
- 节点领取命令时获得带期限的 lease；
- Hook 返回下一任务前，在本地事务中将命令置为 `DELIVERED` 并记录会话/回合；
- 厂商支持幂等键时透传 `commandId`；不支持时由节点维护投递账本；
- 进程在不确定窗口崩溃时，状态为 `UNKNOWN_DELIVERY`，禁止静默重投，由对账器或用户决定；
- 每条命令有 TTL、最大重试数、退避策略和取消入口；
- `SEND_NEXT` 默认每个会话只允许一个执行中命令，其余 FIFO 排队。

### 7.3 安全边界

远程下发不是远程任意命令执行。命令载荷只包含已定义字段：工具、会话、动作、用户消息、证据要求和 TTL。节点禁止接收 shell、脚本路径、环境变量或任意 Hook 返回 JSON。

## 8. 数据模型

核心表：

| 表 | 关键字段 | 说明 |
|---|---|---|
| `devices` | id, os, arch, node_version, last_seen_at, revoked_at | 设备身份与心跳 |
| `agent_sessions` | provider, provider_session_id, device_id, mode, capabilities | 会话登记与能力快照 |
| `agent_turns` | session_id, provider_turn_id, state, evidence_level | 统一任务/回合 |
| `events` | idempotency_key, type, payload_redacted, occurred_at | 不可变事件账本 |
| `evidence` | kind, local_ref, digest, summary, sensitivity | 证据摘要与本机引用 |
| `commands` | action, body_encrypted, state, ttl, lease_owner | 后续任务队列 |
| `outbox` | aggregate_id, topic, payload, sent_at | 可靠同步/通知 |
| `notifications` | channel, dedupe_key, state, attempts | 通知记录 |
| `audit_logs` | actor, action, target, result, trace_id | 管理与安全审计 |

本地 SQLite 与中央 PostgreSQL 使用同一逻辑模型，但中央端不复制 `body_encrypted` 和本机原始证据，除非用户显式开启内容同步。

## 9. 接口设计

### 9.1 本地 API

| 方法 | 路径 | 用途 |
|---|---|---|
| `POST` | `/v1/events` | Hook/适配器提交事件 |
| `GET` | `/v1/tasks` | 本机任务列表 |
| `GET` | `/v1/tasks/{id}` | 任务详情、时间线和证据 |
| `POST` | `/v1/sessions/{id}/commands` | 创建继续任务 |
| `POST` | `/v1/commands/{id}/cancel` | 取消未投递命令 |
| `GET` | `/v1/health` | 活性与就绪检查 |
| `POST` | `/v1/diagnostics` | 生成脱敏诊断包 |

本地 API 默认只通过 UDS/Named Pipe 开放。调试 HTTP API 使用当前用户短期令牌与严格 Origin 检查。

### 9.2 中央 API

- `POST /v1/devices/enroll`：一次性注册码换取设备证书；
- `GET /v1/tasks`、`GET /v1/tasks/{id}`：跨设备查询；
- `POST /v1/sessions/{id}/commands`：创建受审计命令；
- `GET /v1/events/stream`：控制台 SSE；
- `POST /v1/integrations/feishu/callback`：飞书卡片回调；
- `POST /v1/nodes/connect`：升级为 WSS 后的设备双向通道。

所有写接口要求认证、目标设备/会话授权、幂等键和审计上下文。

## 10. 飞书通知设计

### 10.1 通知策略

- `WAITING_USER`、`FAILED`：立即通知；
- `UNKNOWN`：持续超过阈值后通知；
- `SUCCEEDED`：默认 5 分钟窗口聚合，可对重点任务即时通知；
- `CANCELLED`：用户主动取消不通知，异常取消即时通知；
- 同一 `taskId + stateVersion + channel` 只发送一次；
- 发送失败写 Outbox，指数退避，达到上限后在控制台告警。

### 10.2 凭据与卡片

- 飞书应用凭据存放在 macOS Keychain、Windows Credential Manager 或服务器密钥管理服务；
- 卡片仅展示脱敏摘要、工具、设备别名、耗时、状态和安全深链；
- “继续任务”按钮跳转到受认证页面确认，默认不在聊天卡片中直接执行任意 Prompt；
- 回调需要验签、防重放、用户身份映射和目标资源授权。

## 11. 安全与隐私

### 11.1 威胁边界

重点防范：恶意 Hook 输入、Prompt 注入变成控制指令、远程命令越权、重放、设备证书泄漏、诊断包泄密和 GUI 驱动误操作。

### 11.2 控制措施

- 每台设备独立密钥和证书，可单独撤销；
- 节点到服务端 TLS，团队模式启用 mTLS；
- 本地数据库敏感列应用层加密，密钥只存系统凭据库；
- 严格 JSON Schema、载荷上限、路径规范化和超时；
- Prompt 永远作为数据，不解释为本产品管理指令；
- Hook 子进程使用最小环境变量，不继承不必要凭据；
- 日志经过字段级脱敏，默认不记录 Prompt 与 Agent 完整回复；
- GUI 驱动运行在隔离进程，要求前台用户会话，敏感动作仍由原工具审批；
- 不写厂商内部会话数据库，不绕过 TCC、UAC、EDR 或企业策略；
- 更新包签名、校验哈希，支持回滚到上一稳定版本。

## 12. macOS、Windows 与 WSL 实现

### 12.1 macOS

- 以当前用户 `LaunchAgent` 运行，不使用 root daemon；
- Universal Binary 或分别发布 arm64/amd64，并签名、公证；
- GUI 降级需要 Accessibility/Screen Recording 时，安装器必须解释用途，未授权仍可使用事件模式；
- 配置、数据、日志放在用户 Library 的标准目录，卸载时询问是否保留历史。

### 12.2 Windows Native

- 本机节点在交互式用户会话中启动；若使用 Windows Service，Service 只负责更新/守护，不直接做 GUI 自动化；
- 使用 Named Pipe 和当前用户 SID ACL；
- 安装器采用 MSIX 或企业 MSI，二进制 Authenticode 签名；
- 凭据进入 Credential Manager/DPAPI；
- 支持睡眠/唤醒、快速用户切换和 UAC 场景；
- 不依赖 Session 0 与 IDE 交互。

### 12.3 WSL

- WSL 内安装独立 Linux 节点，通过出站连接登记为不同逻辑设备；
- 不直接读取 Windows 节点 SQLite；
- 工作区路径以 URI 表示，避免 `C:\...` 与 `/mnt/c/...` 被误判为两个项目；
- Windows IDE 与 WSL CLI 的会话归并必须通过显式关联，不做路径猜测。

## 13. 可观测性与诊断

- 统一结构化日志：`trace_id`、`device_id`、`provider`、`session_id_hash`、`event_type`、`result`；
- 指标：Hook 延迟/失败、事件积压、状态分布、命令投递延迟、通知延迟、适配器错误率、数据库大小；
- 健康检查区分 `liveness` 与 `readiness`；
- 适配器连续失败自动熔断，不影响其他工具；
- 诊断包包含版本、脱敏配置、最近错误、健康结果和事件统计，不包含令牌、完整 Prompt、源码与截图；
- 每个状态和通知能沿 `trace_id` 回溯到原事件与规则版本。

## 14. 性能与容量目标

| 指标 | MVP 目标 |
|---|---:|
| Hook 入口 P95 | 小于 100 ms（节点在线） |
| Hook fail-open 上限 | 800 ms |
| 本机事件到状态更新 P95 | 小于 1 秒 |
| 在线事件到中央控制台 P95 | 小于 5 秒 |
| 异常到飞书通知 P95 | 小于 30 秒 |
| 节点空闲内存 | 小于 100 MB |
| 节点空闲 CPU | 小于 1% 单核均值 |
| 单设备离线缓存 | 至少 100,000 事件 |
| 单用户任务查询 | 10,000 条下 P95 小于 1 秒 |

性能目标是发布门槛，不是厂商事件自身延迟的保证；报告需拆分“厂商到 Hook”和“Hook 到通知”。

## 15. 交付阶段

### 阶段 A：事件内核（2 周）

- Go 工程、SQLite migration、统一事件和状态机；
- Hook Shim、幂等、Outbox、脱敏日志；
- 四厂商脱敏 Fixture 与契约测试；
- 本机 CLI 状态查询。

### 阶段 B：本机 MVP（2 周）

- 四适配器真实接入；
- `SEND_NEXT`、命令账本和恢复；
- React 本机控制台；
- macOS/Windows 安装、升级、卸载；
- 飞书单用户通知。

### 阶段 C：多设备 Beta（2～3 周）

- 中央 Go 服务、PostgreSQL、设备注册、WSS；
- 多设备任务和远程命令；
- RBAC、审计、飞书卡片；
- Windows Native、WSL、睡眠/断网和 72 小时稳定性测试。

### 阶段 D：受控 GA（2 周）

- 签名、公证、自动更新和回滚；
- 安全测试、性能基线、兼容矩阵；
- 运维手册、故障演练、数据保留策略；
- 按 [TEST_PLAN.md](TEST_PLAN.md) 完成发布门禁。

按 2～3 名工程师估算，受控 GA 为 8～10 周；若首版同时要求企业 SSO、高可用和 UI 自动化模板库，应另加 4～6 周并独立评估。

## 16. 明确不采用的实现

- 不用窗口标题、像素颜色或 OCR 作为唯一任务状态；
- 不轮询并改写工具内部 SQLite/LevelDB 来投递消息；
- 不让云端直接执行任意本机 shell；
- 不把 Hook 的 Stop/idle 一律映射成成功；
- 不让每个 Hook 直接调用飞书，避免阻塞、重复和密钥散落；
- MVP 不先上 Kafka、Kubernetes 或微服务拆分；当前规模用模块化单体更容易验证可靠性。

## 17. 官方能力依据

- [OpenAI Codex Hooks](https://learn.chatgpt.com/docs/hooks)
- [OpenAI Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [OpenAI Codex Automations](https://learn.chatgpt.com/docs/automations)
- [Claude Code Hooks](https://code.claude.com/docs/en/hooks)
- [Claude Code Sessions](https://code.claude.com/docs/en/sessions)
- [Cursor Hooks](https://cursor.com/docs/hooks)
- [Cursor Cloud Agents API](https://cursor.com/docs/cloud-agent/api/endpoints)
- [Antigravity Hooks](https://antigravity.google/docs/hooks)
- [Antigravity Headless CLI](https://antigravity.google/docs/cli/headless/)
- [Antigravity SDK Lifecycle](https://antigravity.google/docs/sdk/lifecycle)

厂商能力会变化。适配器发布前必须重新运行契约测试和真机 Smoke Test，运行时必须做能力探测，不能仅依赖本文档。
