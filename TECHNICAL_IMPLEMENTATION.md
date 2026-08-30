# AI 任务中控台技术实施方案

> 版本：v1.1
> 日期：2026-08-30
> 对应产品文档：[PRODUCT_PRD.md](PRODUCT_PRD.md)
> 对应测试计划：[TEST_PLAN.md](TEST_PLAN.md)

## 1. 技术决策摘要

首版采用“**Rust 本机节点 + Tauri 桌面壳 + React 控制台 + SQLite**”的本地优先架构。

- 原生核心使用 **Rust 1.98**：同一套领域代码覆盖 macOS、Windows、CLI、Hook 和 Tauri；
- 桌面端使用 **Tauri 2 + TypeScript + React + Vite**：复用系统 WebView，包体和常驻资源小于 Electron 路线；
- 本地状态使用 **SQLite WAL**：事件、任务快照、命令、审计和 Outbox 在事务中一致更新；
- 密钥进入 **macOS Keychain / Windows Credential Manager**，命令正文用 AES-256-GCM 加密；
- 本地 API 只监听 `127.0.0.1:3847`，Tauri、Hook CLI 与浏览器开发环境使用同一接口；
- 状态由确定性状态机判断，模型文本不能越过配置的 E2/E3 证据门槛；
- P0 仍可完全本地运行；v0.2 已加入面向 ctyun 的单用户 Personal Sync。团队 RBAC、高可用和企业 SSO 保留到后续版本。

Rust、Node.js 和依赖由 `rust-toolchain.toml`、`Cargo.lock`、`.nvmrc`、`package-lock.json` 固定。

## 2. 为什么选择 Rust + Tauri

| 关注点 | Rust + Tauri | Electron/纯 Node.js | Go + Web UI | 结论 |
|---|---|---|---|---|
| macOS/Windows 原生分发 | 好，Tauri 直接产出 DMG/MSI/NSIS | 包体大、需捆绑 Chromium | 后端好，桌面壳需另选 | Rust + Tauri |
| Hook 冷启动与常驻资源 | 低 | 中 | 低 | Rust/Go 均可 |
| 内存与并发安全 | 编译期强保证 | 运行时边界较多 | GC + 类型安全 | Rust 最强 |
| SQLite、HTTP、加密、系统凭据 | 生态成熟 | 生态成熟 | 生态成熟 | 均可 |
| 移动端演进 | Tauri 2 可共享前端与 Rust Core | 需另建移动端 | 需另建移动端 | Tauri 更连贯 |
| 工程复杂度 | 较高，但领域与壳统一 | 前端团队上手快 | 双技术栈且需额外桌面壳 | 接受 Rust 成本 |

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
                                                    P1 Central Control Plane
                                                    PostgreSQL / API / Feishu
                                                                        │
                                                                        v
                                                            React Web Console
