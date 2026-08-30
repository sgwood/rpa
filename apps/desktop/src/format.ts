import type { Provider, TaskState } from "./types";

export const stateMeta: Record<TaskState, { label: string; tone: string }> = {
  RUNNING: { label: "运行中", tone: "blue" },
  WAITING_USER: { label: "等待人工", tone: "orange" },
  FAILED: { label: "失败", tone: "red" },
  SUCCEEDED: { label: "已成功", tone: "green" },
  CANCELLED: { label: "已取消", tone: "gray" },
  UNKNOWN: { label: "状态不明", tone: "gray" },
};

export const providerLabel: Record<Provider, string> = {
  CODEX: "Codex",
  CLAUDE: "Claude",
  CURSOR: "Cursor",
  ANTIGRAVITY: "Antigravity",
};

const eventLabels: Record<string, string> = {
  SESSION_STARTED: "会话开始",
  TURN_STARTED: "回合开始",
  HEARTBEAT: "执行心跳",
  WAITING_USER: "等待人工",
  RESULT: "产生结果",
  FAILED: "执行失败",
  TURN_STOPPED: "回合停止",
  SESSION_ENDED: "会话结束",
  CANCELLED: "已取消",
};

export function eventLabel(value: string): string {
  return eventLabels[value] ?? value.replaceAll("_", " ");
}

export function installStateLabel(value: string): string {
  const labels: Record<string, string> = {
    NOT_INSTALLED: "未安装",
    INSTALLED_NOT_RUNNING: "已安装，未运行",
    RUNNING: "正在运行",
  };
  return labels[value] ?? value.replaceAll("_", " ");
}

export function adapterMessageLabel(value: string): string {
  const labels: Record<string, string> = {
    "process detected; hook health requires an event": "已检测到进程，收到首个 Hook 事件后可确认链路健康",
    "no active process detected": "当前未检测到运行中的进程",
  };
  return labels[value] ?? value;
}

export function diagnosticLabel(value: string): string {
  const labels: Record<string, string> = {
    "SQLite WAL": "本地事件库",
    "Local API": "本机接口",
    Feishu: "飞书通知",
    Privacy: "隐私保护",
    PASS: "通过",
    NOT_CONFIGURED: "未配置",
    "database opened and schema is readable": "数据库已打开，结构可正常读取",
    "bound to loopback only": "仅监听本机回环地址",
    "webhook credential found in environment or OS credential store": "已在系统凭据库中找到 Webhook",
    "configure webhook in OS credential store before sending notifications": "发送通知前需将 Webhook 保存到系统凭据库",
    "diagnostic output excludes prompts, transcripts, secrets and screenshots": "诊断信息不包含提示词、对话全文、密钥或截图",
  };
  return labels[value] ?? value;
}

export function evidenceSummaryLabel(value?: string): string {
  if (!value) return "没有可展示的脱敏摘要";
  const match = value.match(/^([A-Z_]+) event with (E[0-3]) evidence$/);
  return match ? `${eventLabel(match[1])}，证据等级 ${match[2]}` : value;
}

export function formatDuration(milliseconds?: number): string {
  if (milliseconds === undefined || milliseconds === null) return "未知";
  const seconds = Math.max(0, Math.round(milliseconds / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} 分 ${seconds % 60} 秒`;
  return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分`;
}

export function timeAgo(input?: string): string {
  if (!input) return "尚无事件";
  const seconds = Math.max(0, Math.round((Date.now() - new Date(input).getTime()) / 1000));
  if (seconds < 60) return `${seconds} 秒前`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} 小时前`;
  return new Date(input).toLocaleString("zh-CN");
}
