#!/bin/zsh
set -euo pipefail

APP_EXECUTABLE="${1:-/Applications/AI 任务中控台.app/Contents/MacOS/ai-rpa-desktop}"
PURGE_DATA="${2:-}"
LAUNCH_AGENT="$HOME/Library/LaunchAgents/com.stargold.ai-rpa.plist"

if [[ -x "$APP_EXECUTABLE" ]]; then
  "$APP_EXECUTABLE" uninstall-hooks
else
  print -u2 "应用已不存在，无法自动清理 Hook；请从备份恢复或删除含 ' hook --provider ' 的本产品条目。"
fi
/bin/launchctl bootout "gui/$(/usr/bin/id -u)/com.stargold.ai-rpa" 2>/dev/null || true
/bin/rm -f "$LAUNCH_AGENT"

if [[ "$PURGE_DATA" == "--purge-data" ]]; then
  DATA_DIR="$HOME/Library/Application Support/com.stargold.ai-rpa"
  if [[ -d "$DATA_DIR" ]]; then
    /bin/rm -rf "$DATA_DIR"
    print "已删除本机任务数据库和诊断缓存：$DATA_DIR"
  fi
else
  print "已移除后台节点与 Hook；本机任务数据保留。传入 --purge-data 才会删除数据。"
fi
