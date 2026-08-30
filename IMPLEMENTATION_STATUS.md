# AI 任务中控台首版功能与验证状态

> 基线：v0.2.0，2026-08-30
> 范围：P0 本地版 + P1 单用户 Personal Sync；团队 RBAC 与正式 GA 门禁不计入本版。

## 0. v0.2 Personal Sync

| 功能 | 实现位置 | 自动验证 | 当前结论 |
|---|---|---|---|
| ctyun Rust 中央服务 | `crates/server` | 编译、认证和 PostgreSQL 集成测试 | 代码完成；ctyun RC 待部署 |
| PostgreSQL 中央账本 | `crates/server/migrations` | 注册、事件、任务、命令闭环 | 完成 |
| 一次性配对与设备撤销 | 中央设备 API、节点系统凭据库 | 令牌哈希、配对码消费、撤销契约 | 完成 |
| 公网 WSS 双向通道 | `crates/node/src/sync.rs`、中央 WebSocket Hub | 协议、重连和幂等存储测试 | 代码完成；真实弱网待 RC |
| 离线补传 | SQLite `sync_events` + batch ACK | 重试、ACK 后删除、重复事件测试 | 完成 |
| 远程继续任务 | 中央加密命令、节点导入、状态回执 | 目标会话、重复命令、投递状态测试 | 完成 |
| 多设备 PC/手机 UI | `apps/desktop` | 登录、总览、设备管理、响应式构建 | 完成 |
| ctyun 容器交付 | `Dockerfile.central`、`deploy/ctyun` | Compose 校验、CI PostgreSQL 服务 | 完成；正式域名/ELB/RDS 待配置 |

## 1. P0 功能对照

| P0 功能 | 实现位置 | 自动验证 | 当前结论 |
|---|---|---|---|
| macOS / Windows 本机节点 | `crates/node`、Tauri 内嵌节点、双平台安装脚本 | Tauri 编译、CLI、API 测试 | 代码完成；Windows 真机待 RC |
| 四工具 Hook 接入 | `hook_install.rs`、`hook.rs` | 合并、幂等、损坏 JSON、四厂商契约测试 | 完成 |
| 总览、列表、详情、时间线、设备 | `apps/desktop/src/App.tsx` | React 渲染测试、生产构建、桌面/移动浏览器验收 | 完成 |
| 任务筛选与检索 | Provider、状态、控制模式、项目、设备、更新时间、全文搜索 | Store 过滤测试、桌面/390×844 浏览器验收 | 完成 |
| 六态状态机 | `crates/core/src/state.rs` | 成功门槛、Stop 不误判、晚到错误、等待恢复 | 完成 |
| 飞书异常即时通知 | `notify.rs` + SQLite Outbox | URL 边界、重试状态与脱敏单元测试 | 代码完成；真实 Webhook 待验收 |
| 飞书成功汇总 | `notify.rs` | 多任务合并卡片测试 | 完成 |
| `SEND_NEXT` | Stop Hook + SQLite 命令账本 | 重复 Stop 只投一次、同一会话只租用一条命令 | 完成 |
| 托管 `RESUME_AND_SEND` | `command_runner.rs` | 能力校验、租约恢复、重试上限 | 完成；各 CLI 版本真机待 RC |
| `OPEN_AND_PREFILL` | 打开工具/工作区 + 系统剪贴板 | 前端类型检查与构建 | 完成；出于安全不自动模拟粘贴/回车 |
| SQLite / 离线 / 幂等 / 脱敏 | `crates/store`、spool、redact | 1,000 次重复风暴、加密、spool 契约 | 完成 |
| 安装 / 升级 / 卸载 / 诊断 | `scripts`、Hook 备份、诊断 API | Shell 语法、Tauri 双平台 CI | 代码完成；签名包待 RC |
| Windows Native / WSL 分设备 | `config.rs` | 逻辑环境单元测试 | 完成；WSL 真机待 RC |
| 移动端准备 | 响应式 UI、Tauri 生成 iOS/Android 图标、共享 Rust Core | 前端生产构建 | 架构就绪；移动安装包属后续版本 |

## 2. 安全边界

- API 仅允许 loopback；不提供远程任意 shell。
- Hook 输入最大 1 MiB，未知关键事件进入隔离队列；Hook 离线时本机落盘，避免阻断 IDE。
- 命令正文 AES-256-GCM 加密，主密钥和飞书 Webhook 进入操作系统凭据库。
- 单任务最多排队 5 条命令；全局待处理命令达到 1,000 条时熔断，防止失控堆积。
- 日志、通知和诊断不包含完整 Prompt、源码、令牌或截图。
- `RESUME_AND_SEND` 不附加跳过权限、绕过沙箱等参数；删除、发布、支付仍由原工具审批。
- `OPEN_AND_PREFILL` 只打开工具并复制文本，不自动粘贴或发送，避免焦点错误把内容送到其他窗口。

## 3. 可复现验证

```bash
./scripts/verify.sh
```

流水线执行：Rust 格式、Clippy、全 workspace 测试、React 测试、TypeScript/Vite 构建，并在 macOS 与 Windows 构建 Tauri 原生安装包。

当前自动测试覆盖统一状态机、脱敏、四厂商 Hook 输入/输出、配置合并、SQLite 幂等、中央同步队列、远程命令幂等、租约恢复、飞书汇总和控制台渲染。浏览器验收已覆盖桌面端中央总览/设备配对，以及 390×844 手机端设备管理、任务详情和远程续派；本轮前端共通过 6 个测试，Rust 全 workspace 测试通过。PostgreSQL 集成用例已进入 CI，但本机 Docker 服务未运行，因此本轮本机数据库实跑记为 `SKIPPED`，不能冒充通过。

## 4. 发布候选仍需外部环境完成的门禁

以下项目不是代码缺口，但本机 macOS 环境不能替代其正式证据：

1. Windows 11 Native 和 WSL2 安装、睡眠唤醒、卸载保留/清除数据；
2. 四工具当前稳定版在 macOS、Windows 的真实 Hook、失败、等待人工、`SEND_NEXT` 闭环；
3. 使用受控飞书测试群验证真实回执、限流和 5xx 重试；
4. Codex、Claude、Antigravity 托管恢复命令的版本兼容；
5. macOS 签名/公证、Windows Authenticode 与 SmartScreen；
6. 72 小时稳定性、100 events/s 持续负载和磁盘满/数据库锁故障注入。
7. ctyun ECS/ELB/RDS 上的 PostgreSQL 集成、双公网节点 WSS 重连、撤销令牌和离线补传闭环。

这些门禁未通过前，交付结论只能是“首版功能代码完成并通过本机自动验证”，不能声称“双平台 GA 已验收”。
