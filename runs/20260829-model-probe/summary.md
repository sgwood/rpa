# AI IDE 模型探测 RPA 实测记录

时间：2026-08-29 20:29～20:34（America/New_York）

探测问题：`当前使用大模型是哪个？`

## 结果

| 工具 | 投递方式 | 返回结果 | 完成判据 | 观测耗时 |
|---|---|---|---|---:|
| Codex Desktop | 当前 Codex 任务 | `gpt-5.6-sol`，推理强度 `xhigh` | rollout `turn_context` 元数据 | 当前任务内实时读取 |
| Claude Desktop | 新建本地任务“当前大模型选择” | Opus 5，模型 ID `claude-opus-5` | JSONL `stop_reason=end_turn` | 用户消息至回复约 4.1 秒 |
| Cursor Agents Window | 空白 Agent 任务，模式 Auto | Auto（Cursor Agent Router）；底层模型按任务动态路由 | transcript `turn_ended=success` | 不超过 10 秒 |
| Antigravity IDE | 当前工作区 Agent 面板 | Google Gemini 3.7 Flash；选择器为 Gemini 3.7 Flash Medium | transcript `PLANNER_RESPONSE=DONE` | 日志不足 1 秒；界面轮询不超过 13 秒 |

总结果：**4/4 投递成功，4/4 获得终态回复。**

## 证据

本机保留了四个工具的回复截图、终态 transcript 与会话 ID，用于审计和后续继续任务。原始证据可能包含企业源码、个人路径或会话信息，因此通过 `.gitignore` 排除，不上传远程仓库。

中央通知和远程仓库不应附带完整 transcript、Prompt 上下文、凭据或本机会话 ID。

## 技术判断

- macOS 单次跨 IDE 投递与回读已经实证可行。
- GUI 辅助功能适合作为触发与兜底，不适合作为唯一状态源；窗口坐标、弹窗和布局会变化。
- 生产实现应优先消费 Codex App Server/JSONL、Claude JSONL/Hooks、Cursor transcript/Hooks、Antigravity transcript/Hooks，并以会话 ID 建立命令队列。
- “继续发任务”需要会话租约、幂等键、队列 TTL 和单次执行上限，避免 RPA 与用户同时控制同一会话。
- Windows 仍需 Native 与 WSL 真机验证，不能把 macOS PoC 直接声明为 Windows 已验收。
- 工具自报的模型名称不是供应商计费审计证据；Codex、Claude 本次有明确会话元数据，Cursor、Antigravity 以界面与本地会话记录为证。

## 飞书通知

状态：已发送。

- 已完成一次受控实测，飞书 API 返回发送成功。
- 公开仓库不保存发送身份、接收人、消息 ID 或精确发送时间。
