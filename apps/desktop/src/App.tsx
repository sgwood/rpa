import {
  type FormEvent,
  type ReactNode,
  createContext,
  useContext,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import {
  NavLink,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
  useParams,
} from "react-router-dom";
import { api, ApiError } from "./api";
import {
  adapterMessageLabel,
  diagnosticLabel,
  eventLabel,
  evidenceSummaryLabel,
  formatDuration,
  installStateLabel,
  providerLabel,
  stateMeta,
  timeAgo,
} from "./format";
import type {
  Adapter,
  CentralStatus,
  Dashboard as DashboardData,
  Device,
  Diagnostics as DiagnosticsData,
  LiveProvider,
  LiveTask,
  SessionInfo,
  Task,
  TaskDetail as TaskDetailData,
  TaskState,
} from "./types";

const SessionContext = createContext<SessionInfo>({ mode: "LOCAL", authenticated: true, version: "" });

function useSession() {
  return useContext(SessionContext);
}

function usePolling<T>(loader: () => Promise<T>, interval = 5_000) {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const refresh = useCallback(async () => {
    try {
      setData(await loader());
      setError(undefined);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "无法连接本机节点");
    } finally {
      setLoading(false);
    }
  }, [loader]);
  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), interval);
    return () => window.clearInterval(timer);
  }, [interval, refresh]);
  return { data, error, loading, refresh };
}

function AppShell({ children }: { children: ReactNode }) {
  const session = useSession();
  const navigation = [
    ["/", "总览", "01"],
    ["/tasks", "任务", "02"],
    ["/tasks?state=WAITING_USER", "等待处理", "03"],
    ["/devices", "设备", "04"],
    ...(session.mode === "LOCAL" ? [["/integrations", "本机接入", "05"]] : []),
  ];
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">AI</span>
          <span>AI 任务中控台</span>
        </div>
        <div className="nav-label">工作区</div>
        <nav>
          {navigation.map(([to, label, index]) => (
            <NavLink
              className={({ isActive }) =>
                `nav-item ${isActive && (to === "/" || location.hash.startsWith(`#${to}`)) ? "active" : ""}`
              }
              end={to === "/"}
              key={to}
              to={to}
            >
              <span className="nav-index">{index}</span>
              <span>{label}</span>
            </NavLink>
          ))}
        </nav>
        <div className="sidebar-device">
          <strong>{session.mode === "CENTRAL" ? "ctyun 中央控制台" : "本机节点"}</strong>
          <span><i className="online-dot" /> {session.mode === "CENTRAL" ? "跨设备实时同步" : "30 秒心跳"}</span>
          <small>{session.mode === "CENTRAL" ? "公网安全连接" : "数据默认仅保存在本机"}</small>
          {session.mode === "CENTRAL" && <button className="sidebar-logout" onClick={() => { api.logout(); window.location.reload(); }}>退出登录</button>}
        </div>
      </aside>
      <main className="main-content">{children}</main>
      <nav className="mobile-nav" aria-label="移动端导航">
        {navigation.map(([to, label, index]) => (
          <NavLink key={to} to={to} end={to === "/"}>
            <span>{index}</span>{label}
          </NavLink>
        ))}
      </nav>
    </div>
  );
}

function PageHeader({
  title,
  subtitle,
  action,
}: {
  title: string;
  subtitle: string;
  action?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div>
        <h1>{title}</h1>
        <p>{subtitle}</p>
      </div>
      {action}
    </header>
  );
}

function StatusBadge({ state }: { state: TaskState }) {
  const meta = stateMeta[state];
  return <span className={`badge ${meta.tone}`}>{meta.label}</span>;
}

function ErrorBanner({ message, retry }: { message?: string; retry?: () => void }) {
  const session = useSession();
  if (!message) return null;
  return (
    <div className="error-banner" role="alert">
      <div><strong>{session.mode === "CENTRAL" ? "中央服务暂不可用" : "本机节点暂不可用"}</strong><span>{message}</span></div>
      {retry && <button className="button secondary" onClick={retry}>重试</button>}
    </div>
  );
}

function EmptyState({ title, description }: { title: string; description: string }) {
  return (
    <div className="empty-state">
      <span className="empty-icon">✓</span>
      <strong>{title}</strong>
      <p>{description}</p>
    </div>
  );
}

