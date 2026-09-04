# AI 任务中控台安装与使用操作手册

> 产品版本：0.2.0 Personal Sync  
> 文档版本：1.0  
> 适用平台：macOS、Windows、PC/手机浏览器  
> 接入工具：Codex、Claude、Cursor、Antigravity IDE

## 1. 手册用途

本手册面向三类人员：

| 角色 | 主要操作 |
| --- | --- |
| 普通使用者 | 安装桌面端、检查本机 AI 工具、查看任务、处理等待事项 |
| 客户管理员 | 登录中央控制台、添加和撤销设备、跨设备查看任务、远程续派任务 |
| 系统运维人员 | 维护 ctyun 中央服务、PostgreSQL、HTTPS/WSS、备份和运行状态 |

产品由两个入口组成：

1. **本机桌面端**：安装在每台 Mac 或 Windows 电脑上，负责本机 Hook、任务采集、本机任务查看、飞书通知和中央配对；
2. **中央 Web 控制台**：汇总多台电脑的数据，供 PC 或手机浏览器统一管理。

当前 0.2.0 原生桌面安装包默认进入本机模式。多设备汇总请使用中央 Web 控制台，不能把本机总览误认为全部设备总览。

当前测试环境：

- 中央控制台：<https://ai-rpa-test.qiyefly.cn/>
- 开源仓库：<https://github.com/sgwood/rpa>
- 中央环境性质：ctyun 共享测试主机上的 RC 环境，不是正式生产环境

## 2. 工作原理

```text
Mac/Windows A ─┐
Mac/Windows B ─┼─ 出站 HTTPS/WSS 443 ─> ctyun 中央服务 ─> PostgreSQL
Mac/Windows C ─┘                              │
                                              ├─ PC 浏览器
                                              └─ 手机浏览器
```

- 每台电脑只主动连接中央服务，不需要开放入站端口；
- Hook 事件先写入本机 SQLite，再同步到中央服务；
- 断网期间事件保存在本机，网络恢复后自动补传；
- 远程任务只包含结构化操作和用户消息，不允许下发任意 Shell；
- 设备令牌保存在 macOS Keychain 或 Windows Credential Manager；
- 中央远程任务正文使用独立密钥加密保存。

## 3. 安装前准备

### 3.1 终端要求

- macOS 或 Windows 10/11；
- 能访问中央服务的 TCP 443；
- 至少安装一个受支持的 AI 工具；
- 操作系统时间应保持自动同步；
- 当前用户应能修改自己的 AI 工具配置目录；
- 后台运行安装需要当前用户创建 LaunchAgent 或登录计划任务的权限。

### 3.2 网络放行

客户端至少需要访问：

| 目的地址 | 端口 | 用途 |
| --- | ---: | --- |
| `ai-rpa-test.qiyefly.cn` | 443 | 中央 Web、设备注册、WSS 同步、远程任务 |
| `open.feishu.cn` 或 `open.larksuite.com` | 443 | 可选的飞书/Lark 自定义机器人通知 |

本机节点只监听 `127.0.0.1:3847`。禁止把该端口改为 `0.0.0.0` 或映射到公网。

### 3.3 当前发布边界

- macOS RC 安装包尚未使用 Developer ID 签名和公证；
- Windows RC 安装包尚未使用正式代码签名证书；
- 当前 CI 产物保留 7 天；正式对外发布应改用 GitHub Releases；
- Windows Native、WSL、睡眠唤醒和 72 小时长稳仍需正式验收；
- Antigravity 接入属于实验性能力。

## 4. 获取安装包

### 4.1 从 GitHub Actions 获取 RC 包

1. 打开仓库的 **Actions** 页面；
2. 选择最新一次成功的 **Verify and package**；
3. 在页面底部下载对应产物：
   - `ai-rpa-macos-15`：macOS `.app` 和 `.dmg`；
   - `ai-rpa-windows-2025`：Windows NSIS 安装包；
   - `ai-rpa-central-linux-x64`：运维使用的 Linux 中央服务产物；
4. 解压下载的 ZIP；
5. 安装前核对发布方提供的 SHA-256，不要运行来源不明的安装包。

