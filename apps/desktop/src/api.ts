import type {
  CentralStatus,
  CodexProject,
  CodexRunResponse,
  CodexStatus,
  Dashboard,
  Device,
  Diagnostics,
  SessionInfo,
  Task,
  TaskDetail,
} from "./types";

const isDesktopShell = window.location.protocol === "tauri:"
  || window.location.hostname === "tauri.localhost"
  || window.location.port === "1420";
const API_BASE = import.meta.env.VITE_API_BASE
  ?? (isDesktopShell ? "http://127.0.0.1:3847/api" : "/api");
const TOKEN_KEY = "ai-rpa-central-token";

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const token = window.localStorage.getItem(TOKEN_KEY);
  const response = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...init?.headers,
    },
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new ApiError(body?.error?.message ?? `请求失败（HTTP ${response.status}）`, response.status);
  }
  return body as T;
}

export const api = {
  session: () => request<SessionInfo>("/session"),
  login: async (token: string) => {
    window.localStorage.setItem(TOKEN_KEY, token.trim());
    try {
      return await request<SessionInfo>("/session");
    } catch (error) {
      window.localStorage.removeItem(TOKEN_KEY);
      throw error;
    }
  },
  logout: () => window.localStorage.removeItem(TOKEN_KEY),
  dashboard: () => request<Dashboard>("/dashboard"),
  tasks: (query = "") => request<{ items: Task[] }>(`/tasks${query ? `?${query}` : ""}`),
  task: (id: string) => request<TaskDetail>(`/tasks/${id}`),
  openTask: (id: string) => request<{ opened: boolean }>(`/tasks/${id}/open`, { method: "POST", body: "{}" }),
  diagnostics: () => request<Diagnostics>("/diagnostics"),
  exportDiagnostics: () => request<Record<string, unknown>>("/diagnostics/export"),
  installHooks: () => request<{ items: Array<{ provider: string; changed: boolean; path: string }> }>("/hooks/install", { method: "POST", body: "{}" }),
  uninstallHooks: () => request<{ items: Array<{ provider: string; changed: boolean; path: string }> }>("/hooks/uninstall", { method: "POST", body: "{}" }),
  command: (
    taskId: string,
    input: { action: string; message: string; ttlSeconds: number },
  ) =>
    request<{ command: unknown }>(`/tasks/${taskId}/commands`, {
      method: "POST",
      body: JSON.stringify({ ...input, createdBy: "desktop-user" }),
    }),
  flushNotifications: () =>
    request<{ configured: boolean; attempted: number; sent: number; failed: number }>(
      "/notifications/flush",
      { method: "POST", body: "{}" },
    ),
  setFeishu: (webhook: string) =>
    request<{ configured: boolean }>("/settings/feishu", {
      method: "POST",
      body: JSON.stringify({ webhook }),
    }),
  devices: () => request<{ items: Device[] }>("/devices"),
  createEnrollmentCode: () =>
    request<{ code: string; expiresAt: string }>("/devices/enrollment-codes", {
      method: "POST",
      body: "{}",
    }),
  renameDevice: (id: string, alias: string) =>
    request<{ updated: boolean }>(`/devices/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify({ alias }),
    }),
  revokeDevice: (id: string) =>
    request<{ revoked: boolean }>(`/devices/${encodeURIComponent(id)}/revoke`, {
      method: "POST",
      body: "{}",
    }),
  centralStatus: () => request<CentralStatus>("/central/status"),
  connectCentral: (input: { serverUrl: string; enrollmentCode: string; deviceAlias?: string }) =>
    request<{ configured: boolean; deviceId: string }>("/central/connect", {
      method: "POST",
      body: JSON.stringify(input),
    }),
  disconnectCentral: () =>
    request<{ configured: boolean }>("/central/disconnect", { method: "POST", body: "{}" }),
  codexStatus: () => request<CodexStatus>("/codex/status"),
  codexProjects: () => request<{ items: CodexProject[] }>("/codex/projects"),
  registerCodexProject: (input: { name: string; path: string }) =>
    request<{ project: CodexProject }>("/codex/projects", {
      method: "POST",
      body: JSON.stringify(input),
    }),
  deleteCodexProject: (id: string) =>
    request<{ deleted: boolean }>(`/codex/projects/${encodeURIComponent(id)}`, { method: "DELETE" }),
  codexTasks: (projectIds: string[]) => {
    const query = projectIds.length
      ? `?projectIds=${encodeURIComponent(projectIds.join(","))}`
      : "";
    return request<{ items: Task[] }>(`/codex/tasks${query}`);
  },
  startCodexRuns: (input: {
    title: string;
    prompt: string;
    projectIds: string[];
    timeoutSeconds: 3600 | 7200 | 10800;
    sandbox: "read-only" | "workspace-write";
  }) => request<CodexRunResponse>("/codex/runs", {
    method: "POST",
    body: JSON.stringify(input),
  }),
};
