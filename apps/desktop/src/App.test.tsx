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
    expect(screen.getByText("Codex")).toBeInTheDocument();
    expect(screen.getByText("健康")).toBeInTheDocument();
    expect(screen.getByText("尚无任务事件")).toBeInTheDocument();
  });
});