### 4.2 从源码构建

需要 Rust 1.98、Node.js 24 和 npm：

```bash
git clone https://github.com/sgwood/rpa.git
cd rpa
npm --prefix apps/desktop ci
```

macOS 打包：

```bash
npm --prefix apps/desktop run tauri build -- --bundles app,dmg
```

Windows PowerShell 打包：

```powershell
npm --prefix apps/desktop run tauri build -- --bundles nsis
```

构建前建议先运行：

```bash
./scripts/verify.sh
```

Windows 使用：

```powershell
.\scripts\verify.ps1
```

## 5. macOS 安装

### 5.1 安装桌面应用

1. 双击下载的 DMG；
2. 将“AI 任务中控台”拖入“应用程序”；
3. 从“应用程序”打开；
4. 如果 RC 包被系统拦截，只能在确认下载来源和校验值后，通过 Finder 右键选择“打开”，或按组织的安全流程处理；
5. 正式客户环境应使用已签名、公证的安装包。

仅打开应用时，桌面端会在应用运行期间启动内嵌本机节点。若要在关闭窗口后继续采集任务，需要安装后台节点。

### 5.2 安装后台节点和四工具 Hook

先退出正在运行的“AI 任务中控台”，进入仓库目录执行：

```bash
./scripts/install-macos.sh
```

默认可执行文件为：

```text
/Applications/AI 任务中控台.app/Contents/MacOS/ai-rpa-desktop
```

如果应用安装在其他位置：

```bash
./scripts/install-macos.sh "/完整路径/AI 任务中控台.app/Contents/MacOS/ai-rpa-desktop"
```

脚本会：

- 创建 `~/Library/LaunchAgents/com.stargold.ai-rpa.plist`；
- 登录时自动启动本机节点；
- 节点退出后自动重启；
- 合并四个工具的 Hook 配置；
- 修改 Hook 文件前创建时间戳备份。

检查后台节点：

```bash
launchctl print "gui/$(id -u)/com.stargold.ai-rpa"
curl --fail http://127.0.0.1:3847/health
```

运行只读诊断：

```bash
'/Applications/AI 任务中控台.app/Contents/MacOS/ai-rpa-desktop' doctor
```

## 6. Windows 安装

### 6.1 安装桌面应用

1. 解压 `ai-rpa-windows-2025`；
2. 运行 NSIS 安装程序；
3. 按向导完成安装；
4. 从开始菜单打开“AI 任务中控台”；
5. RC 包未正式签名。如果 SmartScreen 或企业安全软件拦截，应先核对来源、哈希和组织策略，不建议在正式客户环境绕过安全策略。

### 6.2 安装后台节点和四工具 Hook

先退出桌面应用，以当前登录用户打开 PowerShell。进入仓库目录，将安装后的实际可执行文件路径传给脚本：

```powershell
$exe = "C:\完整路径\ai-rpa-desktop.exe"
.\scripts\install-windows.ps1 -Executable $exe
```

脚本会：

- 创建名为 `AI RPA Node` 的登录计划任务；
- 以当前普通用户权限运行；
- 失败后最多自动重启 5 次；
- 安装四个工具的 Hook。

检查计划任务：

```powershell
Get-ScheduledTask -TaskName "AI RPA Node"
Get-ScheduledTaskInfo -TaskName "AI RPA Node"
```

检查本机节点：

```powershell
Invoke-RestMethod http://127.0.0.1:3847/health
& $exe doctor
```

不要同时手工启动多个 `serve` 实例；同一逻辑环境只应有一个进程监听 `127.0.0.1:3847`。

## 7. 首次启动与本机自检

1. 打开桌面应用；
2. 左侧进入 **本机接入**；
3. 点击 **运行只读自检**；
4. 确认页面显示：
   - 节点在线；
   - SQLite WAL 为通过；
   - 本地 API 仅绑定 loopback；
   - 隐私检查通过；
5. 检查 Codex、Claude、Cursor、Antigravity 卡片；
6. 点击 **安装 / 修复 Hook**；
7. 重新启动已经打开的 AI 工具，使新 Hook 生效。

