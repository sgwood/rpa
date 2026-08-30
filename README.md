# AI 工具任务监控与续跑 RPA

面向 macOS 与 Windows 的本地优先 AI Agent 任务中控台，统一采集、汇总并继续执行以下工具中的任务：

- OpenAI Codex
- Claude
- Cursor
- Antigravity IDE

## 首版实现

首版采用 Rust + Tauri 2 + React + SQLite。一个原生程序同时提供桌面控制台、后台节点和 Hook CLI；所有 API 仅监听 `127.0.0.1`，提示词加密存储，飞书凭据进入系统钥匙串或 Credential Manager。

已实现：四工具安全 Hook 合并/卸载、统一状态机、任务时间线、`SEND_NEXT`、托管 `RESUME_AND_SEND`、打开工具并复制任务、飞书即时通知与成功汇总、离线 spool、命令租约恢复、脱敏诊断、macOS LaunchAgent、Windows 登录任务和双平台 Tauri 打包流水线。

```bash
# 完整验证
./scripts/verify.sh

# 开发启动：先启动本机节点，再启动前端
cargo run -p ai-rpa-node -- serve
cd apps/desktop && npm run dev

# 查看 CLI
cargo run -p ai-rpa-node -- --help
```

macOS 已完成一次四工具真实投递 PoC；Windows Native/WSL、真实飞书 Webhook、签名与公证仍必须在发布候选环境验收，不能用本机编译成功代替。

## 文档

- [产品需求文档（PRD）](PRODUCT_PRD.md)
- [完整技术可行性分析](AI_TASK_RPA_FEASIBILITY.md)
- [技术实施方案](TECHNICAL_IMPLEMENTATION.md)
- [测试与验证计划](TEST_PLAN.md)
- [首版功能与验证状态](IMPLEMENTATION_STATUS.md)
- [macOS 四工具实测摘要](runs/20260829-model-probe/summary.md)

推荐阅读顺序：PRD → 技术实施方案 → 测试与验证计划 → 可行性分析与 PoC 证据。

## 隐私与证据

原始桌面截图、时间戳和本机会话标识只保留在执行机器上，并通过 `.gitignore` 排除，避免上传企业源码画面、个人路径或不必要的会话信息。
