import type { Dashboard, Diagnostics, Task, TaskDetail } from "./types";

const API_BASE = import.meta.env.VITE_API_BASE ?? "http://127.0.0.1:3847/api";

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
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
};