function DashboardPage() {
  const session = useSession();
  const loader = useCallback(() => api.dashboard(), []);
  const { data, error, loading, refresh } = usePolling<DashboardData>(loader, 2_000);
  const cards: Array<[TaskState, string]> = [
    ["RUNNING", "正在执行"],
    ["WAITING_USER", "需要处理"],
    ["FAILED", "执行失败"],
    ["UNKNOWN", "证据不足"],
  ];
  return (
    <AppShell>
      <PageHeader
        title="任务总览"
        subtitle="跨工具、跨设备查看 AI 任务的真实执行状态"
        action={<button className="button" onClick={() => void refresh()}>刷新状态</button>}
      />
      <ErrorBanner message={error} retry={() => void refresh()} />
      <section className="live-panel" aria-label="实时执行任务">
        <div className="live-panel-header">
          <div>
            <span className="live-kicker"><i className="live-pulse" /> 实时监控</span>
            <h2>连接电脑上的 AI 任务</h2>
            <p>状态来自工具 Hook 事件；工具已打开不代表任务正在执行。</p>
          </div>
          <small>{data?.live ? `每 2 秒刷新 · ${timeAgo(data.live.observedAt)}` : "正在连接本机节点…"}</small>
        </div>
        <div className="live-summary" aria-label="实时统计">
          <div><span>已连接工具</span><strong>{loading ? "—" : (data?.live.connectedProviderCount ?? 0)}</strong></div>
          <div><span>已启用监控</span><strong>{loading ? "—" : (data?.live.monitoredProviderCount ?? 0)}</strong></div>
          <div><span>正在执行</span><strong>{loading ? "—" : (data?.live.executingTaskCount ?? 0)}</strong></div>
          <div><span>等待处理</span><strong>{loading ? "—" : (data?.live.waitingTaskCount ?? 0)}</strong></div>
        </div>
        <div className="live-provider-grid">
          {(data?.live.providers ?? []).map((provider) => (
            <LiveProviderCard provider={provider} key={provider.provider} />
          ))}
        </div>
        <div className="live-task-list">
          <div className="live-task-list-title">
            <strong>活动任务</strong>
            <span>{data?.live.tasks.length ?? 0} 个</span>
          </div>
          {data?.live.tasks.length ? (
            data.live.tasks.map((task) => <LiveTaskRow key={task.id} task={task} />)
          ) : !loading ? (
            <EmptyState
              title="当前没有已捕获的活动任务"
              description={`已连接 ${data?.live.connectedProviderCount ?? 0} 个工具；新会话或下一次 Hook 事件后会自动出现。`}
            />
          ) : null}
        </div>
      </section>
      <section className="metric-grid" aria-label="状态统计">
        {cards.map(([state, description]) => (
          <article className="metric-card" key={state}>
            <div><span className={`status-dot ${stateMeta[state].tone}`} />{stateMeta[state].label}</div>
            <strong>{loading ? "—" : (data?.counts[state] ?? 0)}</strong>
            <small>{description}</small>
          </article>
        ))}
      </section>
      <section className="summary-strip">
        <div><span>24 小时完成率</span><strong>{data ? `${Math.round(data.completionRate24h * 100)}%` : "—"}</strong></div>
        <div><span>P95 执行时长</span><strong>{formatDuration(data?.p95DurationMs)}</strong></div>
        <div><span>在线设备</span><strong>{data?.devices.filter((device) => device.online !== false && !device.revoked).length ?? 0}</strong></div>
      </section>
      <SectionTitle
        title="工具接入"
        action={<NavLink to={session.mode === "CENTRAL" ? "/devices" : "/integrations"}>{session.mode === "CENTRAL" ? "管理设备" : "查看诊断"}</NavLink>}
      />
      <section className="tool-grid">
        {(data?.adapters ?? []).map((adapter) => <AdapterCard adapter={adapter} key={adapter.provider} />)}
        {!loading && !data?.adapters.length && (
          <EmptyState
            title="尚未发现工具"
            description={session.mode === "CENTRAL" ? "设备接入并上报心跳后，会显示各电脑上的工具状态。" : "运行只读自检后会显示四个工具的安装和 Hook 状态。"}
          />
        )}
      </section>
      <section className="two-column">
        <Panel title="需要处理" count={data?.attention.length}>
          {data?.attention.length ? (
            data.attention.slice(0, 4).map((task) => <TaskRow key={task.id} task={task} />)
          ) : (
            <EmptyState title="当前没有待处理任务" description="失败、等待人工或状态不明的任务会出现在这里。" />
          )}
        </Panel>
        <Panel title="最近任务" count={data?.recent.length}>
          {data?.recent.length ? (
            data.recent.slice(0, 4).map((task) => <TaskRow key={task.id} task={task} />)
          ) : (
            <EmptyState title="尚无任务事件" description="安装 Hook 或使用 CLI 注入测试事件后即可开始监控。" />
          )}
        </Panel>
      </section>
    </AppShell>
  );
}

const trackingStateMeta: Record<string, { label: string; tone: string }> = {
  LIVE: { label: "实时", tone: "green" },
  READY: { label: "等待首个事件", tone: "blue" },
  STALE: { label: "状态待刷新", tone: "orange" },
  NOT_CONFIGURED: { label: "未配置监控", tone: "orange" },
  OFFLINE: { label: "未运行", tone: "gray" },
};

function LiveProviderCard({ provider }: { provider: LiveProvider }) {
  const meta = trackingStateMeta[provider.trackingState] ?? { label: provider.trackingState, tone: "gray" };
  return (
    <article className="live-provider-card">
      <span className={`provider-icon mini ${provider.provider.toLowerCase()}`}>{providerLabel[provider.provider].slice(0, 1)}</span>
      <span>
        <strong>{providerLabel[provider.provider]}</strong>
        <small>{provider.activeTaskCount} 个活动任务</small>
      </span>
      <span className={`badge ${meta.tone}`}>{meta.label}</span>
    </article>
  );
}

function LiveTaskRow({ task }: { task: LiveTask }) {
  return (
    <NavLink className={`task-row live-task-row ${task.stale ? "stale" : ""}`} to={`/tasks/${task.id}`}>
      <span className={`provider-icon mini ${task.provider.toLowerCase()}`}>{providerLabel[task.provider].slice(0, 1)}</span>
      <span className="task-row-copy">
        <strong>{task.title}</strong>
        <small>{providerLabel[task.provider]} · Hook 事件 · {timeAgo(task.updatedAt)}</small>
      </span>
      <span className="live-task-status">
        <StatusBadge state={task.state} />
        {task.stale && <small>状态待刷新</small>}
      </span>
    </NavLink>
  );
}