工具状态解释：

| 状态 | 含义 |
| --- | --- |
| 未安装 | 未发现可执行文件，也未发现运行进程 |
| 已安装未运行 | 发现工具，但当前没有运行 |
| 运行中 | 检测到工具进程；不代表有任务正在执行 |
| Hook 未配置 | 尚未安装本产品 Hook |
| Hook 已配置 | 配置存在，但还没有收到真实事件 |
| Hook 健康 | 已收到该工具的真实事件 |
| 配置无效 | 工具配置文件不是合法 JSON；产品拒绝覆盖 |

### 7.1 Hook 配置位置

| 工具 | 当前用户配置文件 |
| --- | --- |
| Codex | `~/.codex/hooks.json` |
| Claude | `~/.claude/settings.json` |
| Cursor | `~/.cursor/hooks.json` |
| Antigravity | `~/.gemini/config/hooks.json` |

安装程序只管理带本产品标记的条目。重复安装是幂等操作，不会故意删除其他 Hook。若现有文件不是合法 JSON，安装会停止并提示修复文件，不会直接覆盖。

### 7.2 验证四工具事件

对每个已安装工具分别执行一个无敏感信息的短任务，例如：

```text
请只回复：AI RPA Hook 测试完成
```

回到 **本机接入** 页面：

1. 点击刷新或等待最多 30 秒；
2. 确认对应工具从“已配置”变为“健康”；
3. 在 **任务** 页面确认出现新的会话；
4. 打开任务详情，确认时间线包含开始、停止或结果事件。

## 8. 登录中央控制台

使用 PC 或手机浏览器打开：

<https://ai-rpa-test.qiyefly.cn/>

当前 Personal Sync 采用单管理员令牌：

1. 向系统运维人员获取管理员令牌；
2. 在登录页输入令牌；
3. 点击 **登录控制台**；
4. 登录后应看到总览、任务、等待处理和设备页面。

ctyun 运维人员可以在服务器上读取当前令牌：

```bash
ssh ctyun 'cat /etc/ai-rpa/admin-token'
```

令牌属于敏感凭据：

- 不要通过公开聊天、Issue、截图或日志传递；
- 不要把当前单管理员令牌发给多个互不信任的客户；
- 离开共享电脑前点击 **退出登录**；
- 多客户正式版必须升级为独立账号、租户隔离和 RBAC。

## 9. 将第一台电脑接入中央控制台

### 9.1 生成一次性配对码

在中央控制台：

1. 进入 **设备**；
2. 点击 **添加设备**；
3. 页面显示 10 位一次性配对码；
4. 点击 **复制配对码**；
5. 配对码 15 分钟内有效，只能使用一次。

### 9.2 在目标电脑完成配对

在目标电脑的桌面应用：

1. 进入 **设备**；
2. 在“中央控制台地址”输入：

   ```text
   https://ai-rpa-test.qiyefly.cn
   ```

3. 输入刚生成的配对码；
4. 输入便于识别的设备名称，例如“开发 Mac”“财务 Windows”；
5. 点击 **连接中央控制台**；
6. 页面显示“已接入 ctyun”；
7. 返回中央控制台，设备通常会在数秒内显示在线。

CLI 配对方式：

```bash
ai-rpa connect \
  --server https://ai-rpa-test.qiyefly.cn \
  --code A1B2C3D4E5 \
  --alias "开发 Mac"
```

macOS 安装包内执行示例：

```bash
'/Applications/AI 任务中控台.app/Contents/MacOS/ai-rpa-desktop' connect \
  --server https://ai-rpa-test.qiyefly.cn \
  --code A1B2C3D4E5 \
  --alias "开发 Mac"
```

Windows PowerShell 示例：

```powershell
& $exe connect `
  --server https://ai-rpa-test.qiyefly.cn `
  --code A1B2C3D4E5 `
  --alias "开发 Windows"
```

### 9.3 接入更多电脑

每台电脑必须单独生成新的配对码。重复以下步骤：

