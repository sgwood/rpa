# 贡献指南

感谢你参与 AI 任务中控台。项目优先接受能够保持本地优先、可审计和跨平台特性的改动。

## 开始之前

1. 搜索现有 Issue，避免重复工作。
2. 对较大的功能、协议变化或新工具适配器，先创建 Feature Request 讨论边界。
3. 不要在 Issue、测试夹具、提交记录或截图中包含真实提示词、会话、源码、Webhook、令牌和个人路径。

参与项目即表示你同意遵守[行为准则](CODE_OF_CONDUCT.md)。安全问题请按[安全策略](SECURITY.md)私密报告。

## 开发环境

- Rust 1.98，包含 `rustfmt` 与 `clippy`
- Node.js 24 及 npm
- macOS 或 Windows

```bash
git clone https://github.com/sgwood/rpa.git
cd rpa
npm --prefix apps/desktop ci
```

运行本地节点与前端：

```bash
cargo run -p ai-rpa-node -- serve
npm --prefix apps/desktop run dev
```

## 设计约束

- 优先使用工具官方 Hook、CLI 或 SDK；UI 自动化只能作为显式标注的降级方案。
- 进程在线、Hook 事件、任务完成和结果成功是不同证据，不得互相推断。
- 不采集或持久化提示词、完整回复、聊天记录和终端原文，除非有明确、最小化且可测试的产品需求。
- API 默认仅监听本机回环地址；新增网络入口必须附带威胁分析和授权机制。
- macOS 与 Windows 行为都需要测试。无法实测的平台必须明确标为 `NOT RUN`，不能写成通过。
- 适配器解析应使用脱敏夹具并覆盖重复事件、乱序事件和失败修正。

## 提交前检查

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
```

也可以在 macOS 运行 `./scripts/verify.sh`，或在 Windows PowerShell 运行 `.\scripts\verify.ps1`。

## Pull Request

- 一个 PR 只解决一个清晰问题，并包含必要测试和文档。
- 标题建议使用 `feat:`、`fix:`、`docs:`、`test:`、`refactor:` 或 `chore:` 前缀。
- 说明用户可见变化、验证环境、已通过检查和未执行项。
- UI 变化请附脱敏截图；Hook 或协议变化请附最小化事件样例。
- 不要提交生成目录、数据库、诊断压缩包、签名材料或真实运行日志。

维护者可能要求拆分过大的 PR，或在无法验证安全和证据边界时暂缓合并。
