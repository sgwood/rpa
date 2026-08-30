#!/bin/zsh
set -euo pipefail

APP_EXECUTABLE="${1:-/Applications/AI 任务中控台.app/Contents/MacOS/ai-rpa-desktop}"
if [[ ! -x "$APP_EXECUTABLE" ]]; then
  print -u2 "找不到可执行文件：$APP_EXECUTABLE"
  print -u2 "用法：$0 '/完整路径/ai-rpa-desktop'"
  exit 1
fi

LAUNCH_AGENT="$HOME/Library/LaunchAgents/com.stargold.ai-rpa.plist"
/bin/mkdir -p "$HOME/Library/LaunchAgents"
if [[ -f "$LAUNCH_AGENT" ]]; then
  /bin/cp "$LAUNCH_AGENT" "$LAUNCH_AGENT.ai-rpa.bak.$(/bin/date +%Y%m%d%H%M%S)"
fi
/usr/bin/plutil -create xml1 "$LAUNCH_AGENT"
/usr/bin/plutil -insert Label -string com.stargold.ai-rpa "$LAUNCH_AGENT"
/usr/bin/plutil -insert ProgramArguments -json "[\"$APP_EXECUTABLE\",\"serve\"]" "$LAUNCH_AGENT"
/usr/bin/plutil -insert RunAtLoad -bool true "$LAUNCH_AGENT"
/usr/bin/plutil -insert KeepAlive -bool true "$LAUNCH_AGENT"
/usr/bin/plutil -insert ProcessType -string Background "$LAUNCH_AGENT"
/bin/chmod 600 "$LAUNCH_AGENT"

/bin/launchctl bootout "gui/$(/usr/bin/id -u)/com.stargold.ai-rpa" 2>/dev/null || true
/bin/launchctl bootstrap "gui/$(/usr/bin/id -u)" "$LAUNCH_AGENT"
/bin/launchctl kickstart -k "gui/$(/usr/bin/id -u)/com.stargold.ai-rpa"
"$APP_EXECUTABLE" install-hooks

print "AI RPA 节点和四工具 Hook 已安装。"
print "健康检查：'$APP_EXECUTABLE' doctor"