1. 中央控制台生成新码；
2. 在一台目标电脑使用；
3. 等待设备在线；
4. 修改设备名称；
5. 再为下一台电脑生成新码。

不要把一个配对码同时发给多台电脑。

## 10. 设备管理

中央控制台的 **设备** 页面显示：

- 设备名称；
- 在线、离线或已撤销；
- macOS/Windows 与体系结构；
- 设备 ID；
- 最后心跳时间；
- Node 版本。

### 10.1 修改名称

1. 点击设备卡片中的 **修改名称**；
2. 输入新的业务名称；
3. 保存；
4. 名称建议包含用途和平台，例如“研发一组 Mac-01”。

### 10.2 断开与撤销的区别

- **本机断开中央连接**：清除本机中央地址和设备令牌；中央仍保留历史设备记录；
- **中央撤销设备**：服务器立即让原令牌失效；该设备必须重新生成配对码才能接入。

设备丢失、人员离职或令牌疑似泄露时，应使用中央控制台的 **撤销设备**，不能只在本机断开。

## 11. 查看任务

### 11.1 总览

中央总览展示：

- 在线设备数量；
- 已连接和已监控的 AI 工具；
- 当前执行中任务数；
- 等待人工任务数；
- 最近任务；
- 失败、等待人工和状态未知任务。

“工具运行中”只说明进程存在；只有收到 Hook 事件，系统才会显示活动任务。

### 11.2 任务列表

进入 **任务** 页面，可以按以下条件查找：

- 任务名称、项目、会话或工作区关键词；
- 状态；
- AI 工具；
- 设备 ID；
- 控制模式；
- 时间范围。

中央列表中的每条任务都带设备信息，用于区分不同电脑上的同名会话。

### 11.3 等待处理

进入 **等待处理**，优先处理：

- 权限确认；
- 登录或身份验证；
- AI 要求补充信息；
- 执行失败；
- 状态未知。

系统不会自动批准删除、发布、支付、密钥访问或绕过沙箱等敏感动作，这些操作仍应在原 AI 工具中由用户确认。

### 11.4 任务详情

任务详情包括：

- AI 工具、设备、项目和工作区；
- 会话 ID；
- 当前状态与持续时间；
- 观察模式或托管模式；
- 当前证据等级和要求等级；
- 完整事件时间线；
- 后续任务命令及其状态；
- 审计记录。

## 12. 正确理解任务完成状态

| 状态 | 含义 | 建议动作 |
| --- | --- | --- |
| 执行中 `RUNNING` | 已开始且未收到终态 | 继续等待 |
| 等待人工 `WAITING_USER` | 需要权限、输入或登录 | 回原工具处理 |
| 成功 `SUCCEEDED` | 已达到该任务要求的完成证据 | 查看证据后关闭 |
| 失败 `FAILED` | 工具、命令或验收明确失败 | 查看错误并续派修复 |
| 已取消 `CANCELLED` | 用户或工具取消 | 确认是否重新执行 |
| 状态未知 `UNKNOWN` | 事件不足或连接中断 | 检查 Hook、节点和原会话 |

证据等级：

| 等级 | 代表证据 |
| --- | --- |
| E0 | 只检测到进程或会话，不能证明任务执行或完成 |
| E1 | 收到正式生命周期事件，例如 Stop/SessionEnd |
| E2 | 收到结构化结果、托管命令结果或明确退出状态 |
| E3 | 自动测试、产物、部署读回或业务验收通过 |

必须同时查看“当前证据/要求证据”。AI 停止输出、IDE 空闲或模型回复“完成了”都不能单独作为业务完成证明。

## 13. 继续下发任务

1. 打开任务详情；
2. 点击 **继续任务**；
3. 选择系统实际提供的执行方式；
4. 选择有效期：30 分钟、2 小时、8 小时或 24 小时；
5. 输入下一步要求；
6. 点击 **加入队列**；
7. 返回任务详情查看命令状态和回执。

执行方式：