function SectionTitle({ title, action }: { title: string; action?: ReactNode }) {
  return <div className="section-title"><h2>{title}</h2>{action}</div>;
}

function Panel({ title, count, children }: { title: string; count?: number; children: ReactNode }) {
  return (
    <section className="panel">
      <div className="panel-title"><h2>{title}</h2>{count !== undefined && <span>{count}</span>}</div>
      <div className="panel-body">{children}</div>
    </section>
  );
}

function AdapterCard({ adapter }: { adapter: Adapter }) {
  const healthy = adapter.hookState === "HEALTHY";
  const attention = adapter.installState === "NOT_INSTALLED" || !healthy;
  return (
    <article className="tool-card">
      <div className="tool-card-top">
        <span className={`provider-icon ${adapter.provider.toLowerCase()}`}>{providerLabel[adapter.provider].slice(0, 2)}</span>
        <span className={`badge ${attention ? "orange" : "green"}`}>{attention ? "需关注" : "健康"}</span>
      </div>
      <h3>{providerLabel[adapter.provider]}</h3>
      <p>{installStateLabel(adapter.installState)}</p>
      <small>{adapter.lastEventAt ? `最近事件 ${timeAgo(adapter.lastEventAt)}` : adapterMessageLabel(adapter.message)}</small>
    </article>
  );
}

function TaskRow({ task }: { task: Task }) {
  return (
    <NavLink className="task-row" to={`/tasks/${task.id}`}>
      <span className={`provider-icon mini ${task.provider.toLowerCase()}`}>{providerLabel[task.provider].slice(0, 1)}</span>
      <span className="task-row-copy">
        <strong>{task.title}</strong>
        <small>{providerLabel[task.provider]} · {task.project ?? "未关联项目"} · {timeAgo(task.updatedAt)}</small>
      </span>
      <StatusBadge state={task.state} />
    </NavLink>
  );
}

