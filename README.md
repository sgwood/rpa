# AI 任务中控台

[![Verify and package](https://github.com/sgwood/rpa/actions/workflows/verify-and-package.yml/badge.svg)](https://github.com/sgwood/rpa/actions/workflows/verify-and-package.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.98-orange.svg)](rust-toolchain.toml)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB.svg)](https://tauri.app/)

一个面向 macOS 与 Windows 的、本地优先的开源 AI Agent 任务中控台。它统一采集和展示多个 AI 编程工具的任务状态，并在工具官方能力允许时继续发送任务。

> 项目目前处于 `0.1.x` 早期阶段。请先阅读[当前验证边界](#当前验证边界)，不要将“检测到工具进程”理解为“已确认任务正在执行”。

English documentation: [README.en.md](README.en.md)

## 支持的工具

| 工具 | 状态采集 | 继续任务 | 当前接入方式 |
| --- | --- | --- | --- |
| OpenAI Codex | 支持 | 支持 | 官方 Hook / 本地 CLI |
| Claude | 支持 | 支持 | 官方 Hook / 本地 CLI |
| Cursor | 支持 | 支持 | 官方 Hook |
| Antigravity IDE | 实验性 | 实验性 | Hook / 兼容适配层 |

工具名称和商标归各自权利人所有。本项目是独立的开源项目，与 OpenAI、Anthropic、Cursor 或 Antigravity IDE 官方无隶属或背书关系。

## 核心能力

- 每 2 秒刷新本机 AI 工具与活动任务状态
- 统一任务状态机、时间线和等待用户识别
- `SEND_NEXT`、托管 `RESUME_AND_SEND`、打开工具并复制任务
- 四类工具 Hook 的安全合并、卸载、离线 spool 与去重
- 飞书即时通知与成功任务汇总
- 本地 SQLite 存储，敏感命令体加密，诊断信息脱敏
- macOS LaunchAgent、Windows 登录任务和 Tauri 桌面打包
- API 默认仅监听 `127.0.0.1`

## 技术栈

- Rust 1.98、Tokio、Axum、SQLite
- Tauri 2
- React 19、TypeScript、Vite
- macOS Keychain / Windows Credential Manager

## 快速开始

### 环境要求

- Rust 1.98（仓库内 `rust-toolchain.toml` 会固定版本）
- Node.js 24 及 npm
- macOS 或 Windows

### 本地开发

```bash
git clone https://github.com/sgwood/rpa.git
cd rpa

# 安装前端依赖
cd apps/desktop
npm ci
cd ../..

# 启动本地节点
cargo run -p ai-rpa-node -- serve

# 在另一个终端启动桌面前端
cd apps/desktop
npm run dev
```

查看 CLI：

```bash
cargo run -p ai-rpa-node -- --help
```

### 验证

```bash
./scripts/verify.sh
```

Windows PowerShell：

```powershell
.\scripts\verify.ps1
```

也可以分别运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
```

### 打包

macOS：

```bash
npm --prefix apps/desktop run tauri build -- --bundles app,dmg
```

Windows（推荐 NSIS，不依赖已弃用的 VBSCRIPT 可选功能）：

```powershell
npm --prefix apps/desktop run tauri build -- --bundles nsis
```

企业 MSI 仍可通过 `--bundles msi` 构建，但构建机必须启用 Windows VBSCRIPT 可选功能。

macOS 安装脚本和 Windows PowerShell 安装脚本分别位于 `scripts/install-macos.sh` 与 `scripts/install-windows.ps1`。

## 当前验证边界

- macOS 已完成一次四工具真实投递 PoC，并完成签名应用的本机安装验证。
- Windows Native / WSL、真实飞书 Webhook、macOS 公证及 Windows 正式签名仍需在发布候选环境验收。
- 实时任务数来自工具 Hook 事件；仅检测到 IDE 进程时，只能说明工具已连接，不能证明其中存在执行中的任务。
- 项目不会根据窗口标题、提示词或聊天内容猜测任务结果。

完整状态参见[首版功能与验证状态](IMPLEMENTATION_STATUS.md)与[测试计划](TEST_PLAN.md)。

## 隐私与安全

本项目默认采用本地优先设计。原始桌面截图、时间戳、本机会话标识和数据库通过 `.gitignore` 排除；提示词等敏感命令内容不得写入日志或公开 Issue。

发现安全问题时，请不要创建公开 Issue。请按[安全策略](SECURITY.md)使用 GitHub 私密漏洞报告通道联系维护者。

## 文档

- [产品需求文档（PRD）](PRODUCT_PRD.md)
- [完整技术可行性分析](AI_TASK_RPA_FEASIBILITY.md)
- [技术实施方案](TECHNICAL_IMPLEMENTATION.md)
- [测试与验证计划](TEST_PLAN.md)
- [首版功能与验证状态](IMPLEMENTATION_STATUS.md)
- [macOS 四工具实测摘要](runs/20260829-model-probe/summary.md)

推荐阅读顺序：PRD → 技术实施方案 → 测试与验证计划 → 可行性分析与 PoC 证据。

## 参与贡献

欢迎提交问题、文档改进和代码贡献。开始前请阅读[贡献指南](CONTRIBUTING.md)与[行为准则](CODE_OF_CONDUCT.md)。

适合首次贡献的方向包括：

- Windows Native / WSL 验证和兼容性修复
- 新 AI 工具适配器
- Hook 契约样例与状态机测试
- 无障碍、国际化和移动端只读视图

## 许可证

本项目基于 [Apache License 2.0](LICENSE) 开源。