| 方式 | 含义 | 使用边界 |
| --- | --- | --- |
| `SEND_NEXT` | 当前回合停止后发送下一条任务 | 最常用；不打断活动回合 |
| `RESUME_AND_SEND` | 恢复托管会话并发送 | 仅正式登记的托管会话 |
| `OPEN_AND_PREFILL` | 打开原工具并把内容复制到剪贴板 | 仅本机模式；不会自动粘贴或回车 |

中央控制台不会显示 `OPEN_AND_PREFILL`，因为中央服务器不能安全操作远程电脑的 GUI。

命令状态可能经历：

```text
已创建 -> 已排队 -> 已领取 -> 已投递 -> 已接受 -> 已完成
```

也可能显示重试等待、投递未知、已过期、失败或取消。看到“已排队”并不代表 AI 已完成任务。

建议的任务描述：

```text
继续运行完整测试；如果失败，请定位根因并修复后再次验证。
完成条件：测试全部通过，输出失败数和最终提交号。
```

不要在任务正文中放入密码、私钥、支付信息或长期访问令牌。

## 14. 飞书通知

飞书 Webhook 当前按本机节点配置。若多台电脑都需要通知，应在每台电脑分别配置，或在后续版本部署中央通知服务。

### 14.1 创建机器人

在目标飞书群中创建自定义机器人，并取得官方 HTTPS Webhook。产品只接受：

- `https://open.feishu.cn/open-apis/bot/v2/hook/...`
- `https://open.larksuite.com/open-apis/bot/v2/hook/...`

### 14.2 保存配置

1. 打开桌面端 **本机接入**；
2. 在“飞书通知”输入 Webhook；
3. 点击 **保存到系统凭据库**；
4. 点击 **立即处理通知队列**；
5. 查看页面显示的发送和失败数量；
6. 到飞书群确认收到卡片。

通知策略：

- 失败和等待人工：即时通知；
- 成功：进入约 5 分钟汇总窗口；
- 发送失败：指数退避，最多 8 次；
- 通知只包含脱敏摘要，不包含提示词、源代码和远程任务正文。

## 15. 诊断与问题反馈

### 15.1 图形界面诊断

1. 进入 **本机接入**；
2. 点击 **运行只读自检**；
3. 查看节点、工具、Hook、SQLite、隐私和飞书状态；
4. 点击 **导出脱敏诊断包**；
5. 将 JSON 诊断包交给受信任的支持人员。

诊断包声明不包含：

- 密钥和 Webhook；
- 提示词和会话全文；
- 源代码；
- 截图。

### 15.2 CLI 诊断

```bash
ai-rpa doctor
ai-rpa discover
ai-rpa central-status
```

导出诊断：

```bash
ai-rpa export-diagnostics --output ./ai-rpa-diagnostics.json
```

macOS 输出文件权限会设置为 `0600`。

## 16. 常见故障处理

### 16.1 桌面端显示“本机节点暂不可用”

检查：

```bash
curl http://127.0.0.1:3847/health
```

macOS：

```bash
launchctl print "gui/$(id -u)/com.stargold.ai-rpa"
```

Windows：

```powershell
Get-ScheduledTaskInfo -TaskName "AI RPA Node"
```

常见原因：

- 后台节点没有安装或没有启动；
- `3847` 被另一个进程占用；
- 同时手工启动了两个 `serve`；
- 系统凭据库不可用，导致本地加密密钥读取失败。

### 16.2 工具显示运行中，但没有任务

这是允许的。进程检测只证明工具打开。依次检查：

1. Hook 是否显示“已配置”或“健康”；
2. 是否在安装 Hook 后重启了 AI 工具；
3. 是否完成过一次真实开始/停止事件；
4. 工具配置 JSON 是否有效；
5. 本机节点在事件发生时是否运行。

节点短时离线时，Hook 会把事件写入本机 spool；节点恢复后自动导入。

### 16.3 配对码失败

检查：

- 配对码是否超过 15 分钟；
- 是否已经被另一台设备使用；
- 是否完整输入 10 位字符；
- 中央地址是否为 HTTPS；
- 设备是否能访问中央服务 443；
- 该设备是否曾被中央管理员撤销。

失败后应重新生成新码，不要重复传播旧码。

### 16.4 中央设备显示离线

在目标电脑检查：