function TasksPage() {
  const locationQuery = new URLSearchParams(window.location.hash.split("?")[1] ?? "");
  const [provider, setProvider] = useState(locationQuery.get("provider") ?? "");
  const [state, setState] = useState(locationQuery.get("state") ?? "");
  const [search, setSearch] = useState("");
  const [deviceId, setDeviceId] = useState("");
  const [project, setProject] = useState("");
  const [controlMode, setControlMode] = useState("");
  const [timeRange, setTimeRange] = useState("");
  const query = useMemo(() => {
    const value = new URLSearchParams();
    if (provider) value.set("provider", provider);
    if (state) value.set("state", state);
    if (search) value.set("search", search);
    if (deviceId) value.set("deviceId", deviceId);
    if (project) value.set("project", project);
    if (controlMode) value.set("controlMode", controlMode);
    if (timeRange) {
      const hours = Number(timeRange);
      value.set("updatedAfter", new Date(Date.now() - hours * 60 * 60 * 1000).toISOString());
    }
    value.set("limit", "200");
    return value.toString();
  }, [controlMode, deviceId, project, provider, search, state, timeRange]);
  const loader = useCallback(() => api.tasks(query), [query]);
  const { data, error, loading, refresh } = usePolling(loader);
  return (
    <AppShell>
      <PageHeader title="全部任务" subtitle="基于厂商事件与证据等级展示真实状态" />
      <ErrorBanner message={error} retry={() => void refresh()} />
      <section className="filter-bar">
        <label className="search-field"><span>搜索</span><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="任务、项目、会话或工作区" /></label>
        <label><span>工具</span><select value={provider} onChange={(event) => setProvider(event.target.value)}><option value="">全部工具</option>{Object.entries(providerLabel).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
        <label><span>状态</span><select value={state} onChange={(event) => setState(event.target.value)}><option value="">全部状态</option>{Object.entries(stateMeta).map(([value, meta]) => <option key={value} value={value}>{meta.label}</option>)}</select></label>
        <label><span>控制模式</span><select value={controlMode} onChange={(event) => setControlMode(event.target.value)}><option value="">全部模式</option><option value="MANAGED">托管会话</option><option value="OBSERVED">观察会话</option></select></label>
        <label><span>项目</span><input value={project} onChange={(event) => setProject(event.target.value)} placeholder="精确项目名" /></label>
        <label><span>设备</span><input value={deviceId} onChange={(event) => setDeviceId(event.target.value)} placeholder="设备 ID" /></label>
        <label><span>更新时间</span><select value={timeRange} onChange={(event) => setTimeRange(event.target.value)}><option value="">全部时间</option><option value="24">最近 24 小时</option><option value="168">最近 7 天</option><option value="720">最近 30 天</option></select></label>
      </section>
      <section className="task-list">
        <div className="list-summary"><span>{loading ? "读取中…" : `${data?.items.length ?? 0} 个任务`}</span><button className="link-button" onClick={() => void refresh()}>刷新</button></div>
        {data?.items.map((task) => <TaskCard key={task.id} task={task} />)}
        {!loading && !data?.items.length && <EmptyState title="没有匹配的任务" description="尝试清除筛选条件，或先接入一个 AI 工具。" />}
      </section>
    </AppShell>
  );
}

function TaskCard({ task }: { task: Task }) {
  return (
    <NavLink className="task-card" to={`/tasks/${task.id}`}>
      <span className={`provider-icon ${task.provider.toLowerCase()}`}>{providerLabel[task.provider].slice(0, 2)}</span>
      <span className="task-card-main">
        <span className="task-card-title"><strong>{task.title}</strong><StatusBadge state={task.state} /></span>
        <span className="task-card-meta">{providerLabel[task.provider]} · {task.controlMode === "MANAGED" ? "托管会话" : "观察会话"} · 可信度 {task.confidence}</span>
        <span className="task-card-meta">{task.workspace ?? "工作区未知"} · 设备 {compactDeviceId(task.deviceId)} · 证据 {task.evidenceLevel}/{task.requiredEvidenceLevel}</span>
      </span>
      <span className="task-card-time"><small>最近事件</small><strong>{timeAgo(task.updatedAt)}</strong><small>{formatDuration(task.durationMs)}</small></span>
      <span className="chevron">›</span>
    </NavLink>
  );
}

function compactDeviceId(value: string) {
  return value.length > 14 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;
}

function TaskDetailPage() {
  const { id } = useParams();
  const location = useLocation();
  const navigate = useNavigate();
  const loader = useCallback(() => api.task(id!), [id]);
  const { data, error, loading, refresh } = usePolling<TaskDetailData>(loader, 3_000);
  const [copied, setCopied] = useState(false);
  if (!id) return <Navigate to="/tasks" replace />;
  const task = data?.task;
  const copySession = async () => {
    if (!task) return;
    await navigator.clipboard.writeText(task.sessionId);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };
  return (
    <AppShell>
      <PageHeader
        title={task?.title ?? (loading ? "读取任务…" : "任务详情")}
        subtitle={task ? `${providerLabel[task.provider]} · ${task.controlMode === "MANAGED" ? "托管会话" : "观察会话"} · 更新于 ${timeAgo(task.updatedAt)}` : "查看时间线、证据与命令投递结果"}
        action={task && <div className="header-actions"><button className="button secondary" onClick={() => void copySession()}>{copied ? "已复制" : "复制会话 ID"}</button><button className="button" disabled={!task.capabilities.includes("SEND_NEXT")} onClick={() => navigate(`/tasks/${task.id}/continue`)}>继续任务</button></div>}
      />
      <ErrorBanner message={error} retry={() => void refresh()} />
      {(location.state as { notice?: string } | null)?.notice && <p className="inline-notice success-notice">{(location.state as { notice: string }).notice}</p>}
      {task && (
        <>
          <section className="detail-status-card">
            <div><StatusBadge state={task.state} /><h2>{statusTitle(task)}</h2><p>{evidenceSummaryLabel(task.evidenceSummary)}</p></div>
            <dl><div><dt>证据门槛</dt><dd>{task.evidenceLevel} / {task.requiredEvidenceLevel}</dd></div><div><dt>可信度</dt><dd>{{ HIGH: "高", MEDIUM: "中", LOW: "低" }[task.confidence] ?? task.confidence}</dd></div><div><dt>执行时长</dt><dd>{formatDuration(task.durationMs)}</dd></div></dl>
          </section>
          <section className="detail-grid">
            <Panel title="事件时间线" count={data?.events.length}>
              <div className="timeline">
                {data?.events.map((event) => (
                  <article key={event.eventId} className="timeline-item">
                    <i className={`timeline-dot ${event.eventType.includes("FAILED") ? "red" : "blue"}`} />
                    <div><strong>{eventLabel(event.eventType)}</strong><p>{evidenceSummaryLabel(event.evidenceSummary)}</p><small>{new Date(event.occurredAt).toLocaleString("zh-CN")} · 证据 {event.evidenceLevel}</small></div>
                  </article>
                ))}
                {!data?.events.length && <EmptyState title="尚无事件" description="该任务还没有进入事件账本。" />}
              </div>
            </Panel>
            <div className="detail-side">
              <Panel title="会话信息">
                <InfoRow label="工具" value={providerLabel[task.provider]} />
                <InfoRow label="控制模式" value={task.controlMode === "MANAGED" ? "托管会话" : "观察会话"} />
                <InfoRow label="设备" value={task.deviceId} mono />
                <InfoRow label="会话" value={task.sessionId} mono />
                <InfoRow label="项目" value={task.project ?? "未知"} />
                <InfoRow label="工作区" value={task.workspace ?? "未知"} mono />
              </Panel>
              <Panel title="命令队列" count={data?.commands.length}>
                {data?.commands.map((command) => <div className="command-row" key={command.id}><div><strong>{command.action}</strong><small>{command.resultSummary ?? "内容已加密"}</small></div><span className="badge gray">{command.state}</span></div>)}
                {!data?.commands.length && <p className="muted">尚未排队后续任务。</p>}
              </Panel>
            </div>
          </section>
        </>
      )}
    </AppShell>
  );
}

function statusTitle(task: Task): string {
  switch (task.state) {
    case "WAITING_USER": return "需要你在原工具中完成授权或输入";
    case "FAILED": return "任务执行失败，请根据证据决定下一步";
    case "SUCCEEDED": return "任务已达到要求的完成证据";
    case "RUNNING": return "任务正在执行，状态会自动刷新";
    case "CANCELLED": return "任务已被用户或工具取消";
    default: return "任务已停止，但证据不足以判断成功或失败";
  }
}

function InfoRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return <div className="info-row"><span>{label}</span><strong className={mono ? "mono" : ""}>{value}</strong></div>;
}

