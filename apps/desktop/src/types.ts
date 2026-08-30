export type Provider = "CODEX" | "CLAUDE" | "CURSOR" | "ANTIGRAVITY";
export type TaskState =
  | "RUNNING"
  | "WAITING_USER"
  | "FAILED"
  | "SUCCEEDED"
  | "CANCELLED"
  | "UNKNOWN";
export type Capability =
  | "SEND_NEXT"
  | "RESUME_AND_SEND"
  | "STEER_ACTIVE"
  | "OPEN_AND_PREFILL";

export interface Device {
  id: string;
  os: string;
  arch: string;
  hostname: string;
  logicalEnvironment: string;
  nodeVersion: string;
  lastSeenAt: string;
}
export interface Adapter {
  provider: Provider;
  installState: string;
  executable?: string;
  version?: string;
  hookState: string;
  lastEventAt?: string;
  capabilities: Capability[];
  message: string;
}

export interface Task {
  id: string;
  provider: Provider;
  deviceId: string;
  sessionId: string;
  title: string;
  workspace?: string;
  project?: string;
  controlMode: "OBSERVED" | "MANAGED";
  capabilities: Capability[];
  state: TaskState;
  confidence: string;
  requiredEvidenceLevel: "E0" | "E1" | "E2" | "E3";
  evidenceLevel: "E0" | "E1" | "E2" | "E3";
  evidenceSummary?: string;
  startedAt?: string;
  updatedAt: string;
  durationMs?: number;
  lastEventType: string;
  stateVersion: number;
}

export interface TimelineEvent {
  eventId: string;
  eventType: string;
  provider: Provider;
  occurredAt: string;
  evidenceLevel: string;
  evidenceSummary?: string;
  attributes: Record<string, unknown>;
}

export interface Command {
  id: string;
  action: string;
  state: string;
  createdAt: string;
  expiresAt: string;
  createdBy: string;
  attempts: number;
  resultSummary?: string;
}

export interface Audit {
  id: string;
  action: string;
  actor: string;
  summary: string;
  occurredAt: string;
}

export interface TaskDetail {
  task: Task;
  events: TimelineEvent[];
  commands: Command[];
  audit: Audit[];
}

export interface Dashboard {
  counts: Record<string, number>;
  completionRate24h: number;
  p95DurationMs?: number;
  devices: Device[];
  adapters: Adapter[];
  attention: Task[];
  recent: Task[];
}

export interface DiagnosticCheck {
  name: string;
  status: string;
  message: string;
}

export interface Diagnostics {
  generatedAt: string;
  nodeVersion: string;
  device: Device;
  checks: DiagnosticCheck[];
  adapters: Adapter[];
  counts: Record<string, number>;
}