```bash
ai-rpa central-status
curl http://127.0.0.1:3847/health
```

然后检查：

- 后台节点是否在运行；
- 网络、代理或防火墙是否允许 HTTPS/WSS；
- 系统时间是否正确；
- Keychain/Credential Manager 中的设备令牌是否仍存在；
- 中央控制台是否撤销了设备。

### 16.5 任务长期显示状态未知

状态未知不能手工改成成功。检查：

- 原 AI 工具中任务是否真的结束；
- 是否只收到了 Stop 而没有结果事件；
- Hook 是否健康；
- 会话 ID 是否变化；
- 网络恢复后事件是否补传；
- 任务要求的证据等级是否高于现有证据。

必要时在原工具中重新运行验收，再让系统收到新的结果事件。

### 16.6 飞书通知失败

检查：

- Webhook 是否来自允许的官方域名；
- 机器人是否仍在目标群中；
- 当前电脑是否能访问飞书；
- 群机器人安全策略是否拒绝消息；
- 待发送 Outbox 和失败次数。

修复后点击 **立即处理通知队列**。

### 16.7 中央控制台打不开

客户端先检查：

```bash
curl --fail https://ai-rpa-test.qiyefly.cn/health
```

运维人员检查：

```bash
ssh ctyun 'systemctl is-active ai-rpa-server.service'
ssh ctyun 'docker inspect --format "{{.State.Health.Status}}" ai-rpa-postgres-1'
ssh ctyun 'curl --fail http://127.0.0.1:18180/health'
```

如果健康检查通过但浏览器仍失败，再检查 DNS、TLS 证书、Nginx 和客户端代理。

## 17. 更新

### 17.1 更新客户端

1. 确认新版本来源和校验值；
2. 退出桌面应用；
3. 保留本机数据目录和系统凭据；
4. 安装新版本覆盖旧版本；
5. 重新运行后台安装脚本，确保计划任务引用的是新可执行文件；
6. 打开 **本机接入**，执行 **安装 / 修复 Hook**；
7. 运行自检和一个短 Hook 测试；
8. 确认中央设备显示新 Node 版本。

### 17.2 更新中央服务

中央服务应采用版本目录和符号链接滚动发布。更新后至少验证：

```bash
curl --fail https://ai-rpa-test.qiyefly.cn/health
ssh ctyun 'systemctl is-active ai-rpa-server.service'
ssh ctyun 'docker inspect --format "{{.State.Health.Status}}" ai-rpa-postgres-1'
```

更新中央二进制不应重新生成管理员令牌、数据加密密钥或 PostgreSQL 数据卷。

## 18. 卸载

卸载前先在中央控制台撤销该设备，避免遗留有效设备令牌。

### 18.1 macOS

保留任务数据：

```bash
./scripts/uninstall-macos.sh
```

指定非默认应用路径：

```bash
./scripts/uninstall-macos.sh "/完整路径/ai-rpa-desktop"
```

删除应用程序本体后清空废纸篓即可。

只有在确认不再需要本机任务历史和诊断缓存时，才使用：

```bash
./scripts/uninstall-macos.sh "/Applications/AI 任务中控台.app/Contents/MacOS/ai-rpa-desktop" --purge-data
```

`--purge-data` 会删除本机数据库，不能通过产品恢复。

### 18.2 Windows

保留任务数据：

```powershell
.\scripts\uninstall-windows.ps1 -Executable $exe
```

确认不需要本机任务数据后：

```powershell
.\scripts\uninstall-windows.ps1 -Executable $exe -PurgeData
```

然后从 Windows“已安装的应用”卸载“AI 任务中控台”。

### 18.3 默认数据目录

| 平台 | 数据目录 |
| --- | --- |
| macOS | `~/Library/Application Support/com.stargold.ai-rpa` |
| Windows | `%LOCALAPPDATA%\stargold\ai-rpa\data` |

数据目录包含本机 SQLite、离线事件和诊断缓存，应按敏感业务数据保护。

## 19. CLI 速查