function ContinueTaskPage() {
  const session = useSession();
  const { id } = useParams();
  const navigate = useNavigate();
  const loader = useCallback(() => api.task(id!), [id]);
  const { data, error, loading, refresh } = usePolling<TaskDetailData>(loader, 30_000);
  const [action, setAction] = useState("SEND_NEXT");
  const [ttl, setTtl] = useState(7200);
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string>();
  if (!id) return <Navigate to="/tasks" replace />;
  const task = data?.task;
  const availableActions = [
    ["SEND_NEXT", "当前回合停止后执行"],
    ["RESUME_AND_SEND", "立即恢复托管会话"],
    ["OPEN_AND_PREFILL", "复制任务，手动打开原会话"],
  ].filter(([value]) => task?.capabilities.includes(value as never) && !(session.mode === "CENTRAL" && value === "OPEN_AND_PREFILL"));
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!message.trim()) return setFormError("请输入要发送的任务内容");
    setSubmitting(true);
    setFormError(undefined);
    try {
      if (action === "OPEN_AND_PREFILL") {
        await navigator.clipboard.writeText(message);
        await api.openTask(id);
        navigate(`/tasks/${id}`, { state: { notice: "任务内容已复制，请在原会话中粘贴发送。" } });
        return;
      }
      await api.command(id, { action, message, ttlSeconds: ttl });
      navigate(`/tasks/${id}`);
    } catch (reason) {
      setFormError(reason instanceof Error ? reason.message : "命令入队失败");
    } finally {
      setSubmitting(false);
    }
  };
  return (
    <AppShell>
      <PageHeader title="继续任务" subtitle={task ? `${providerLabel[task.provider]} · ${task.title}` : "选择安全的继续方式"} />
      <ErrorBanner message={error} retry={() => void refresh()} />
      <form className="continue-form" onSubmit={(event) => void submit(event)}>
        <div className="form-intro"><span className={`provider-icon ${task?.provider.toLowerCase()}`}>{task ? providerLabel[task.provider].slice(0, 2) : "AI"}</span><div><strong>{task?.sessionId ?? (loading ? "读取会话…" : "未知会话")}</strong><p>{task?.controlMode === "MANAGED" ? "托管会话：支持后台恢复" : "观察会话：能力会自动降级"}</p></div></div>
        <label><span>执行方式</span><select value={action} onChange={(event) => setAction(event.target.value)}>{availableActions.map(([value, description]) => <option key={value} value={value}>{value} · {description}</option>)}</select></label>
        <div className="form-row"><label><span>命令有效期</span><select value={ttl} onChange={(event) => setTtl(Number(event.target.value))}><option value={1800}>30 分钟</option><option value={7200}>2 小时</option><option value={28800}>8 小时</option><option value={86400}>24 小时</option></select></label><label><span>要求证据</span><input disabled value={task?.requiredEvidenceLevel ?? "E2"} /></label></div>
        <label><span>任务内容</span><textarea rows={8} maxLength={32 * 1024} value={message} onChange={(event) => setMessage(event.target.value)} placeholder="例如：继续运行完整测试；如果失败，请定位原因并修复后再次验证。" /><small>{session.mode === "CENTRAL" ? "内容经 HTTPS/WSS 传输，并在 ctyun PostgreSQL 中加密保存；不会写入日志或通知。" : "内容会在本机使用系统凭据保护的密钥加密；不会写入日志或通知。"}</small></label>
        <div className="security-note"><strong>权限边界</strong><p>该产品只投递用户消息，不会自动批准删除、发布、支付或绕过沙箱。敏感动作仍由原工具请求确认。</p></div>
        {formError && <p className="form-error">{formError}</p>}
        <footer><button type="button" className="button secondary" onClick={() => navigate(-1)}>取消</button><button className="button" disabled={submitting || !task}>{submitting ? "正在入队…" : "加入队列"}</button></footer>
      </form>
    </AppShell>
  );
}

function DevicesPage() {
  const session = useSession();
  return session.mode === "CENTRAL" ? <CentralDevicesPage /> : <LocalDevicePage />;
}

