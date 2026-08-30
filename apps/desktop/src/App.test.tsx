import { cleanup, render, screen, waitFor } from "@testing-library/react";
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
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify(dashboard), {
      status: 200,
      headers: { "content-type": "application/json" },
    })));
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("renders live dashboard metrics and adapter health", async () => {
    render(<MemoryRouter initialEntries={["/"]}><App /></MemoryRouter>);
    expect(screen.getByRole("heading", { name: "任务总览" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("100%")).toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "连接电脑上的 AI 任务" })).toBeInTheDocument();
    expect(screen.getByText("分析 AI 任务汇总 RPA 可行性")).toBeInTheDocument();
    expect(screen.getByText("查找可清理文件")).toBeInTheDocument();
    expect(screen.getByText("状态待刷新")).toBeInTheDocument();
    expect(screen.getAllByText("Codex").length).toBeGreaterThan(0);
    expect(screen.getByText("健康")).toBeInTheDocument();
    expect(screen.getByText("尚无任务事件")).toBeInTheDocument();
  });
});