```text
ai-rpa serve
ai-rpa doctor
ai-rpa discover
ai-rpa install-hooks
ai-rpa uninstall-hooks
ai-rpa export-diagnostics --output <文件>
ai-rpa connect --server <HTTPS地址> --code <配对码> --alias <设备名>
ai-rpa disconnect
ai-rpa central-status
ai-rpa command --task-id <UUID> --action SEND_NEXT --message <任务> --ttl-seconds 7200
```

单独安装某个工具 Hook：

```bash
ai-rpa install-hooks --provider codex
ai-rpa install-hooks --provider claude
ai-rpa install-hooks --provider cursor
ai-rpa install-hooks --provider antigravity
```

单独移除：

```bash
ai-rpa uninstall-hooks --provider cursor
```

普通用户不应手工调用 `hook` 子命令；它由 AI 工具的生命周期 Hook 自动调用。

## 20. 运维速查

### 20.1 服务状态

```bash
ssh ctyun 'systemctl status ai-rpa-server.service --no-pager'
ssh ctyun 'journalctl -u ai-rpa-server.service -n 100 --no-pager'
ssh ctyun 'docker ps --filter name=ai-rpa'
```

### 20.2 健康检查

```bash
curl --fail https://ai-rpa-test.qiyefly.cn/health
```

预期返回：

```json
{"mode":"CENTRAL","status":"ok","version":"0.2.0"}
```

### 20.3 当前服务器文件

```text
/opt/ai-rpa/bin/ai-rpa-server
/opt/ai-rpa/ui
/opt/ai-rpa/deploy
/etc/ai-rpa/server.env
/etc/ai-rpa/postgres.env
/etc/ai-rpa/admin-token
/usr/local/nginx/conf/vhost/ai-rpa-test.qiyefly.cn.conf
```

密钥文件不得复制到仓库、日志、工单或诊断包。

### 20.4 数据库备份

在受控终端执行，将备份保存到当前电脑：

```bash
ssh ctyun 'docker exec ai-rpa-postgres-1 pg_dump -U ai_rpa -Fc ai_rpa' \
  > "ai-rpa-$(date +%Y%m%d-%H%M%S).dump"
```

备份文件可能包含设备和任务数据，应加密保存并限制访问。恢复操作会改写数据，必须使用单独恢复演练流程，不要直接在生产库试验。

## 21. 安装验收清单

每台电脑完成以下检查：

- [ ] 桌面应用可以打开；
- [ ] `127.0.0.1:3847/health` 返回成功；
- [ ] 后台 LaunchAgent 或计划任务正常；
- [ ] 四工具配置未破坏原有 JSON；
- [ ] 至少一个工具收到真实 Hook 事件；
- [ ] 本机任务列表出现测试任务；
- [ ] 中央配对成功，设备显示在线；
- [ ] 断网恢复后事件能够补传；
- [ ] 中央控制台能按设备查看任务；
- [ ] `SEND_NEXT` 只投递一次；
- [ ] 被撤销设备无法继续同步；
- [ ] 飞书通知配置时已完成真实消息验证；
- [ ] 脱敏诊断包不包含提示词、源代码和密钥。

正式上线还需完成：

- [ ] 两台不同公网的 Mac/Windows 实机闭环；
- [ ] Windows Native/WSL 兼容性验证；
- [ ] macOS 公证和 Windows 代码签名；
- [ ] ctyun 独立 ECS + RDS + ELB/WAF；
- [ ] PostgreSQL 备份恢复演练；
- [ ] 72 小时长稳、休眠唤醒、数据库短时不可用测试；
- [ ] 多客户场景的账号、租户隔离和 RBAC。

## 22. 安全原则

1. 不以“检测到进程”作为任务完成证据；
2. 不从窗口标题、聊天正文或提示词猜测任务状态；
3. 不开放本机节点公网端口；
4. 不允许中央控制台下发任意 Shell；
5. 不自动批准删除、支付、发布或权限提升；
6. 不把令牌、Webhook、提示词和源代码写入通知或诊断包；
7. 设备丢失或人员离职时立即在中央撤销设备；
8. 对外提供服务前必须完成签名、公证、租户隔离和正式运行验收。