function CentralDevicesPage() {
  const loader = useCallback(() => api.devices(), []);
  const { data, error, loading, refresh } = usePolling(loader, 5_000);
  const [enrollment, setEnrollment] = useState<{ code: string; expiresAt: string }>();
  const [notice, setNotice] = useState<string>();
  const [editingId, setEditingId] = useState<string>();
  const [alias, setAlias] = useState("");
  const [confirmRevoke, setConfirmRevoke] = useState<string>();
  const online = data?.items.filter((device) => device.online && !device.revoked).length ?? 0;
  const createCode = async () => {
    try {
      setEnrollment(await api.createEnrollmentCode());
      setNotice("配对码已生成，15 分钟内可使用一次");
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "配对码生成失败");
    }
  };
  const copyCode = async () => {
    if (!enrollment) return;
    await navigator.clipboard.writeText(enrollment.code);
    setNotice("配对码已复制");
  };
  const saveAlias = async (device: Device) => {
    if (!alias.trim()) return setNotice("设备名称不能为空");
    try {
      await api.renameDevice(device.id, alias.trim());
      setEditingId(undefined);
      setAlias("");
      setNotice("设备名称已更新");
      await refresh();
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "设备名称更新失败");
    }
  };
  const revoke = async (device: Device) => {
    if (confirmRevoke !== device.id) {
      setConfirmRevoke(device.id);
      setNotice(`再次点击“确认撤销”即可断开 ${device.alias ?? device.hostname}`);
      return;
    }
    try {
      await api.revokeDevice(device.id);
      setConfirmRevoke(undefined);
      setNotice("设备已撤销，原令牌立即失效");
      await refresh();
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "设备撤销失败");
    }
  };
  return (
    <AppShell>
      <PageHeader
        title="设备中心"
        subtitle="管理通过公网安全连接到 ctyun 的 Mac 与 Windows 节点"
        action={<button className="button" onClick={() => void createCode()}>添加设备</button>}
      />
      <ErrorBanner message={error} retry={() => void refresh()} />
      <section className="fleet-summary">
        <div><span>设备总数</span><strong>{loading ? "—" : data?.items.length ?? 0}</strong></div>
        <div><span>当前在线</span><strong>{loading ? "—" : online}</strong></div>
        <div><span>连接方式</span><strong>WSS / 443</strong></div>
      </section>
      {enrollment && (
        <section className="enrollment-card">
          <div><span>一次性设备配对码</span><strong>{enrollment.code}</strong><small>{new Date(enrollment.expiresAt).toLocaleTimeString("zh-CN")} 前有效，使用一次后自动失效</small></div>
          <button className="button" onClick={() => void copyCode()}>复制配对码</button>
        </section>
      )}
      {notice && <p className="inline-notice success-notice">{notice}</p>}
      <div className="device-grid">
        {data?.items.map((device) => (
          <article className={`device-card ${device.revoked ? "revoked" : ""}`} key={device.id}>
            <header>
              <span className={`device-os ${device.os}`}>{device.os === "macos" ? "Mac" : device.os === "windows" ? "Win" : device.os}</span>
              <span className={`badge ${device.revoked ? "gray" : device.online ? "green" : "orange"}`}>{device.revoked ? "已撤销" : device.online ? "在线" : "离线"}</span>
            </header>
            {editingId === device.id ? (
              <div className="device-edit"><input autoFocus maxLength={64} value={alias} onChange={(event) => setAlias(event.target.value)} /><button className="button" onClick={() => void saveAlias(device)}>保存</button><button className="button secondary" onClick={() => setEditingId(undefined)}>取消</button></div>
            ) : (
              <><h3>{device.alias ?? device.hostname}</h3><p>{device.hostname} · {device.arch} · Agent {device.nodeVersion}</p></>
            )}
            <dl><div><dt>环境</dt><dd>{device.logicalEnvironment}</dd></div><div><dt>最后心跳</dt><dd>{timeAgo(device.lastSeenAt)}</dd></div><div><dt>设备 ID</dt><dd className="mono">{compactDeviceId(device.id)}</dd></div></dl>
            {!device.revoked && editingId !== device.id && <footer><button className="link-button" onClick={() => { setEditingId(device.id); setAlias(device.alias ?? device.hostname); }}>修改名称</button><button className={`link-button danger ${confirmRevoke === device.id ? "confirm" : ""}`} onClick={() => void revoke(device)}>{confirmRevoke === device.id ? "确认撤销" : "撤销设备"}</button></footer>}
          </article>
        ))}
        {!loading && !data?.items.length && <EmptyState title="还没有连接设备" description="点击“添加设备”生成一次性配对码，然后在目标电脑的 AI 任务中控台中完成配对。" />}
      </div>
    </AppShell>
  );
}

