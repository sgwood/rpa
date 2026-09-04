import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import App from "./App";

const dashboard = {
  counts: { RUNNING: 1, WAITING_USER: 0, FAILED: 0, UNKNOWN: 0 },
  completionRate24h: 1,
  p95DurationMs: 2_000,
  devices: [{
    id: "dev-1",
    os: "macos",
    arch: "aarch64",
    hostname: "test-mac",
    logicalEnvironment: "macos",
    nodeVersion: "0.1.0",
    lastSeenAt: new Date().toISOString(),
  }],
  adapters: [{
    provider: "CODEX",
    installState: "RUNNING",
    hookState: "HEALTHY",
    capabilities: ["SEND_NEXT"],
    message: "healthy",
  }],
  live: {
    observedAt: new Date().toISOString(),
    pollIntervalMs: 2_000,
    connectedProviderCount: 2,
    monitoredProviderCount: 2,
    executingTaskCount: 1,
    waitingTaskCount: 1,
    providers: [
      { provider: "CODEX", connectionState: "RUNNING", trackingState: "LIVE", activeTaskCount: 1 },
      { provider: "CLAUDE", connectionState: "RUNNING", trackingState: "LIVE", activeTaskCount: 1 },
    ],
    tasks: [
      {
        id: "task-1",
        provider: "CODEX",
        deviceId: "dev-1",
        sessionId: "session-1",
        title: "分析 AI 任务汇总 RPA 可行性",
        controlMode: "OBSERVED",
        capabilities: ["SEND_NEXT"],
        state: "RUNNING",
        confidence: "MEDIUM",
        requiredEvidenceLevel: "E2",
        evidenceLevel: "E1",
        updatedAt: new Date().toISOString(),
        lastEventType: "TURN_STARTED",
        stateVersion: 1,
        source: "HOOK_EVENT",
        stale: false,
        ageSeconds: 1,
      },
      {
        id: "task-2",
        provider: "CLAUDE",
        deviceId: "dev-1",
        sessionId: "session-2",
        title: "查找可清理文件",
        controlMode: "OBSERVED",
        capabilities: ["SEND_NEXT"],
        state: "WAITING_USER",
        confidence: "MEDIUM",
        requiredEvidenceLevel: "E2",
        evidenceLevel: "E1",
        updatedAt: new Date(Date.now() - 600_000).toISOString(),
        lastEventType: "WAITING_USER",
        stateVersion: 2,
        source: "HOOK_EVENT",
        stale: true,
        ageSeconds: 600,
      },
    ],
  },
  attention: [],
  recent: [],
};

