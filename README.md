# AI 工具任务监控与续跑 RPA

面向 macOS 与 Windows 的 AI Agent 控制面可行性研究，目标是统一采集、汇总并继续执行以下工具中的任务：

- OpenAI Codex
- Claude
- Cursor
- Antigravity IDE

## 当前结论

综合技术可行性为高。稳定实现应优先使用 Hook、JSONL、App Server、CLI、SDK 或 Cloud API，GUI 辅助功能自动化仅作为触发和兜底。

macOS 已完成一次四工具真实投递 PoC，四个工具均成功返回终态。Windows 的接口与架构分析已完成，但仍需分别进行 Windows Native 与 WSL 真机验收。

## 文档

- [完整技术可行性分析](AI_TASK_RPA_FEASIBILITY.md)
- [macOS 四工具实测摘要](runs/20260829-model-probe/summary.md)

## 隐私与证据

原始桌面截图、时间戳和本机会话标识只保留在执行机器上，并通过 `.gitignore` 排除，避免上传企业源码画面、个人路径或不必要的会话信息。