```

### 3.1 进程划分

1. `ai-rpa-node`：每个登录用户一个常驻进程，负责事件、状态、命令和本地存储；
2. `ai-rpa hook <provider> <event>`：极轻量 Hook 入口，把标准输入转发给节点后立即退出；
3. `ai-rpa-desktop`：Tauri 原生程序；正常启动显示 UI，携带 CLI 子命令时作为后台节点或 Hook 运行；
4. `apps/desktop`：React 控制台和响应式移动布局；
5. `ai-rpa-server`：部署在 ctyun 的独立 Rust 中央服务，负责 PostgreSQL、设备配对、WSS、远程命令和 Web 控制台；
6. GUI 输入驱动仍不进入可信链，继续只作为未来的隔离降级插件。

### 3.2 部署模式

| 模式 | 组件 | 适用场景 |
|---|---|---|
| Local Only | Node + SQLite + 本地 Web UI | 单机试用、数据不出设备 |
| Personal Sync | 多 Node + Server + PostgreSQL + 飞书 | 用户的 Mac、Windows 多设备 |
| Team | 多 Node + HA Server + PostgreSQL + SSO/RBAC | 团队统一监控和审计 |

## 4. 本机节点模块

推荐目录结构：

```text
crates/core/                统一模型、厂商映射、状态机、脱敏
crates/store/               SQLite、加密命令、租约、审计、Outbox
crates/node/                API、CLI、Hook、发现、通知、诊断、命令运行器
apps/desktop/               React/Vite 控制台
apps/desktop/src-tauri/     Tauri 壳、内嵌本机节点、原生打包
scripts/                    macOS/Windows 安装卸载与验证脚本
.github/workflows/          双平台验证与原生包构建
```

### 4.1 IPC

- P0：固定监听 `127.0.0.1:3847`，拒绝非 loopback bind，并限制 Tauri/本机开发 Origin；
- 数据目录权限在 Unix 上设为 `0700`，spool 文件设为 `0600`；
- P1：迁移到 Unix Domain Socket / Windows Named Pipe，并增加当前用户 ACL；
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
| Codex | Hooks；P0 托管模式用 CLI | Stop Hook | `codex exec resume` | P1 使用 `turn/steer` | P0 不宣称活动回合插话能力 |
| Claude | Hooks；托管模式用 CLI/SDK | Stop Hook 返回 block/reason 后继续，或 CLI 新消息 | `--resume` / session ID | 默认排队；可选能力单独探测 | Stop Hook 必须处理重复触发保护 |
| Cursor IDE | 本机 Hooks | `stop` 的 `followup_message` | 仅承诺产品登记的可恢复会话 | 默认排队 | 不承诺任意手工 IDE Chat 的稳定恢复 |
| Cursor Cloud | v1 API 状态查询/SSE | 新建后续 Run | Durable Agent 上下文 | busy 时排队重试 | v1 为 Public Beta；v1 Webhook 尚未开放 |
| Antigravity | Hooks；托管模式用 `agy --output-format stream-json` | Stop Hook `decision: continue` 或持久 stdin | `--conversation` / conversation ID | 默认排队 | `fullyIdle` 仍需结合结果证据，不能直接当成功 |

适配器启动时执行能力探测，将 `capabilities` 固化到会话快照。控制台只根据快照显示按钮，不能只根据产品名称假设能力。

### 5.2 Codex

- 观察模式：接收 Stop、SessionEnd 等 Hook；
- P0 托管模式：使用 `codex exec resume <session-id> - --json`，任务正文通过 stdin 传入，避免出现在进程参数中；
- P1 再接入 App Server 的 `thread/resume`、`turn/start` 和 `turn/steer`；活动回合追加必须携带当前预期 `turnId`，不匹配则返回冲突并转为 `SEND_NEXT`；
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

本地 SQLite 与中央 PostgreSQL 使用同一逻辑模型。中央端同步脱敏任务事件和摘要，不上传本机原始证据；用户从中央端主动创建的远程任务正文使用独立 AES-256-GCM 密钥加密后保存，API、日志与审计只返回脱敏元数据。

## 9. 接口设计

### 9.1 本地 API

| 方法 | 路径 | 用途 |
|---|---|---|
| `POST` | `/api/events` | Hook/适配器提交事件 |
| `GET` | `/api/tasks` | 本机任务列表与筛选 |
| `GET` | `/api/tasks/{id}` | 任务详情、时间线、命令和审计 |
| `POST` | `/api/tasks/{id}/commands` | 创建继续任务 |
| `POST` | `/api/tasks/{id}/open` | 打开对应工具/工作区 |
| `POST` | `/api/hooks/install` | 备份并幂等合并四工具 Hook |
| `POST` | `/api/hooks/uninstall` | 只移除本产品 Hook 条目 |
| `GET` | `/api/health` | 活性检查 |
| `GET` | `/api/diagnostics/export` | 下载脱敏诊断 JSON |
| `POST` | `/api/settings/feishu` | 把 Webhook 写入系统凭据库 |
| `POST` | `/api/notifications/flush` | 立即处理通知 Outbox |

v0.2 本地 API 固定监听 loopback HTTP，并执行严格 Origin 检查；迁移到 UDS/Named Pipe 与当前用户 ACL 是后续加固项。

### 9.2 中央 API

- `POST /v1/devices/enroll`：一次性注册码换取独立设备令牌；
- `GET /v1/nodes/connect`：携带设备 Bearer Token 升级为 WSS 双向通道；
- `GET /api/session`、`GET /api/dashboard`：中央 Web 登录校验与跨设备总览；
- `GET /api/devices`、`POST /api/devices/enrollment-codes`、`PATCH /api/devices/{id}`、`POST /api/devices/{id}/revoke`：设备生命周期；
- `GET /api/tasks`、`GET /api/tasks/{id}`：跨设备任务查询；
- `POST /api/tasks/{id}/commands`：创建加密且受审计的结构化远程任务。

管理员接口要求 Bearer Token；设备接口要求独立设备令牌并核对设备 ID。v0.2 控制台使用有界轮询，团队身份、SSE 和飞书交互回调属于后续版本。

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

重点防范：恶意 Hook 输入、Prompt 注入变成控制指令、远程命令越权、重放、设备令牌泄漏、诊断包泄密和 GUI 驱动误操作。

### 11.2 控制措施

- 每台设备独立随机令牌，服务端只保存哈希并可单独撤销；团队版再升级为短期凭据或 mTLS；
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

### 阶段 A：事件内核（首版已实现）

- Rust workspace、SQLite schema、统一事件和状态机；
- Hook Shim、幂等、Outbox、脱敏日志；
- 四厂商脱敏 Fixture 与契约测试；
- 本机 CLI 状态查询。

### 阶段 B：本机 MVP（代码已实现，RC 真机门禁待完成）

- 四适配器真实接入；
- `SEND_NEXT`、命令账本和恢复；
- React 本机控制台；
- macOS/Windows 安装、升级、卸载；
- 飞书单用户通知。

### 阶段 C：多设备 Personal Sync Beta（v0.2 代码已实现，ctyun RC 待验收）

- 中央 Rust 服务、PostgreSQL、设备注册、WSS；
- 多设备任务和远程命令；
- 单管理员令牌、设备撤销、命令审计；团队 RBAC 与飞书交互卡片留待后续；
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