describe("desktop console", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      const body = url.endsWith("/session")
        ? { mode: "LOCAL", authenticated: true, version: "0.2.0" }
        : dashboard;
      return new Response(JSON.stringify(body), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }));
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    window.localStorage.clear();
  });

  it("renders live dashboard metrics and adapter health", async () => {
    render(<MemoryRouter initialEntries={["/"]}><App /></MemoryRouter>);
    expect(await screen.findByRole("heading", { name: "任务总览" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("100%")).toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "连接电脑上的 AI 任务" })).toBeInTheDocument();
    expect(screen.getByText("分析 AI 任务汇总 RPA 可行性")).toBeInTheDocument();
    expect(screen.getByText("查找可清理文件")).toBeInTheDocument();
    expect(screen.getByText("状态待刷新")).toBeInTheDocument();
    expect(screen.getAllByText("Codex").length).toBeGreaterThan(0);
    expect(screen.getByText("健康")).toBeInTheDocument();
    expect(screen.getByText("尚无任务事件")).toBeInTheDocument();
  });

  it("requires the central token and then opens the remote console", async () => {
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/session")) {
        const authorized = new Headers(init?.headers).get("authorization") === "Bearer valid-token";
        return new Response(JSON.stringify(authorized
          ? { mode: "CENTRAL", authenticated: true, version: "0.2.0" }
          : { error: { message: "请登录中央控制台" } }), {
          status: authorized ? 200 : 401,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(JSON.stringify(dashboard), { status: 200, headers: { "content-type": "application/json" } });
    }));
    render(<MemoryRouter initialEntries={["/"]}><App /></MemoryRouter>);
    expect(await screen.findByRole("heading", { name: "管理所有电脑上的 AI 任务" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("管理员访问令牌"), { target: { value: "valid-token" } });
    fireEvent.click(screen.getByRole("button", { name: "登录控制台" }));
    expect(await screen.findByRole("heading", { name: "任务总览" })).toBeInTheDocument();
    expect(screen.getByText("ctyun 中央控制台")).toBeInTheDocument();
  });

  it("renders central fleet management for desktop and mobile navigation", async () => {
    window.localStorage.setItem("ai-rpa-central-token", "valid-token");
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      const body = url.endsWith("/session")
        ? { mode: "CENTRAL", authenticated: true, version: "0.2.0" }
        : url.endsWith("/devices")
          ? { items: [{ ...dashboard.devices[0], alias: "开发 Mac", online: true, revoked: false }] }
          : {};
      return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
    }));
    render(<MemoryRouter initialEntries={["/devices"]}><App /></MemoryRouter>);
    expect(await screen.findByRole("heading", { name: "设备中心" })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "开发 Mac" })).toBeInTheDocument();
    expect(screen.getAllByText("在线").length).toBeGreaterThan(0);
    expect(screen.getAllByText("设备").length).toBeGreaterThan(0);
  });

  it("assigns one Codex task to every selected project", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      let body: unknown = {};
      if (url.endsWith("/session")) {
        body = { mode: "LOCAL", authenticated: true, version: "0.2.0" };
      } else if (url.endsWith("/codex/status")) {
        body = { state: "READY", version: "codex-cli 0.151.0", authenticated: true, message: "Codex CLI 可用且已登录，可以分配任务。" };
      } else if (url.endsWith("/codex/projects")) {
        body = { items: [
          { id: "project-a", name: "RPA 控制台", path: "/code/rpa", createdAt: new Date().toISOString() },
          { id: "project-b", name: "服务端", path: "/code/server", createdAt: new Date().toISOString() },
        ] };
      } else if (url.includes("/codex/tasks")) {
        body = { items: [] };
      } else if (url.endsWith("/codex/runs") && init?.method === "POST") {
        body = {
          assignments: [
            { projectId: "project-a", projectName: "RPA 控制台", taskId: "task-a", sessionId: "session-a", state: "RUNNING" },
            { projectId: "project-b", projectName: "服务端", taskId: "task-b", sessionId: "session-b", state: "RUNNING" },
          ],
          errors: [],
        };
      }
      return new Response(JSON.stringify(body), { status: url.endsWith("/codex/runs") ? 202 : 200, headers: { "content-type": "application/json" } });
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<MemoryRouter initialEntries={["/codex"]}><App /></MemoryRouter>);

    expect(await screen.findByRole("heading", { name: "Codex 项目任务" })).toBeInTheDocument();
    expect(await screen.findByText("可分配")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: /RPA 控制台/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /服务端/ }));
    fireEvent.change(screen.getByLabelText("任务名称"), { target: { value: "检查构建" } });
    fireEvent.change(screen.getByPlaceholderText("明确说明目标、限制条件和验收方式。"), { target: { value: "运行测试并汇总结果" } });
    fireEvent.click(screen.getByRole("button", { name: "分配到 2 个项目" }));

    expect(await screen.findByText("已向 2 个项目分别创建独立 Codex 任务。" )).toBeInTheDocument();
    const runCall = fetchMock.mock.calls.find(([input]) => String(input).endsWith("/codex/runs"));
    expect(runCall).toBeDefined();
    expect(JSON.parse(String(runCall?.[1]?.body))).toMatchObject({
      title: "检查构建",
      projectIds: ["project-a", "project-b"],
      timeoutSeconds: 3600,
      sandbox: "read-only",
    });
  });
});