function LocalDevicePage() {
  const deviceLoader = useCallback(() => api.devices(), []);
  const { data, error, refresh } = usePolling(deviceLoader, 15_000);
  const [central, setCentral] = useState<CentralStatus>();
  const [serverUrl, setServerUrl] = useState("");
  const [code, setCode] = useState("");
  const [deviceAlias, setDeviceAlias] = useState("");
  const [notice, setNotice] = useState<string>();
  const [submitting, setSubmitting] = useState(false);
  const loadCentral = useCallback(async () => setCentral(await api.centralStatus()), []);
  useEffect(() => { void loadCentral(); }, [loadCentral]);
  const connect = async (event: FormEvent) => {
    event.preventDefault();
    if (!serverUrl.trim() || !code.trim()) return setNotice("请输入 ctyun 控制台地址和配对码");
    setSubmitting(true);
    try {
      await api.connectCentral({ serverUrl: serverUrl.trim(), enrollmentCode: code.trim(), deviceAlias: deviceAlias.trim() || undefined });
      setCode("");
      setNotice("配对成功，节点将在数秒内建立 WSS 安全连接");
      await loadCentral();
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "中央控制台配对失败");
    } finally {
      setSubmitting(false);
    }
  };
  const disconnect = async () => {
    try {
      await api.disconnectCentral();
      setNotice("已清除本机中央连接凭据");
      await loadCentral();
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "断开失败");
    }
  };
  const device = data?.items[0];
  return (
    <AppShell>
      <PageHeader title="本机与中央连接" subtitle="将这台电脑安全接入 ctyun，实现跨网络监控和任务下发" action={<button className="button secondary" onClick={() => void refresh()}>刷新</button>} />
      <ErrorBanner message={error} retry={() => void refresh()} />
      {device && <section className="node-card"><div><i className="online-dot large" /><div><strong>{device.hostname}</strong><span>{device.os} · {device.arch} · Agent v{device.nodeVersion}</span></div></div><span className="badge green">本机在线</span></section>}
      {central?.configured ? (
        <section className="central-connected-card">
          <div><span className="live-kicker"><i className="live-pulse" /> 已接入 ctyun</span><h2>{central.deviceAlias}</h2><p>{central.serverUrl}</p><small>设备 ID：{central.deviceId}</small></div>
          <button className="button secondary" onClick={() => void disconnect()}>断开中央连接</button>
        </section>
      ) : (
        <form className="central-connect-form" onSubmit={(event) => void connect(event)}>
          <div><h2>连接 ctyun 中央控制台</h2><p>在中央控制台“设备”页面生成一次性配对码。节点只建立出站 443/WSS 连接，无需开放本机端口。</p></div>
          <label><span>中央控制台地址</span><input inputMode="url" value={serverUrl} onChange={(event) => setServerUrl(event.target.value)} placeholder="https://ai-control.example.com" /></label>
          <div className="form-row"><label><span>一次性配对码</span><input autoCapitalize="characters" value={code} onChange={(event) => setCode(event.target.value.toUpperCase())} placeholder="例如 A1B2C3D4E5" /></label><label><span>设备名称</span><input value={deviceAlias} onChange={(event) => setDeviceAlias(event.target.value)} placeholder={device?.hostname ?? "我的电脑"} /></label></div>
          <div className="security-note"><strong>安全说明</strong><p>配对码使用一次即失效；设备令牌保存在系统钥匙串或 Credential Manager，中央端不能执行任意 shell。</p></div>
          <button className="button full" disabled={submitting}>{submitting ? "正在安全配对…" : "连接中央控制台"}</button>
        </form>
      )}
      {notice && <p className="inline-notice success-notice">{notice}</p>}
    </AppShell>
  );
}

function IntegrationsPage() {
  const loader = useCallback(() => api.diagnostics(), []);
  const { data, error, loading, refresh } = usePolling<DiagnosticsData>(loader, 15_000);
  const [notice, setNotice] = useState<string>();
  const [webhook, setWebhook] = useState("");
  const [saving, setSaving] = useState(false);
  const flush = async () => {
    try {
      const report = await api.flushNotifications();
      setNotice(report.configured ? `通知处理完成：发送 ${report.sent}，失败 ${report.failed}` : "飞书 Webhook 尚未配置");
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "通知测试失败");
    }
  };
  const saveWebhook = async (event: FormEvent) => {
    event.preventDefault();
    if (!webhook.trim()) return setNotice("请输入飞书自定义机器人 Webhook");
    setSaving(true);
    try {
      await api.setFeishu(webhook.trim());
      setWebhook("");
      setNotice("飞书 Webhook 已安全保存到系统凭据库");
      await refresh();
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "飞书配置保存失败");
    } finally {
      setSaving(false);
    }
  };
  const exportDiagnostics = async () => {
    try {
      const report = await api.exportDiagnostics();
      const blob = new Blob([JSON.stringify(report, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = `ai-rpa-diagnostics-${new Date().toISOString().replaceAll(":", "-")}.json`;
      link.click();
      URL.revokeObjectURL(url);
      setNotice("脱敏诊断包已导出");
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "诊断导出失败");
    }
  };
  const updateHooks = async (remove = false) => {
    try {
      const result = remove ? await api.uninstallHooks() : await api.installHooks();
      const changed = result.items.filter((item) => item.changed).length;
      setNotice(remove ? `已安全移除 ${changed} 个工具的 AI RPA Hook` : `Hook 接入完成：${changed} 个配置已更新；原配置已自动备份`);
      await refresh();
    } catch (reason) {
      setNotice(reason instanceof Error ? reason.message : "Hook 配置更新失败");
    }
  };
  return (
    <AppShell>
      <PageHeader title="接入与诊断" subtitle="统一检查本机节点、IDE 适配器、通知链路与证据存储" action={<button className="button" onClick={() => void refresh()}>{loading ? "检查中…" : "运行只读自检"}</button>} />
      <ErrorBanner message={error} retry={() => void refresh()} />
      {data && <section className="node-card"><div><i className="online-dot large" /><div><strong>{data.device.hostname} · 节点在线</strong><span>{data.device.os} · {data.device.arch} · Agent v{data.nodeVersion}</span></div></div><span className="badge green">健康</span></section>}
      <SectionTitle title="工具接入" />
      <section className="tool-grid">{data?.adapters.map((adapter) => <AdapterCard adapter={adapter} key={adapter.provider} />)}</section>
      <section className="hook-actions"><div><strong>安全安装四工具 Hook</strong><p>合并现有 JSON、写前备份、重复执行幂等；不会覆盖你已有的自动化。</p></div><div><button className="button" onClick={() => void updateHooks(false)}>安装 / 修复 Hook</button><button className="button secondary" onClick={() => void updateHooks(true)}>移除本产品 Hook</button></div></section>
      <section className="two-column diagnostic-panels">
        <Panel title="链路与存储" count={data?.checks.length}>
          {data?.checks.map((check) => <div className="diagnostic-row" key={check.name}><div><strong>{diagnosticLabel(check.name)}</strong><small>{diagnosticLabel(check.message)}</small></div><span className={`badge ${check.status === "PASS" ? "green" : "orange"}`}>{diagnosticLabel(check.status)}</span></div>)}
        </Panel>
        <Panel title="飞书通知">
          <p className="muted">失败和等待人工即时发送，成功任务进入五分钟汇总窗口。通知正文只包含脱敏摘要。</p>
          <div className="notification-summary"><InfoRow label="待发送 Outbox" value={String(data?.counts.pendingOutbox ?? 0)} /><InfoRow label="凭据位置" value="系统钥匙串 / Credential Manager" /><InfoRow label="失败策略" value="指数退避，最多 8 次" /></div>
          <form className="webhook-form" onSubmit={(event) => void saveWebhook(event)}><label><span>飞书机器人 Webhook</span><input type="password" autoComplete="off" value={webhook} onChange={(event) => setWebhook(event.target.value)} placeholder="https://open.feishu.cn/open-apis/bot/v2/hook/…" /></label><button className="button secondary full" disabled={saving}>{saving ? "安全保存中…" : "保存到系统凭据库"}</button></form>
          <button className="button secondary full" onClick={() => void flush()}>立即处理通知队列</button>
          <button className="button secondary full" onClick={() => void exportDiagnostics()}>导出脱敏诊断包</button>
          {notice && <p className="inline-notice">{notice}</p>}
        </Panel>
      </section>
    </AppShell>
  );
}

function NotFoundPage() {
  return <AppShell><PageHeader title="页面不存在" subtitle="该入口可能已经移动" /><EmptyState title="找不到页面" description="返回任务总览继续操作。" /><NavLink className="button inline" to="/">返回总览</NavLink></AppShell>;
}

function AuthenticatedApp() {
  return (
    <Routes>
      <Route path="/" element={<DashboardPage />} />
      <Route path="/tasks" element={<TasksPage />} />
      <Route path="/tasks/:id" element={<TaskDetailPage />} />
      <Route path="/tasks/:id/continue" element={<ContinueTaskPage />} />
      <Route path="/devices" element={<DevicesPage />} />
      <Route path="/integrations" element={<IntegrationsPage />} />
      <Route path="*" element={<NotFoundPage />} />
    </Routes>
  );
}

function LoginPage({ onLogin }: { onLogin: (session: SessionInfo) => void }) {
  const [token, setToken] = useState("");
  const [error, setError] = useState<string>();
  const [submitting, setSubmitting] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!token.trim()) return setError("请输入管理员访问令牌");
    setSubmitting(true);
    try {
      onLogin(await api.login(token));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "登录失败");
    } finally {
      setSubmitting(false);
    }
  };
  return (
    <main className="login-page">
      <section className="login-card">
        <div className="login-brand"><span className="brand-mark">AI</span><span>AI 任务中控台</span></div>
        <span className="login-cloud">ctyun Personal Sync</span>
        <h1>管理所有电脑上的 AI 任务</h1>
        <p>登录后可以查看设备在线状态、跟踪 Codex、Claude、Cursor 和 Antigravity，并安全地下达后续任务。</p>
        <form onSubmit={(event) => void submit(event)}>
          <label><span>管理员访问令牌</span><input type="password" autoFocus autoComplete="current-password" value={token} onChange={(event) => setToken(event.target.value)} placeholder="由服务器管理员提供" /></label>
          {error && <p className="form-error">{error}</p>}
          <button className="button full" disabled={submitting}>{submitting ? "正在验证…" : "登录控制台"}</button>
        </form>
        <small>令牌仅保存在当前浏览器，不会写入服务器日志。</small>
      </section>
    </main>
  );
}

export default function App() {
  const [session, setSession] = useState<SessionInfo>();
  const [loading, setLoading] = useState(true);
  const [loginRequired, setLoginRequired] = useState(false);
  const [startupError, setStartupError] = useState<string>();
  const load = useCallback(async () => {
    setLoading(true);
    try {
      const current = await api.session();
      setSession(current.mode ? current : { mode: "LOCAL", authenticated: true, version: "0.2.0" });
      setLoginRequired(false);
      setStartupError(undefined);
    } catch (reason) {
      if (reason instanceof Error && "status" in reason && (reason as ApiError).status === 401) {
        setLoginRequired(true);
      } else {
        setStartupError(reason instanceof Error ? reason.message : "无法连接服务");
      }
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => { void load(); }, [load]);
  if (loading) return <main className="boot-page"><span className="brand-mark">AI</span><strong>正在连接任务中控台…</strong></main>;
  if (loginRequired) return <LoginPage onLogin={(value) => { setSession(value); setLoginRequired(false); }} />;
  if (!session || startupError) return <main className="boot-page"><span className="brand-mark">AI</span><strong>暂时无法连接</strong><p>{startupError}</p><button className="button" onClick={() => void load()}>重试</button></main>;
  return <SessionContext.Provider value={session}><AuthenticatedApp /></SessionContext.Provider>;
}
