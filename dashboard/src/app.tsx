import { useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { flexRender, getCoreRowModel, getSortedRowModel, useReactTable, type ColumnDef, type SortingState } from "@tanstack/react-table";
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";
import {
  ArrowUpDown,
  BarChart3,
  Bell,
  ChevronDown,
  CheckCircle2,
  ClipboardList,
  Command,
  FileSearch,
  Gauge,
  HardDrive,
  LogIn,
  LogOut,
  MonitorUp,
  Moon,
  Network,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Rocket,
  Rows3,
  Save,
  Search,
  ServerCog,
  ShieldCheck,
  SkipForward,
  Sun,
  Trash2,
  UserRound,
  X,
  type LucideIcon
} from "lucide-react";
import commandMarkUrl from "@/assets/synapse-command-mark.png";
import emptyStateUrl from "@/assets/synapse-empty-state.webp";
import statusIconsUrl from "@/assets/synapse-status-icons.webp";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { StatusBadge } from "@/components/ui/badge";
import {
  AgentPeek,
  AppShell,
  DataTable,
  EmptyState,
  FleetRow,
  MetricRow,
  PageHeader,
  RawValue,
  Section,
  StatCard,
  ToolCallCard,
  TranscriptTurn
} from "@/primitives";
import {
  buildAgents,
  buildToolCalls,
  deleteDashboardView,
  fetchAuditQuery,
  fetchTemplates,
  fetchDashboardAuthStatus,
  fetchDashboardState,
  fetchModels,
  fetchSavedViews,
  forceReleaseLease,
  handoffLease,
  killAgent,
  loginDashboard,
  logoutDashboard,
  panelData,
  pauseTimeline,
  pruneTargetClaims,
  registerApiModel,
  resumeTimeline,
  saveDashboardView,
  spawnAgent,
  type AgentSummary,
  type AuditQueryFilters,
  type AuditQueryResponse,
  type AuditQueryRow,
  type DashboardAuthStatus,
  type DashboardControlResponse,
  type DashboardRouteReadback,
  type DashboardSavedView,
  type DashboardState,
  type FleetStatus,
  type ModelRow,
  type AgentKillResponse,
  type SpawnAgentResponse,
  type TimelineControlResponse
} from "@/lib/dashboard-state";
import { asArray, asRecord, cn, nsToTime, rawText, timeAgo, unixMsToTime } from "@/lib/utils";
import { useUiStore, type DashboardRouteId, type Density, type Theme } from "@/store/ui-store";

type RouteDefinition = {
  id: DashboardRouteId;
  label: string;
  title: string;
  icon: LucideIcon;
};

const routeDefinitions: RouteDefinition[] = [
  { id: "fleet", label: "Fleet", title: "Fleet Overview", icon: Gauge },
  { id: "agent", label: "Agent", title: "Agent Detail", icon: UserRound },
  { id: "tasks", label: "Tasks", title: "Task Board", icon: ClipboardList },
  { id: "approvals", label: "Approvals", title: "Approvals Inbox", icon: CheckCircle2 },
  { id: "analytics", label: "Analytics", title: "Cost & Token Analytics", icon: BarChart3 },
  { id: "timeline", label: "Timeline", title: "Timeline", icon: Rows3 },
  { id: "system", label: "System", title: "System Status", icon: ServerCog },
  { id: "audit", label: "Audit", title: "Audit Explorer", icon: ShieldCheck }
];

const auditTextFields = ["cursor", "start_ts_ns", "end_ts_ns", "session_id", "tool", "status", "error_code", "row_kind"] as const;

function defaultAuditFilters(): AuditQueryFilters {
  const sixHoursMs = 6 * 60 * 60 * 1000;
  return {
    limit: "100",
    scan_limit: "1000",
    row_kind: "all",
    start_ts_ns: (BigInt(Date.now() - sixHoursMs) * 1_000_000n).toString()
  };
}

function auditCursorFilters(keyHex: string): AuditQueryFilters {
  return {
    limit: "1",
    scan_limit: "1",
    cursor: keyHex,
    row_kind: "all"
  };
}

function sanitizeAuditFilters(value: unknown): AuditQueryFilters {
  const source = asRecord(value);
  const base = defaultAuditFilters();
  const next: AuditQueryFilters = {
    limit: rawText(source.limit || base.limit),
    scan_limit: rawText(source.scan_limit || base.scan_limit)
  };
  for (const field of auditTextFields) {
    const text = rawText(source[field]);
    if (text) next[field] = text;
  }
  if (!next.row_kind) next.row_kind = "all";
  return next;
}

function shortKey(value?: string | null) {
  if (!value) return "";
  return value.length > 18 ? `${value.slice(0, 10)}...${value.slice(-6)}` : value;
}

export function App() {
  const density = useUiStore((state) => state.density);
  const setDensity = useUiStore((state) => state.setDensity);
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);
  const route = useUiStore((state) => state.route);
  const setRoute = useUiStore((state) => state.setRoute);
  const savedViewId = useUiStore((state) => state.savedViewId);
  const setSavedViewId = useUiStore((state) => state.setSavedViewId);
  const selectedAgentId = useUiStore((state) => state.selectedAgentId);
  const setSelectedAgentId = useUiStore((state) => state.setSelectedAgentId);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [hashRouteReady, setHashRouteReady] = useState(false);
  const [savedViewName, setSavedViewName] = useState("");
  const [savedViewError, setSavedViewError] = useState("");
  const [auditFilters, setAuditFilters] = useState<AuditQueryFilters>(() => defaultAuditFilters());

  const authQuery = useQuery({
    queryKey: ["dashboard-auth"],
    queryFn: fetchDashboardAuthStatus,
    retry: false
  });
  const query = useQuery({
    queryKey: ["dashboard-state"],
    queryFn: fetchDashboardState,
    enabled: authQuery.data?.authenticated === true
  });
  const savedViewsQuery = useQuery({
    queryKey: ["dashboard-saved-views"],
    queryFn: fetchSavedViews,
    enabled: authQuery.data?.authenticated === true
  });

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.density = density;
  }, [theme, density]);

  useEffect(() => {
    const syncFromHash = () => {
      const hashRoute = routeFromHash(window.location.hash);
      if (hashRoute && useUiStore.getState().route !== hashRoute) {
        setRoute(hashRoute);
      }
    };
    syncFromHash();
    setHashRouteReady(true);
    window.addEventListener("hashchange", syncFromHash);
    return () => window.removeEventListener("hashchange", syncFromHash);
  }, [setRoute]);

  useEffect(() => {
    if (!hashRouteReady) return;
    const expected = `#/${route}`;
    if (window.location.hash !== expected) {
      window.history.replaceState(null, "", expected);
    }
  }, [hashRouteReady, route]);

  useEffect(() => {
    let icon = document.querySelector<HTMLLinkElement>('link[rel="icon"][data-synapse-generated="true"]');
    if (!icon) {
      icon = document.createElement("link");
      icon.rel = "icon";
      icon.type = "image/png";
      icon.dataset.synapseGenerated = "true";
      document.head.appendChild(icon);
    }
    icon.href = commandMarkUrl;
  }, []);

  const agents = useMemo(() => buildAgents(query.data), [query.data]);
  const toolCalls = useMemo(() => buildToolCalls(query.data), [query.data]);
  const attentionAgents = useMemo(
    () => agents.filter((agent) => ["stuck", "needs_input", "awaiting_approval", "ready_for_review"].includes(agent.status)),
    [agents]
  );
  const selectedAgent = agents.find((agent) => agent.id === selectedAgentId) ?? attentionAgents[0] ?? agents[0];

  useEffect(() => {
    if (!selectedAgentId && selectedAgent) {
      setSelectedAgentId(selectedAgent.id);
    }
  }, [selectedAgentId, selectedAgent, setSelectedAgentId]);

  const attentionCount = attentionAgents.length;
  useEffect(() => {
    document.title = attentionCount ? `${attentionCount} awaiting input - Synapse Command Center` : "Synapse Command Center";
  }, [attentionCount]);

  const advanceAttention = () => {
    if (attentionAgents.length === 0) return;
    const current = attentionAgents.findIndex((agent) => agent.id === selectedAgent?.id);
    const next = attentionAgents[(current + 1 + attentionAgents.length) % attentionAgents.length];
    setSelectedAgentId(next.id);
    setRoute("agent");
  };

  const focusAuditKey = (keyHex: string) => {
    if (!keyHex) return;
    setAuditFilters(auditCursorFilters(keyHex));
    setRoute("audit");
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (isEditableShortcutTarget(event.target)) return;
      const key = event.key.toLowerCase();
      const plainKey = !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
      if (((event.ctrlKey || event.metaKey) && key === "k") || (plainKey && (key === "/" || key === "p"))) {
        event.preventDefault();
        setPaletteOpen(true);
      }
      if ((event.altKey && key === "n") || (plainKey && key === "n")) {
        event.preventDefault();
        advanceAttention();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const state = query.data;
  const freshnessMs = state ? Date.now() - state.generated_at_unix_ms : undefined;
  const stale = query.isError || (freshnessMs !== undefined && freshnessMs > 10000);
  const activeRoute = routeDefinitions.find((item) => item.id === route) ?? routeDefinitions[0];

  const commands = useMemo(
    () => [
      ...routeDefinitions.map((item) => ({
        id: `route-${item.id}`,
        label: item.title,
        group: "Views",
        icon: item.icon,
        action: () => setRoute(item.id)
      })),
      {
        id: "attention-next",
        label: "Next attention",
        group: "Controls",
        icon: Bell,
        action: advanceAttention
      },
      {
        id: "refresh",
        label: "Refresh dashboard state",
        group: "Controls",
        icon: RefreshCw,
        action: () => {
          query.refetch();
          savedViewsQuery.refetch();
        }
      },
      ...agents.slice(0, 8).map((agent) => ({
        id: `agent-${agent.id}`,
        label: agent.id,
        group: "Agents",
        icon: UserRound,
        action: () => {
          setSelectedAgentId(agent.id);
          setRoute("agent");
        }
      }))
    ],
    [agents, query, savedViewsQuery, setRoute, setSelectedAgentId]
  );

  if (authQuery.data?.authenticated !== true) {
    return (
      <AppShell sidebar={<Sidebar route={route} setRoute={setRoute} state={state} auth={authQuery.data} />}>
        <LoginView
          auth={authQuery.data}
          pending={authQuery.isLoading}
          onAuthenticated={() => {
            authQuery.refetch();
            query.refetch();
            savedViewsQuery.refetch();
          }}
        />
      </AppShell>
    );
  }

  const applySavedView = (view?: DashboardSavedView) => {
    setSavedViewError("");
    if (!view) {
      setSavedViewId(null);
      setSavedViewName("");
      return;
    }
    setSavedViewId(view.view_id);
    setSavedViewName(view.name);
    if (isDashboardRouteId(view.route)) setRoute(view.route);
    const filters = asRecord(view.filters);
    const agentId = rawText(filters.selectedAgentId);
    if (agentId) setSelectedAgentId(agentId);
    if (isDensity(filters.density)) setDensity(filters.density);
    if (isTheme(filters.theme)) setTheme(filters.theme);
    if (filters.audit) setAuditFilters(sanitizeAuditFilters(filters.audit));
  };

  const saveCurrentView = async () => {
    const name = savedViewName.trim();
    if (!name) return;
    setSavedViewError("");
    try {
      const saved = await saveDashboardView({
        view_id: savedViewId || undefined,
        name,
        route,
        filters: {
          selectedAgentId,
          density,
          theme,
          audit: auditFilters
        }
      });
      setSavedViewId(saved.view.view_id);
      setSavedViewName(saved.view.name);
      await savedViewsQuery.refetch();
    } catch (error) {
      setSavedViewError(rawText(error) || "Saved view write failed");
    }
  };

  const deleteCurrentView = async () => {
    if (!savedViewId) return;
    setSavedViewError("");
    try {
      await deleteDashboardView(savedViewId);
      setSavedViewId(null);
      setSavedViewName("");
      await savedViewsQuery.refetch();
    } catch (error) {
      setSavedViewError(rawText(error) || "Saved view delete failed");
    }
  };

  return (
    <AppShell sidebar={<Sidebar route={route} setRoute={setRoute} state={state} auth={authQuery.data} />}>
      <PageHeader
        title={activeRoute.title}
        subtitle={
          <span className={stale ? "text-warning-fg" : "text-secondary"}>
            {query.isError ? rawText(query.error) : `Updated ${freshnessMs === undefined ? "pending" : timeAgo(freshnessMs)} ago`}
          </span>
        }
        actions={
          <>
            <SavedViewControls
              views={savedViewsQuery.data?.views ?? []}
              selectedId={savedViewId}
              name={savedViewName}
              error={savedViewError}
              onNameChange={setSavedViewName}
              onSelect={applySavedView}
              onSave={saveCurrentView}
              onDelete={deleteCurrentView}
              saving={savedViewsQuery.isFetching}
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="icon" variant="ghost" onClick={() => setPaletteOpen(true)} aria-label="Open command palette" aria-keyshortcuts="/ P Control+K">
                  <Command aria-hidden="true" className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Command palette</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="icon" variant="ghost" onClick={() => query.refetch()} aria-label="Refresh dashboard state">
                  <RefreshCw aria-hidden="true" className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Refresh</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() =>
                    logoutDashboard().then(() => {
                      authQuery.refetch();
                    })
                  }
                  aria-label="Lock dashboard"
                >
                  <LogOut aria-hidden="true" className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Lock</TooltipContent>
            </Tooltip>
            <DensityControl density={density} setDensity={setDensity} />
            <label className="flex items-center gap-2 text-sm text-secondary">
              {theme === "dark" ? <Moon aria-hidden="true" className="h-4 w-4" /> : <Sun aria-hidden="true" className="h-4 w-4" />}
              <Switch checked={theme === "light"} onCheckedChange={(checked) => setTheme(checked ? "light" : "dark")} aria-label="Toggle light theme" />
            </label>
          </>
        }
      />

      {route === "fleet" ? (
        <FleetView
          state={state}
          agents={agents}
          attentionAgents={attentionAgents}
          selectedAgent={selectedAgent}
          setSelectedAgentId={setSelectedAgentId}
          attentionCount={attentionCount}
          stale={stale}
          toolCalls={toolCalls}
          advanceAttention={advanceAttention}
          onSpawned={() => query.refetch()}
          onAuditKeySelect={focusAuditKey}
        />
      ) : null}
      {route === "agent" ? <AgentView state={state} selectedAgent={selectedAgent} toolCalls={toolCalls} onAuditKeySelect={focusAuditKey} /> : null}
      {route === "tasks" ? <TasksView agents={agents} attentionCount={attentionCount} /> : null}
      {route === "approvals" ? <ApprovalsView state={state} /> : null}
      {route === "analytics" ? <AnalyticsView state={state} agents={agents} attentionCount={attentionCount} stale={stale} /> : null}
      {route === "timeline" ? <TimelineView state={state} toolCalls={toolCalls} /> : null}
      {route === "system" ? <SystemView state={state} stale={stale} onRefresh={() => query.refetch()} /> : null}
      {route === "audit" ? (
        <AuditView state={state} toolCalls={toolCalls} filters={auditFilters} onFiltersChange={setAuditFilters} />
      ) : null}

      <CommandPalette open={paletteOpen} commands={commands} onClose={() => setPaletteOpen(false)} />
    </AppShell>
  );
}

function LoginView({
  auth,
  pending,
  onAuthenticated
}: {
  auth?: DashboardAuthStatus;
  pending: boolean;
  onAuthenticated: () => void;
}) {
  const [credential, setCredential] = useState("");
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError("");
    setSubmitting(true);
    try {
      await loginDashboard(credential);
      setCredential("");
      onAuthenticated();
    } catch (loginError) {
      setError(rawText(loginError) || "Access denied");
    } finally {
      setSubmitting(false);
    }
  };
  return (
    <>
      <PageHeader
        title="Dashboard Access"
        subtitle={<span>{pending ? "Checking session" : auth?.authenticated ? "Session active" : "Session required"}</span>}
        actions={<span className="text-sm text-secondary">Loopback only</span>}
      />
      <Section
        title="Unlock"
        tier="overview"
        questions={["Is a dashboard session active?", "Can the operator mint a cookie session?", "Did the login fail closed?"]}
      >
        <form className="max-w-md space-y-3" onSubmit={submit}>
          <label className="block text-sm text-secondary">
            <span className="mb-1 block text-label font-medium uppercase text-muted">Access token</span>
            <input
              className="h-10 w-full rounded-md border border-border bg-surface-1 px-3 font-mono text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
              type="password"
              value={credential}
              autoComplete="off"
              onChange={(event) => setCredential(event.target.value)}
            />
          </label>
          {error ? <div className="text-sm text-danger-fg">{error}</div> : null}
          <Button type="submit" variant="primary" disabled={!credential.trim() || submitting}>
            <LogIn aria-hidden="true" className="h-4 w-4" />
            Unlock
          </Button>
        </form>
      </Section>
    </>
  );
}

function Sidebar({
  route,
  setRoute,
  state,
  auth
}: {
  route: DashboardRouteId;
  setRoute: (route: DashboardRouteId) => void;
  state?: DashboardState;
  auth?: DashboardAuthStatus;
}) {
  const health = asRecord(panelData(state?.daemon));
  return (
    <nav className="space-y-4" aria-label="Dashboard">
      <button type="button" className="flex w-full items-center gap-3 text-left" onClick={() => setRoute("fleet")}>
        <img src={commandMarkUrl} alt="" className="h-10 w-10 rounded-lg border border-border bg-surface-2 object-cover" />
        <span className="min-w-0">
          <span className="block text-md font-semibold text-primary">Synapse</span>
          <span className="block truncate text-xs text-muted">{rawText(health.version || "dashboard")}</span>
        </span>
      </button>
      <div className="grid gap-1">
        {routeDefinitions.map((item) => (
          <SidebarItem key={item.id} item={item} active={route === item.id} onSelect={() => setRoute(item.id)} />
        ))}
      </div>
      <div className="rounded-lg border border-border bg-surface-2 p-3">
        <div className="text-label font-medium uppercase text-muted">Loopback</div>
        <div className="mt-1 truncate font-mono text-sm text-primary">{state?.bind_addr || "pending"}</div>
      </div>
      <div className="rounded-lg border border-border bg-surface-2 p-3">
        <div className="text-label font-medium uppercase text-muted">Auth</div>
        <div className="mt-1 truncate font-mono text-sm text-primary">{auth?.authenticated ? auth.method : "locked"}</div>
      </div>
      <img src={statusIconsUrl} alt="" className="w-full rounded-lg border border-border bg-surface-2 object-cover" />
    </nav>
  );
}

function SidebarItem({ item, active, onSelect }: { item: RouteDefinition; active: boolean; onSelect: () => void }) {
  const Icon = item.icon;
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "flex min-h-10 w-full items-center gap-2 rounded-md px-3 text-left text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring",
        active ? "bg-surface-2 text-primary" : "text-secondary hover:bg-surface-2 hover:text-primary"
      )}
    >
      <Icon aria-hidden="true" className="h-4 w-4" />
      {item.label}
    </button>
  );
}

function SavedViewControls({
  views,
  selectedId,
  name,
  error,
  onNameChange,
  onSelect,
  onSave,
  onDelete,
  saving
}: {
  views: DashboardSavedView[];
  selectedId: string | null;
  name: string;
  error: string;
  onNameChange: (value: string) => void;
  onSelect: (view?: DashboardSavedView) => void;
  onSave: () => void;
  onDelete: () => void;
  saving: boolean;
}) {
  return (
    <div className="flex max-w-full flex-wrap items-center gap-2">
      <label className="sr-only" htmlFor="saved-view-select">
        Saved view
      </label>
      <select
        id="saved-view-select"
        className="h-9 min-w-36 rounded-md border border-border bg-surface-1 px-2 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
        value={selectedId ?? ""}
        onChange={(event) => onSelect(views.find((view) => view.view_id === event.target.value))}
      >
        <option value="">Default view</option>
        {views.map((view) => (
          <option key={view.view_id} value={view.view_id}>
            {view.name}
          </option>
        ))}
      </select>
      <label className="sr-only" htmlFor="saved-view-name">
        View name
      </label>
      <input
        id="saved-view-name"
        className="h-9 w-36 rounded-md border border-border bg-surface-1 px-2 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
        value={name}
        placeholder="View name"
        onChange={(event) => onNameChange(event.target.value)}
      />
      <Tooltip>
        <TooltipTrigger asChild>
          <Button size="icon" variant="ghost" onClick={onSave} disabled={!name.trim() || saving} aria-label="Save view">
            <Save aria-hidden="true" className="h-4 w-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Save view</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button size="icon" variant="ghost" onClick={onDelete} disabled={!selectedId || saving} aria-label="Delete view">
            <Trash2 aria-hidden="true" className="h-4 w-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Delete view</TooltipContent>
      </Tooltip>
      {error ? <span className="max-w-56 truncate text-xs text-danger-fg">{error}</span> : null}
    </div>
  );
}

function DensityControl({
  density,
  setDensity
}: {
  density: Density;
  setDensity: (density: Density) => void;
}) {
  return (
    <div className="inline-flex rounded-lg border border-border bg-surface-1 p-1" aria-label="Density">
      {(["comfortable", "compact"] as const).map((value) => (
        <button
          key={value}
          type="button"
          onClick={() => setDensity(value)}
          className={`h-8 rounded-md px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring ${density === value ? "bg-surface-2 text-primary" : "text-muted hover:text-primary"}`}
        >
          {value === "comfortable" ? "Comfort" : "Compact"}
        </button>
      ))}
    </div>
  );
}

function CommandPalette({
  open,
  commands,
  onClose
}: {
  open: boolean;
  commands: Array<{ id: string; label: string; group: string; icon: LucideIcon; action: () => void }>;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (open) {
      setQuery("");
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);
  if (!open) return null;
  const filtered = commands.filter((command) => `${command.group} ${command.label}`.toLowerCase().includes(query.toLowerCase())).slice(0, 12);
  return (
    <div className="fixed inset-0 z-50 bg-black/60 p-4" role="dialog" aria-modal="true" aria-label="Command palette" onMouseDown={onClose}>
      <div className="mx-auto mt-16 max-w-2xl rounded-lg border border-border bg-surface-1 shadow-xl" onMouseDown={(event) => event.stopPropagation()}>
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search aria-hidden="true" className="h-4 w-4 text-muted" />
          <input
            ref={inputRef}
            className="h-10 min-w-0 flex-1 bg-transparent text-sm text-primary outline-none"
            value={query}
            placeholder="Search commands"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") onClose();
              if (event.key === "Enter" && filtered[0]) {
                filtered[0].action();
                onClose();
              }
            }}
          />
          <Button size="icon" variant="ghost" onClick={onClose} aria-label="Close command palette">
            <X aria-hidden="true" className="h-4 w-4" />
          </Button>
        </div>
        <div className="max-h-[60vh] overflow-auto p-2">
          {filtered.length ? (
            filtered.map((command) => {
              const Icon = command.icon;
              return (
                <button
                  key={command.id}
                  type="button"
                  className="flex min-h-11 w-full items-center gap-3 rounded-md px-3 text-left text-sm text-secondary hover:bg-surface-2 hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring"
                  onClick={() => {
                    command.action();
                    onClose();
                  }}
                >
                  <Icon aria-hidden="true" className="h-4 w-4 text-info" />
                  <span className="min-w-0 flex-1 truncate">{command.label}</span>
                  <span className="text-label uppercase text-muted">{command.group}</span>
                </button>
              );
            })
          ) : (
            <EmptyState title="No matching commands" />
          )}
        </div>
      </div>
    </div>
  );
}

function FleetView({
  state,
  agents,
  attentionAgents,
  selectedAgent,
  setSelectedAgentId,
  attentionCount,
  stale,
  toolCalls,
  advanceAttention,
  onSpawned,
  onAuditKeySelect
}: {
  state?: DashboardState;
  agents: AgentSummary[];
  attentionAgents: AgentSummary[];
  selectedAgent?: AgentSummary;
  setSelectedAgentId: (id: string) => void;
  attentionCount: number;
  stale: boolean;
  toolCalls: ReturnType<typeof buildToolCalls>;
  advanceAttention: () => void;
  onSpawned: () => void;
  onAuditKeySelect: (keyHex: string) => void;
}) {
  const [killingId, setKillingId] = useState("");
  const [killError, setKillError] = useState("");
  const [killReadback, setKillReadback] = useState<AgentKillResponse | null>(null);

  const killFromFleet = async (agent: AgentSummary) => {
    if (!agent.killable || !agent.killId || killingId) return;
    setKillingId(agent.killId);
    setKillError("");
    try {
      const readback = await killAgent({
        session_id: agent.killId,
        grace_ms: 0,
        interrupt_first: false
      });
      setKillReadback(readback);
      onSpawned();
    } catch (error) {
      setKillError(rawText(error) || "Agent kill failed");
    } finally {
      setKillingId("");
    }
  };

  return (
    <>
      <OverviewBand state={state} agents={agents} attentionCount={attentionCount} stale={stale} />
      <Section
        title="Spawn Console"
        tier="triage"
        questions={["Which models can I launch right now?", "How do I add a cloud API model like DeepSeek?", "Did the spawn succeed, step by step?"]}
      >
        <SpawnConsole onSpawned={onSpawned} />
      </Section>
      <div className="grid gap-6 xl:grid-cols-[minmax(0,1fr)_minmax(20rem,0.42fr)]">
        <div className="min-w-0">
          <Section
            title="Attention Groups"
            tier="triage"
            questions={["Which agents need a human now?", "Which session should I inspect first?", "What changed since the last refresh?"]}
            actions={
              <Button variant="secondary" size="sm" onClick={advanceAttention} disabled={attentionAgents.length === 0} aria-keyshortcuts="N Alt+N">
                <Bell aria-hidden="true" className="h-4 w-4" />
                Next
              </Button>
            }
          >
            <FleetList
              agents={attentionAgents.length ? attentionAgents : agents}
              selectedId={selectedAgent?.id}
              onSelect={setSelectedAgentId}
              onKill={killFromFleet}
              killingId={killingId}
            />
          </Section>
          <ToolActivity toolCalls={toolCalls} onAuditKeySelect={onAuditKeySelect} />
          <Section
            title="Fleet Table"
            tier="drill-down"
            questions={["Which sessions are live?", "Which rows are stale?", "Which row links to detail?"]}
          >
            <FleetTable agents={agents} onSelect={setSelectedAgentId} onKill={killFromFleet} killingId={killingId} />
            {killError ? <div className="mt-3 rounded-md border border-danger-border bg-danger-bg p-3 text-sm text-danger-fg">{killError}</div> : null}
            {killReadback ? <RawValue value={killReadback} label="Kill readback" /> : null}
          </Section>
        </div>
        <aside className="min-w-0">
          <Section
            title="Peek Panel"
            tier="drill-down"
            questions={["Why is this agent in its current state?", "Which detail surface proves it?", "Is raw verification available without flooding the page?"]}
          >
            <AgentPeek agent={selectedAgent} />
          </Section>
          <Section
            title="System Shape"
            tier="overview"
            questions={["Is storage pressure rising?", "Which column family is largest?", "Is the daemon still local?"]}
          >
            <SystemShape state={state} />
          </Section>
        </aside>
      </div>
      <TranscriptSamples state={state} />
    </>
  );
}

function AgentView({
  state,
  selectedAgent,
  toolCalls,
  onAuditKeySelect
}: {
  state?: DashboardState;
  selectedAgent?: AgentSummary;
  toolCalls: ReturnType<typeof buildToolCalls>;
  onAuditKeySelect: (keyHex: string) => void;
}) {
  return (
    <div className="grid gap-6 xl:grid-cols-[minmax(0,0.42fr)_minmax(0,1fr)]">
      <Section
        title="Agent Surfaces"
        tier="drill-down"
        questions={["Which agent is selected?", "Which state explains the current badge?", "Can raw verification stay collapsed?"]}
      >
        <AgentPeek agent={selectedAgent} />
      </Section>
      <div className="min-w-0">
        <ToolActivity toolCalls={toolCalls} onAuditKeySelect={onAuditKeySelect} />
        <TranscriptSamples state={state} />
      </div>
    </div>
  );
}

function TasksView({ agents, attentionCount }: { agents: AgentSummary[]; attentionCount: number }) {
  const reviewAgents = agents.filter((agent) => agent.status === "ready_for_review").length;
  return (
    <>
      <Section
        title="Task Queue"
        tier="overview"
        questions={["How many review items need action?", "Which rows are attention candidates?", "Is the shell route stable for #924?"]}
      >
        <div className="grid gap-4 md:grid-cols-3">
          <StatCard label="Attention" value={attentionCount} status={attentionCount ? "needs_input" : "done"} />
          <StatCard label="Review" value={reviewAgents} status={reviewAgents ? "ready_for_review" : "idle"} />
          <StatCard label="Shell" value="ready" status="working" />
        </div>
      </Section>
      <Section
        title="Board"
        tier="triage"
        questions={["Which task lane is populated?", "Which attempt needs review?", "Where will queue transitions appear?"]}
      >
        <EmptyStateArt title="No task board rows in this shell feed" />
      </Section>
    </>
  );
}

function ApprovalsView({ state }: { state?: DashboardState }) {
  return (
    <div className="grid gap-6 xl:grid-cols-3">
      <ApprovalPanel title="Approvals" panel={state?.approvals} />
      <ApprovalPanel title="Suggestions" panel={state?.suggestions} />
      <ApprovalPanel title="Armed Runs" panel={state?.armed_runs} />
    </div>
  );
}

function ApprovalPanel({ title, panel }: { title: string; panel?: DashboardState["approvals"] }) {
  const data = asRecord(panelData(panel));
  const rows = asArray<Record<string, unknown>>(data.rows);
  return (
    <Section
      title={title}
      tier="triage"
      questions={["Which approvals are pending?", "What source row backs this list?", "Does raw detail stay collapsed?"]}
    >
      <div className="space-y-3">
        <StatCard label="Rows" value={rows.length} status={rows.length ? "awaiting_approval" : "done"} delta={rawText(data.tool || panel?.source)} />
        {rows.length ? rows.slice(0, 4).map((row, index) => <RawValue key={index} value={row} label="Approval row" />) : <EmptyStateArt title="No approval rows" />}
      </div>
    </Section>
  );
}

function AnalyticsView({ state, agents, attentionCount, stale }: { state?: DashboardState; agents: AgentSummary[]; attentionCount: number; stale: boolean }) {
  return (
    <>
      <OverviewBand state={state} agents={agents} attentionCount={attentionCount} stale={stale} />
      <Section
        title="Storage Distribution"
        tier="triage"
        questions={["Which store has the most rows?", "Is pressure elevated?", "Which counters changed since refresh?"]}
      >
        <SystemShape state={state} />
      </Section>
    </>
  );
}

function TimelineView({ state, toolCalls }: { state?: DashboardState; toolCalls: ReturnType<typeof buildToolCalls> }) {
  return (
    <>
      <ToolActivity toolCalls={toolCalls} />
      <TranscriptSamples state={state} />
    </>
  );
}

function SystemView({
  state,
  stale,
  onRefresh
}: {
  state?: DashboardState;
  stale: boolean;
  onRefresh: () => void | Promise<unknown>;
}) {
  const health = asRecord(panelData(state?.daemon));
  const subsystems = asRecord(health.subsystems);
  const storage = asRecord(panelData(state?.storage));
  const pressure = asRecord(storage.pressure_level);
  const timeline = asRecord(panelData(state?.timeline));
  const recorder = asRecord(timeline.recorder);
  const lease = asRecord(panelData(state?.lease));
  const targetClaims = asRecord(panelData(state?.target_claims));
  const claims = asArray<Record<string, unknown>>(targetClaims.claims);
  const sessions = asArray<Record<string, unknown>>(asRecord(panelData(state?.sessions)).sessions);
  const commandAuditRows = asArray<Record<string, unknown>>(asRecord(panelData(state?.command_audit)).rows);
  const hiddenDesktops = asArray<Record<string, unknown>>(asRecord(panelData(state?.hidden_desktops)).rows);
  const cdpAttachments = asArray<Record<string, unknown>>(asRecord(panelData(state?.cdp_attachments)).rows);
  const shellJobsData = asRecord(panelData(state?.shell_jobs));
  const shellJobs = asArray<Record<string, unknown>>(shellJobsData.rows);
  const events = asRecord(panelData(state?.events));
  const pressureName = rawText(pressure.name || pressure.level || pressure.value || storage.pressure_level || "unknown");
  const pressureStatus: FleetStatus = /level[34]|l[34]|refus/i.test(pressureName) ? "stuck" : /level[12]|l[12]/i.test(pressureName) ? "needs_input" : "done";
  const recorderPaused = recorder.paused === true;
  const [recorderBusy, setRecorderBusy] = useState<"pause" | "resume" | "">("");
  const [recorderError, setRecorderError] = useState("");
  const [lastRecorderControl, setLastRecorderControl] = useState<TimelineControlResponse | null>(null);
  const ownerSessionId = rawText(lease.owner_session_id);
  const [leaseBusy, setLeaseBusy] = useState<"force" | "handoff" | "prune" | "">("");
  const [leaseError, setLeaseError] = useState("");
  const [lastLeaseControl, setLastLeaseControl] = useState<DashboardControlResponse | null>(null);
  const [forceReleaseOwner, setForceReleaseOwner] = useState("");
  const [forceReleaseConfirmed, setForceReleaseConfirmed] = useState(false);
  const [handoffToSession, setHandoffToSession] = useState("");
  const [handoffTtlMs, setHandoffTtlMs] = useState("5000");
  const forceReleaseTarget = forceReleaseOwner.trim() || ownerSessionId;
  const contentionRows = commandAuditRows.filter((row) => {
    const errorCode = rawText(row.error_code);
    return errorCode === "ACTION_FOREGROUND_LEASE_BUSY" || rawText(row).includes("ACTION_FOREGROUND_LEASE_BUSY");
  });
  const contentionGroups: Record<string, unknown>[] = Object.values(
    contentionRows.reduce<Record<string, { session_id: string; count: number; last_ts_ns: number; tools: string[] }>>((groups, row) => {
      const payload = asRecord(row.payload_bounded);
      const sessionId =
        rawText(row.actor_session_id || payload.session_id || payload.requesting_session_id || row.target_session_id) || "unknown";
      const group = groups[sessionId] ?? {
        session_id: sessionId,
        count: 0,
        last_ts_ns: 0,
        tools: []
      };
      group.count += 1;
      group.last_ts_ns = Math.max(group.last_ts_ns, Number(row.ts_ns || 0));
      const tool = rawText(row.tool || row.verb || "unknown");
      if (tool && !group.tools.includes(tool)) group.tools.push(tool);
      groups[sessionId] = group;
      return groups;
    }, {})
  )
    .sort((a, b) => b.last_ts_ns - a.last_ts_ns)
    .map((group) => ({
      session_id: group.session_id,
      count: group.count,
      last_ts_ns: group.last_ts_ns,
      tools: group.tools
    }));

  const submitRecorderControl = async (mode: "pause" | "resume") => {
    setRecorderBusy(mode);
    setRecorderError("");
    try {
      const response = mode === "pause" ? await pauseTimeline() : await resumeTimeline();
      setLastRecorderControl(response);
      await onRefresh();
    } catch (error) {
      setRecorderError(error instanceof Error ? error.message : String(error));
    } finally {
      setRecorderBusy("");
    }
  };

  const submitForceRelease = async () => {
    if (!forceReleaseTarget) return;
    setLeaseBusy("force");
    setLeaseError("");
    try {
      const response = await forceReleaseLease({
        owner_session_id: forceReleaseTarget,
        confirmed: forceReleaseConfirmed
      });
      setLastLeaseControl(response);
      setForceReleaseConfirmed(false);
      await onRefresh();
    } catch (error) {
      setLeaseError(error instanceof Error ? error.message : String(error));
    } finally {
      setLeaseBusy("");
    }
  };

  const submitHandoff = async () => {
    const toSession = handoffToSession.trim();
    if (!ownerSessionId || !toSession) return;
    const ttl = handoffTtlMs.trim() ? Number(handoffTtlMs.trim()) : undefined;
    if (ttl !== undefined && !Number.isFinite(ttl)) {
      setLeaseError("TOOL_PARAMS_INVALID: ttl_ms must be numeric");
      return;
    }
    setLeaseBusy("handoff");
    setLeaseError("");
    try {
      const response = await handoffLease({
        from_session_id: ownerSessionId,
        to_session_id: toSession,
        ttl_ms: ttl
      });
      setLastLeaseControl(response);
      await onRefresh();
    } catch (error) {
      setLeaseError(error instanceof Error ? error.message : String(error));
    } finally {
      setLeaseBusy("");
    }
  };

  const submitPruneClaims = async () => {
    setLeaseBusy("prune");
    setLeaseError("");
    try {
      const response = await pruneTargetClaims();
      setLastLeaseControl(response);
      await onRefresh();
    } catch (error) {
      setLeaseError(error instanceof Error ? error.message : String(error));
    } finally {
      setLeaseBusy("");
    }
  };

  const chromeBridge = asRecord(subsystems.chrome_bridge);
  const action = asRecord(subsystems.action);
  const daemonLifecycle = asRecord(subsystems.daemon_lifecycle);
  const http = asRecord(subsystems.http);

  return (
    <>
      <Section title="Substrate" tier="overview" questions={["Is the daemon healthy?", "Is storage refusing work?", "Is the recorder paused?"]}>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <StatCard label="Health" value={health.ok === true ? "ok" : "check"} status={health.ok === true && !stale ? "done" : "stuck"} delta={`pid ${rawText(health.pid || "unknown")}`} />
          <StatCard label="Storage" value={pressureName || "unknown"} status={pressureStatus} delta={state?.storage.source || "storage_inspect"} />
          <StatCard label="Recorder" value={recorderPaused ? "paused" : "running"} status={recorderPaused ? "needs_input" : "working"} delta={state?.timeline.source || "timeline_stats"} />
          <StatCard label="Shell Jobs" value={rawText(shellJobsData.running_count || 0)} status={Number(shellJobsData.running_count || 0) ? "working" : "idle"} delta={`${rawText(shellJobsData.job_count || 0)} status files`} />
        </div>
      </Section>

      <Section
        title="Daemon"
        tier="triage"
        questions={["Which process owns the server?", "Is Chrome bridge connected?", "Is action routing healthy?"]}
      >
        <div className="grid gap-4 xl:grid-cols-[1fr_1fr]">
          <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
            <div className="mb-2 flex items-center gap-2 text-primary">
              <ServerCog aria-hidden="true" className="h-4 w-4 text-info" />
              <h3 className="text-md font-medium tracking-normal">Runtime</h3>
            </div>
            <MetricRow label="Bind" value={state?.bind_addr || rawText(http.bind_addr)} />
            <MetricRow label="PID" value={rawText(health.pid)} />
            <MetricRow label="Tools" value={rawText(health.tool_count)} />
            <MetricRow label="Lifecycle" value={rawText(daemonLifecycle.status)} />
            <MetricRow label="Auth SoT" value={state?.auth.source || "CF_KV dashboard-auth/v1"} />
          </div>
          <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
            <div className="mb-2 flex items-center gap-2 text-primary">
              <Network aria-hidden="true" className="h-4 w-4 text-info" />
              <h3 className="text-md font-medium tracking-normal">Bridge</h3>
            </div>
            <MetricRow label="Chrome" value={rawText(chromeBridge.status)} />
            <MetricRow label="HTTP" value={rawText(http.status)} />
            <MetricRow label="Action" value={rawText(action.status)} />
            <MetricRow label="SSE" value={rawText(events.active_subscription_count || 0)} />
            <MetricRow label="Tool SoT" value={state?.daemon.source || "health"} />
          </div>
        </div>
        <div className="mt-3">
          <RawValue value={{ daemon: state?.daemon, chrome_bridge: chromeBridge }} label="Daemon readback" />
        </div>
      </Section>

      <Section
        title="Storage"
        tier="triage"
        questions={["Which store is largest?", "What pressure level is active?", "Which rows back the chart?"]}
      >
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(280px,360px)]">
          <SystemShape state={state} />
          <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
            <div className="mb-2 flex items-center gap-2 text-primary">
              <HardDrive aria-hidden="true" className="h-4 w-4 text-info" />
              <h3 className="text-md font-medium tracking-normal">Pressure</h3>
            </div>
            <MetricRow label="Level" value={pressureName || "unknown"} />
            <MetricRow label="Transitions" value={asArray(storage.pressure_transition_codes).length} />
            <MetricRow label="CF rows" value={Object.keys(asRecord(storage.cf_row_counts)).length} />
            <MetricRow label="CF sizes" value={Object.keys(asRecord(storage.cf_sizes)).length} />
            <MetricRow label="SoT" value={state?.storage.source || "storage_inspect"} />
          </div>
        </div>
      </Section>

      <Section
        title="Recorder"
        tier="triage"
        questions={["Is recording active?", "Do pause and resume hit the live gate?", "Which timeline counts changed?"]}
        actions={
          <>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button type="button" variant="ghost" size="sm" disabled={Boolean(recorderBusy)} onClick={() => submitRecorderControl("pause")}>
                  <Pause aria-hidden="true" className="h-4 w-4" />
                  {recorderBusy === "pause" ? "Pausing" : "Pause"}
                </Button>
              </TooltipTrigger>
              <TooltipContent>Pause timeline recorder</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button type="button" variant="ghost" size="sm" disabled={Boolean(recorderBusy)} onClick={() => submitRecorderControl("resume")}>
                  <Play aria-hidden="true" className="h-4 w-4" />
                  {recorderBusy === "resume" ? "Resuming" : "Resume"}
                </Button>
              </TooltipTrigger>
              <TooltipContent>Resume timeline recorder</TooltipContent>
            </Tooltip>
          </>
        }
      >
        <div className="grid gap-4 xl:grid-cols-3">
          <StatCard label="Rows" value={rawText(timeline.total_rows || 0)} status={timeline.scan_complete === false ? "needs_input" : "done"} delta={`${rawText(timeline.scanned_rows || 0)} scanned`} />
          <StatCard label="Invalid" value={rawText(timeline.invalid_rows || 0)} status={Number(timeline.invalid_rows || 0) ? "stuck" : "done"} delta="CF_TIMELINE decode" />
          <StatCard label="Feeds" value={recorder.clipboard_feed_enabled || recorder.file_activity_feed_enabled ? "enabled" : "base"} status="working" delta={`paused=${rawText(recorder.paused)}`} />
        </div>
        {recorderError ? <div className="mt-3 rounded-lg border border-danger/40 bg-danger/10 p-3 text-sm text-danger">{recorderError}</div> : null}
        {lastRecorderControl ? (
          <div className="mt-3">
            <RawValue value={lastRecorderControl} label="Recorder control readback" />
          </div>
        ) : null}
        <div className="mt-3 grid gap-4 xl:grid-cols-2">
          <RawValue value={timeline.rows_by_kind} label="Rows by kind" />
          <RawValue value={timeline.rows_by_day_utc} label="Rows by day" />
        </div>
      </Section>

      <Section
        title="Claims"
        tier="drill-down"
        questions={["Who holds the input lease?", "Which sessions are waiting?", "Which targets are stale?"]}
        actions={
          <Tooltip>
            <TooltipTrigger asChild>
              <Button type="button" variant="ghost" size="sm" disabled={leaseBusy === "prune"} onClick={submitPruneClaims}>
                <RefreshCw aria-hidden="true" className="h-4 w-4" />
                {leaseBusy === "prune" ? "Pruning" : "Prune"}
              </Button>
            </TooltipTrigger>
            <TooltipContent>Prune expired target claims</TooltipContent>
          </Tooltip>
        }
      >
        <div className="mb-3 grid gap-4 md:grid-cols-3">
          <StatCard label="Lease" value={lease.held === true ? "held" : "free"} status={lease.held === true ? "needs_input" : "done"} delta={ownerSessionId || "no owner"} />
          <StatCard label="Claims" value={rawText(targetClaims.claim_count || claims.length)} status={claims.length ? "working" : "idle"} delta={state?.target_claims.source || "target_claim_status"} />
          <StatCard label="Contention" value={contentionRows.length} status={contentionRows.length ? "needs_input" : "done"} delta={state?.command_audit.source || "CF_ACTION_LOG"} />
        </div>
        <div className="mb-3 grid gap-3 xl:grid-cols-2">
          <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
            <div className="mb-3 flex items-center gap-2 text-primary">
              <X aria-hidden="true" className="h-4 w-4 text-warning" />
              <h3 className="text-md font-medium tracking-normal">Force Release</h3>
            </div>
            <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto]">
              <input
                value={forceReleaseOwner}
                onChange={(event) => setForceReleaseOwner(event.target.value)}
                placeholder={ownerSessionId || "owner session id"}
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 font-mono text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
              />
              <label className="inline-flex h-10 items-center gap-2 rounded-md border border-border bg-surface-2 px-3 text-sm text-primary">
                <input
                  type="checkbox"
                  checked={forceReleaseConfirmed}
                  onChange={(event) => setForceReleaseConfirmed(event.target.checked)}
                  className="h-4 w-4 accent-[rgb(var(--color-info))]"
                />
                Confirm
              </label>
            </div>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={!forceReleaseTarget || !forceReleaseConfirmed || Boolean(leaseBusy)}
                onClick={submitForceRelease}
              >
                <X aria-hidden="true" className="h-4 w-4" />
                {leaseBusy === "force" ? "Releasing" : "Release"}
              </Button>
              <span className="max-w-full overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs text-secondary">
                {forceReleaseTarget || "no target"}
              </span>
            </div>
          </div>
          <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
            <div className="mb-3 flex items-center gap-2 text-primary">
              <SkipForward aria-hidden="true" className="h-4 w-4 text-info" />
              <h3 className="text-md font-medium tracking-normal">Handoff</h3>
            </div>
            <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_7rem]">
              <input
                value={handoffToSession}
                onChange={(event) => setHandoffToSession(event.target.value)}
                placeholder="recipient session id"
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 font-mono text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
              />
              <input
                value={handoffTtlMs}
                onChange={(event) => setHandoffTtlMs(event.target.value)}
                inputMode="numeric"
                aria-label="Lease TTL ms"
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
              />
            </div>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={!ownerSessionId || !handoffToSession.trim() || Boolean(leaseBusy)}
                onClick={submitHandoff}
              >
                <SkipForward aria-hidden="true" className="h-4 w-4" />
                {leaseBusy === "handoff" ? "Handing off" : "Handoff"}
              </Button>
              <span className="max-w-full overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs text-secondary">
                {ownerSessionId || "no holder"}
              </span>
            </div>
          </div>
        </div>
        {leaseError ? <div className="mb-3 rounded-md border border-danger-border bg-danger-bg p-3 text-sm text-danger-fg">{leaseError}</div> : null}
        <div className="mb-3 grid gap-4 xl:grid-cols-2">
          <SystemTable
            title="Contention"
            icon={Gauge}
            rows={contentionGroups}
            empty="No lease denials"
            columns={[
              { id: "session", header: "Waiting Session", cell: ({ row }) => rawText(row.original.session_id) },
              { id: "count", header: "Denials", cell: ({ row }) => rawText(row.original.count) },
              { id: "last", header: "Last", cell: ({ row }) => nsToTime(Number(row.original.last_ts_ns || 0)) || rawText(row.original.last_ts_ns) },
              { id: "tools", header: "Tools", cell: ({ row }) => asArray<string>(row.original.tools).join(", ") || "unknown" }
            ]}
          />
          <SystemTable
            title="Sessions"
            icon={MonitorUp}
            rows={sessions}
            empty="No session rows"
            columns={[
              { id: "session", header: "Session", cell: ({ row }) => rawText(row.original.session_id) },
              { id: "lifecycle", header: "Lifecycle", cell: ({ row }) => rawText(row.original.lifecycle) },
              { id: "lease", header: "Lease", cell: ({ row }) => rawText(asRecord(row.original.lease).held) },
              { id: "last", header: "Last Action", cell: ({ row }) => rawText(row.original.last_action) }
            ]}
          />
        </div>
        {claims.length ? (
          <DataTable
            data={claims}
            getRowId={(row, index) => rawText(row.target_key || row.owner_session_id || index)}
            columns={[
              { id: "target", header: "Target", cell: ({ row }) => rawText(row.original.target_key || row.original.target) },
              { id: "owner", header: "Owner", cell: ({ row }) => rawText(row.original.owner_session_id) },
              { id: "expires", header: "Expires", cell: ({ row }) => rawText(row.original.expires_in_ms) },
              { id: "generation", header: "Gen", cell: ({ row }) => rawText(row.original.generation) }
            ]}
          />
        ) : (
          <EmptyState title="No target claims" />
        )}
        <div className="mt-3">
          <RawValue value={{ lease: state?.lease, target_claims: state?.target_claims, last_control: lastLeaseControl }} label="Lease and claims readback" />
        </div>
      </Section>

      <Section
        title="Targets"
        tier="drill-down"
        questions={["Which sessions own browser tabs?", "Which hidden desktops exist?", "Which session owns each target?"]}
      >
        <div className="grid gap-4 xl:grid-cols-2">
          <SystemTable
            title="Sessions"
            icon={MonitorUp}
            rows={sessions}
            empty="No session rows"
            columns={[
              { id: "session", header: "Session", cell: ({ row }) => rawText(row.original.session_id) },
              { id: "lifecycle", header: "Lifecycle", cell: ({ row }) => rawText(row.original.lifecycle) },
              { id: "target", header: "Target", cell: ({ row }) => rawText(row.original.active_target) },
              { id: "last", header: "Last Action", cell: ({ row }) => rawText(row.original.last_action) }
            ]}
          />
          <SystemTable
            title="CDP Attachments"
            icon={Network}
            rows={cdpAttachments}
            empty="No CDP owner rows"
            columns={[
              { id: "session", header: "Session", cell: ({ row }) => rawText(row.original.session_id) },
              { id: "target", header: "Target", cell: ({ row }) => rawText(row.original.cdp_target_id) },
              { id: "window", header: "HWND", cell: ({ row }) => rawText(row.original.window_hwnd) },
              { id: "url", header: "URL", cell: ({ row }) => <span className="line-clamp-2">{rawText(row.original.target_url)}</span> }
            ]}
          />
        </div>
        <div className="mt-4">
          <SystemTable
            title="Hidden Desktops"
            icon={MonitorUp}
            rows={hiddenDesktops}
            empty="No session-owned hidden desktop rows"
            columns={[
              { id: "session", header: "Session", cell: ({ row }) => rawText(row.original.session_id) },
              { id: "desktops", header: "Desktops", cell: ({ row }) => rawText(row.original.desktop_names) },
              { id: "pids", header: "PIDs", cell: ({ row }) => rawText(row.original.launch_pids) },
              { id: "count", header: "Resources", cell: ({ row }) => rawText(row.original.resource_count) }
            ]}
          />
        </div>
      </Section>

      <Section title="Shell Jobs" tier="drill-down" questions={["Which jobs are still running?", "Which status files were read?", "Can a job be inspected?"]}>
        <div className="mb-3 grid gap-4 md:grid-cols-3">
          <StatCard label="Total" value={rawText(shellJobsData.job_count || 0)} status="idle" delta={rawText(shellJobsData.job_root)} />
          <StatCard label="Running" value={rawText(shellJobsData.running_count || 0)} status={Number(shellJobsData.running_count || 0) ? "working" : "done"} />
          <StatCard label="Unreadable" value={rawText(shellJobsData.skipped_unreadable_status_files || 0)} status={Number(shellJobsData.skipped_unreadable_status_files || 0) ? "stuck" : "done"} />
        </div>
        {shellJobs.length ? (
          <DataTable
            data={shellJobs}
            getRowId={(row, index) => rawText(row.job_id || index)}
            columns={[
              { id: "job", header: "Job", cell: ({ row }) => rawText(row.original.job_id) },
              { id: "status", header: "Status", cell: ({ row }) => rawText(asRecord(row.original.job).status || row.original.status) },
              { id: "running", header: "Running", cell: ({ row }) => rawText(row.original.running) },
              { id: "pid", header: "PID", cell: ({ row }) => rawText(row.original.pid) },
              { id: "session", header: "Session", cell: ({ row }) => rawText(row.original.session_id) }
            ]}
          />
        ) : (
          <EmptyState title="No durable shell jobs" />
        )}
        <div className="mt-3">
          <RawValue value={state?.shell_jobs} label="Shell job readback" />
        </div>
      </Section>

      <Section title="Events" tier="drill-down" questions={["How many SSE subscriptions are open?", "Which sessions own them?", "Are ingress counters moving?"]}>
        <div className="grid gap-4 md:grid-cols-3">
          <StatCard label="SSE" value={rawText(events.active_subscription_count || 0)} status={Number(events.active_subscription_count || 0) ? "working" : "idle"} delta={state?.events.source || "SseState"} />
          <StatCard label="Owners" value={asArray(events.owner_session_ids).length} status="idle" />
          <StatCard label="Ingress" value={rawText(asRecord(events.agent_event_ingress).accepted || asRecord(events.agent_event_ingress).received || 0)} status="working" />
        </div>
        <div className="mt-3">
          <RawValue value={state?.events} label="Event readback" />
        </div>
      </Section>
    </>
  );
}

function SystemTable({
  title,
  icon: Icon,
  rows,
  empty,
  columns
}: {
  title: string;
  icon: LucideIcon;
  rows: Record<string, unknown>[];
  empty: string;
  columns: ColumnDef<Record<string, unknown>>[];
}) {
  return (
    <div className="min-w-0">
      <div className="mb-2 flex items-center gap-2 text-primary">
        <Icon aria-hidden="true" className="h-4 w-4 text-info" />
        <h3 className="text-md font-medium tracking-normal">{title}</h3>
      </div>
      {rows.length ? (
        <DataTable
          data={rows}
          getRowId={(row, index) => rawText(row.owner_key || row.session_id || row.cdp_target_id || row.job_id || index)}
          columns={columns}
        />
      ) : (
        <EmptyState title={empty} />
      )}
    </div>
  );
}

function AuditView({
  state,
  toolCalls,
  filters,
  onFiltersChange
}: {
  state?: DashboardState;
  toolCalls: ReturnType<typeof buildToolCalls>;
  filters: AuditQueryFilters;
  onFiltersChange: (filters: AuditQueryFilters) => void;
}) {
  const [draft, setDraft] = useState<AuditQueryFilters>(() => sanitizeAuditFilters(filters));
  const [selectedKey, setSelectedKey] = useState("");
  const auditQuery = useQuery({
    queryKey: ["dashboard-audit-query", filters],
    queryFn: () => fetchAuditQuery(filters),
    staleTime: 2000
  });

  useEffect(() => {
    setDraft(sanitizeAuditFilters(filters));
  }, [filters]);

  const query = auditQuery.data;
  const rows = query?.rows ?? [];
  const selectedRow = rows.find((row) => row.key_hex === selectedKey) ?? rows[0];

  useEffect(() => {
    if (!selectedKey && rows[0]) {
      setSelectedKey(rows[0].key_hex);
    }
    if (selectedKey && rows.length && !rows.some((row) => row.key_hex === selectedKey)) {
      setSelectedKey(rows[0].key_hex);
    }
  }, [rows, selectedKey]);

  const updateDraft = (field: keyof AuditQueryFilters, value: string) => {
    setDraft((current) => ({ ...current, [field]: value }));
  };

  const applyFilters = (event: FormEvent) => {
    event.preventDefault();
    onFiltersChange(sanitizeAuditFilters(draft));
  };

  const continueFromPartial = () => {
    const next = query?.next_start_key_hex;
    if (!next) return;
    onFiltersChange({ ...filters, cursor: next, start_ts_ns: "" });
  };

  const focusToolActivityRow = (keyHex: string) => {
    setSelectedKey(keyHex);
    onFiltersChange(auditCursorFilters(keyHex));
  };

  return (
    <>
      <ToolActivity toolCalls={toolCalls} onAuditKeySelect={focusToolActivityRow} />
      <Section
        title="Audit Filters"
        tier="triage"
        questions={["Which bounded CF_ACTION_LOG window is scanned?", "Which filters are active?", "Is continuation explicit?"]}
        actions={
          <Button variant="secondary" size="sm" onClick={() => auditQuery.refetch()} disabled={auditQuery.isFetching}>
            <RefreshCw aria-hidden="true" className="h-4 w-4" />
            Refresh
          </Button>
        }
      >
        <form onSubmit={applyFilters} className="space-y-4">
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <TextField label="Session" value={rawText(draft.session_id)} onChange={(value) => updateDraft("session_id", value)} mono placeholder="session id" />
            <TextField label="Tool" value={rawText(draft.tool)} onChange={(value) => updateDraft("tool", value)} mono placeholder="act_run_shell" />
            <TextField label="Status" value={rawText(draft.status)} onChange={(value) => updateDraft("status", value)} mono placeholder="ok / error / denied" />
            <TextField label="Error Code" value={rawText(draft.error_code)} onChange={(value) => updateDraft("error_code", value)} mono placeholder="TOOL_PARAMS_INVALID" />
            <TextField label="Start ns" value={rawText(draft.start_ts_ns)} onChange={(value) => updateDraft("start_ts_ns", value)} mono placeholder="inclusive timestamp" />
            <TextField label="End ns" value={rawText(draft.end_ts_ns)} onChange={(value) => updateDraft("end_ts_ns", value)} mono placeholder="inclusive timestamp" />
            <TextField label="Cursor" value={rawText(draft.cursor)} onChange={(value) => updateDraft("cursor", value)} mono placeholder="physical key hex" />
            <label className="mt-3 block text-sm text-secondary">
              <span className="mb-1 block text-label font-medium uppercase text-muted">Row Kind</span>
              <select
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                value={rawText(draft.row_kind || "all")}
                onChange={(event) => updateDraft("row_kind", event.target.value)}
              >
                <option value="all">All rows</option>
                <option value="command_audit">Command audit</option>
                <option value="action_audit">Action audit</option>
              </select>
            </label>
            <TextField label="Returned Limit" value={rawText(draft.limit)} onChange={(value) => updateDraft("limit", value)} type="number" />
            <TextField label="Scan Budget" value={rawText(draft.scan_limit)} onChange={(value) => updateDraft("scan_limit", value)} type="number" />
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button type="submit" variant="primary" size="sm">
              <Search aria-hidden="true" className="h-4 w-4" />
              Apply
            </Button>
            <Button type="button" variant="secondary" size="sm" onClick={() => onFiltersChange(defaultAuditFilters())}>
              <RefreshCw aria-hidden="true" className="h-4 w-4" />
              Reset
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => onFiltersChange({ limit: "100", scan_limit: "1000", row_kind: "all" })}
            >
              <FileSearch aria-hidden="true" className="h-4 w-4" />
              All Time
            </Button>
          </div>
        </form>
      </Section>

      <Section
        title="Audit Results"
        tier="drill-down"
        questions={["Which rows matched?", "Is truncation visible?", "Can I continue from the exact physical key?"]}
      >
        <div className="grid gap-4 md:grid-cols-4">
          <StatCard label="Scanned" value={rawText(query?.scanned_rows || 0)} status={query?.partial ? "needs_input" : "done"} delta={`budget ${rawText(query?.scan_limit || rawText(filters.scan_limit || ""))}`} />
          <StatCard label="Matched" value={rawText(query?.matched_rows || 0)} status={rows.length ? "working" : "idle"} delta={query?.cf_name || "CF_ACTION_LOG"} />
          <StatCard label="Returned" value={rawText(query?.returned_count || rows.length)} status={query?.partial ? "needs_input" : "done"} delta={`limit ${rawText(query?.limit || rawText(filters.limit || ""))}`} />
          <StatCard label="Corrupt" value={rawText(query?.corrupt_row_count || 0)} status={Number(query?.corrupt_row_count || 0) ? "stuck" : "done"} delta={query?.source_of_truth || "bounded scan"} />
        </div>
        {auditQuery.isError ? (
          <div className="mt-4 rounded-md border border-danger-border bg-danger-bg p-3 text-sm text-danger-fg">{rawText(auditQuery.error)}</div>
        ) : null}
        {query?.partial ? (
          <div className="mt-4 flex flex-wrap items-center justify-between gap-3 rounded-md border border-warning-border bg-warning-bg p-3 text-sm text-warning-fg">
            <span>
              Partial results: scanned {query.scanned_rows} rows, returned {query.returned_count}. Continue from {shortKey(query.next_start_key_hex)}.
            </span>
            <Button type="button" variant="secondary" size="sm" onClick={continueFromPartial} disabled={!query.next_start_key_hex}>
              <SkipForward aria-hidden="true" className="h-4 w-4" />
              Continue
            </Button>
          </div>
        ) : null}
        <div className="mt-4">
          <AuditResultsTable rows={rows} selectedKey={selectedRow?.key_hex} onSelect={setSelectedKey} loading={auditQuery.isFetching} />
        </div>
      </Section>

      <Section
        title="Row Detail"
        tier="drill-down"
        questions={["Which exact physical row is selected?", "What complete structured record is stored?", "Which Source of Truth backs it?"]}
      >
        {selectedRow ? (
          <div className="grid gap-4 xl:grid-cols-[minmax(0,0.36fr)_minmax(0,1fr)]">
            <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
              <MetricRow label="Key" value={<span className="font-mono">{shortKey(selectedRow.key_hex)}</span>} />
              <MetricRow label="Kind" value={selectedRow.row_kind} />
              <MetricRow label="Tool" value={selectedRow.tool} />
              <MetricRow label="Status" value={auditRowStatus(selectedRow)} />
              <MetricRow label="Session" value={selectedRow.actor_session_id || selectedRow.session_id || "none"} />
              <MetricRow label="Bytes" value={rawText(selectedRow.value_len_bytes)} />
              <RawValue value={selectedRow.source_of_truth} label="Physical storage key" />
            </div>
            <RawValue value={selectedRow.record} label="Full structured record" />
          </div>
        ) : (
          <EmptyState title={auditQuery.isFetching ? "Loading audit rows" : "No matching audit rows"} />
        )}
        <div className="mt-4">
          <RawValue value={{ query, snapshot: state?.command_audit }} label="Audit readback" />
        </div>
      </Section>
    </>
  );
}

function AuditResultsTable({
  rows,
  selectedKey,
  onSelect,
  loading
}: {
  rows: AuditQueryRow[];
  selectedKey?: string;
  onSelect: (keyHex: string) => void;
  loading: boolean;
}) {
  const [sorting, setSorting] = useState<SortingState>([{ id: "time", desc: true }]);
  const [scrollTop, setScrollTop] = useState(0);
  const columns = useMemo<ColumnDef<AuditQueryRow>[]>(
    () => [
      {
        id: "time",
        header: "Time",
        accessorFn: (row) => rawText(row.ts_ns_text || row.ts_ns),
        cell: ({ row }) => <span className="font-mono">{nsToTime(row.original.ts_ns_text || row.original.ts_ns)}</span>
      },
      {
        id: "kind",
        header: "Kind",
        accessorFn: (row) => row.row_kind,
        cell: ({ row }) => row.original.row_kind
      },
      {
        id: "tool",
        header: "Tool",
        accessorFn: (row) => row.tool,
        cell: ({ row }) => <span className="font-mono">{row.original.tool}</span>
      },
      {
        id: "status",
        header: "Status",
        accessorFn: auditRowStatus,
        cell: ({ row }) => auditRowStatus(row.original)
      },
      {
        id: "session",
        header: "Session",
        accessorFn: (row) => row.actor_session_id || row.session_id || "",
        cell: ({ row }) => <span className="font-mono">{shortKey(row.original.actor_session_id || row.original.session_id)}</span>
      },
      {
        id: "error",
        header: "Error",
        accessorFn: (row) => row.error_code || "",
        cell: ({ row }) => <span className="font-mono text-danger-fg">{row.original.error_code || ""}</span>
      },
      {
        id: "key",
        header: "Key",
        accessorFn: (row) => row.key_hex,
        cell: ({ row }) => <span className="font-mono">{shortKey(row.original.key_hex)}</span>
      },
      {
        id: "detail",
        header: "",
        enableSorting: false,
        cell: ({ row }) => (
          <Button type="button" variant="ghost" size="sm" onClick={() => onSelect(row.original.key_hex)} aria-label={`Select audit row ${row.original.key_hex}`}>
            <FileSearch aria-hidden="true" className="h-4 w-4" />
            Detail
          </Button>
        )
      }
    ],
    [onSelect]
  );
  const table = useReactTable({
    data: rows,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getRowId: (row) => row.key_hex
  });
  const tableRows = table.getRowModel().rows;
  const rowHeight = 48;
  const viewportHeight = 460;
  const overscan = 6;
  const startIndex = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const endIndex = Math.min(tableRows.length, Math.ceil((scrollTop + viewportHeight) / rowHeight) + overscan);
  const visibleRows = tableRows.slice(startIndex, endIndex);
  const topPad = startIndex * rowHeight;
  const bottomPad = Math.max(0, (tableRows.length - endIndex) * rowHeight);
  const columnCount = columns.length;

  if (!rows.length) {
    return <EmptyState title={loading ? "Loading audit rows" : "No audit rows match this bounded scan"} />;
  }

  return (
    <div className="rounded-lg border border-border">
      <div className="border-b border-border bg-surface-2 px-3 py-2 text-xs text-muted">
        Showing {visibleRows.length} of {tableRows.length} sorted rows in the visible window.
      </div>
      <div className="max-h-[460px] overflow-auto" onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}>
        <table className="w-full min-w-[960px] border-collapse text-sm">
          <thead className="sticky top-0 z-10 bg-surface-2">
            {table.getHeaderGroups().map((headerGroup) => (
              <tr key={headerGroup.id}>
                {headerGroup.headers.map((header) => {
                  const sorted = header.column.getIsSorted();
                  return (
                    <th key={header.id} className="border-b border-border px-3 py-2 text-left text-label font-medium uppercase text-muted">
                      {header.isPlaceholder ? null : header.column.getCanSort() ? (
                        <button type="button" className="inline-flex items-center gap-1" onClick={header.column.getToggleSortingHandler()}>
                          {flexRender(header.column.columnDef.header, header.getContext())}
                          {sorted ? <ChevronDown aria-hidden="true" className={cn("h-3 w-3", sorted === "asc" && "rotate-180")} /> : <ArrowUpDown aria-hidden="true" className="h-3 w-3" />}
                        </button>
                      ) : (
                        flexRender(header.column.columnDef.header, header.getContext())
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {topPad ? (
              <tr aria-hidden="true">
                <td colSpan={columnCount} style={{ height: topPad }} />
              </tr>
            ) : null}
            {visibleRows.map((row) => (
              <tr
                key={row.id}
                aria-selected={row.original.key_hex === selectedKey}
                className={cn(
                  "h-12 border-b border-border-subtle last:border-b-0 hover:bg-surface-2",
                  row.original.key_hex === selectedKey && "bg-surface-2"
                )}
              >
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id} className="max-w-72 px-3 py-2 align-middle text-secondary">
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
            {bottomPad ? (
              <tr aria-hidden="true">
                <td colSpan={columnCount} style={{ height: bottomPad }} />
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function auditRowStatus(row: AuditQueryRow) {
  return rawText(row.status || row.outcome || row.phase || "");
}

function OverviewBand({
  state,
  agents,
  attentionCount,
  stale
}: {
  state?: DashboardState;
  agents: AgentSummary[];
  attentionCount: number;
  stale: boolean;
}) {
  const health = asRecord(panelData(state?.daemon));
  const storage = asRecord(panelData(state?.storage));
  const storagePressure = rawText(asRecord(storage.pressure_level).name || asRecord(storage.pressure_level).value || "unknown");
  const liveAgents = agents.filter((agent) => agent.lifecycle === "live").length;
  const toolCount = Number(health.tool_count || 0);
  return (
    <Section title="Overview" tier="overview" questions={["Is anything wrong?", "How many agents are live?", "Is the daemon stale?"]}>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <StatCard label="Attention" value={attentionCount} status={attentionCount ? "needs_input" : "done"} delta={attentionCount ? "human review queued" : "quiet"} />
        <StatCard label="Live Agents" value={liveAgents} status={liveAgents ? "working" : "idle"} delta={`${agents.length} total rows`} />
        <StatCard label="Tools" value={toolCount} status={toolCount ? "done" : "stuck"} delta="strict client surface" />
        <StatCard label="Freshness" value={stale ? "stale" : "live"} status={stale ? "stuck" : "working"} delta={storagePressure} />
      </div>
    </Section>
  );
}

const deepSeekPresets = {
  flash: {
    label: "DeepSeek Flash",
    name: "deepseek-flash",
    base_url: "",
    model_id: "deepseek-v4-flash",
    runtime_preset: "deepseek_v4_flash_non_thinking",
    api_key_env_var: "DEEPSEEK_API_KEY",
    api_key: "",
    context_length: "1000000",
    max_tools: "128",
    notes: "DeepSeek V4 Flash non-thinking API agent"
  },
  reasoning: {
    label: "DeepSeek Reasoning",
    name: "deepseek-reasoning",
    base_url: "",
    model_id: "deepseek-v4-flash",
    runtime_preset: "deepseek_v4_reasoning",
    api_key_env_var: "DEEPSEEK_API_KEY",
    api_key: "",
    context_length: "1000000",
    max_tools: "128",
    notes: "DeepSeek V4 Flash reasoning API agent"
  }
};

type SpawnMode = "template" | "local_model" | "codex" | "claude";
type SpawnTargetMode = "none" | "window" | "cdp";

function SpawnConsole({ onSpawned }: { onSpawned: () => void }) {
  const modelsQuery = useQuery({
    queryKey: ["dashboard-models"],
    queryFn: fetchModels
  });
  const templatesQuery = useQuery({
    queryKey: ["dashboard-templates"],
    queryFn: fetchTemplates
  });
  const models = modelsQuery.data ?? [];
  const templates = templatesQuery.data ?? [];
  const [spawnMode, setSpawnMode] = useState<SpawnMode>("template");
  const [selectedModel, setSelectedModel] = useState("");
  const [selectedTemplateId, setSelectedTemplateId] = useState("");
  const [templateVersion, setTemplateVersion] = useState("");
  const [templateParams, setTemplateParams] = useState<Record<string, string>>({});
  const [fanOut, setFanOut] = useState("1");
  const [waitTimeoutMs, setWaitTimeoutMs] = useState("300000");
  const [holdOpenMs, setHoldOpenMs] = useState("300000");
  const [prompt, setPrompt] = useState('Use workspace_put with key issue985-deepseek-smoke and value {"ok":true}.');
  const [workingDir, setWorkingDir] = useState("C:\\code\\Synapse");
  const [directModel, setDirectModel] = useState("");
  const [targetMode, setTargetMode] = useState<SpawnTargetMode>("none");
  const [targetWindowHwnd, setTargetWindowHwnd] = useState("");
  const [targetCdpId, setTargetCdpId] = useState("");
  const [registerForm, setRegisterForm] = useState(deepSeekPresets.flash);
  const [pendingAction, setPendingAction] = useState<"register" | "spawn" | "">("");
  const [error, setError] = useState("");
  const [lastRegister, setLastRegister] = useState<DashboardRouteReadback | null>(null);
  const [lastSpawn, setLastSpawn] = useState<SpawnAgentResponse | null>(null);

  useEffect(() => {
    if (!selectedModel && models.length > 0) {
      const firstLaunchable = models.find((model) => model.enabled && model.last_probe?.healthy) ?? models[0];
      setSelectedModel(firstLaunchable.name);
    }
  }, [models, selectedModel]);

  useEffect(() => {
    if (spawnMode === "template" && !selectedTemplateId && templates.length > 0) {
      setSelectedTemplateId(templates[0].template_id);
    }
  }, [spawnMode, selectedTemplateId, templates]);

  const selected = models.find((model) => model.name === selectedModel);
  const selectedTemplate = templates.find((template) => template.template_id === selectedTemplateId);
  const requiredParamSignature = selectedTemplate?.required_params.join("\n") ?? "";

  useEffect(() => {
    if (!selectedTemplate) return;
    setTemplateVersion(String(selectedTemplate.version));
    setTemplateParams((existing) => {
      const next: Record<string, string> = {};
      for (const key of selectedTemplate.required_params) {
        next[key] = existing[key] ?? "";
      }
      return next;
    });
  }, [selectedTemplate?.template_id, selectedTemplate?.version, requiredParamSignature]);

  const fanOutNumber = Number(fanOut.trim() || "1");
  const fanOutValid = Number.isInteger(fanOutNumber) && fanOutNumber >= 1 && fanOutNumber <= 5;
  const templateParamsReady = (selectedTemplate?.required_params ?? []).every((param) => Boolean(templateParams[param]?.trim()));
  const directReady =
    spawnMode === "local_model"
      ? Boolean(selectedModel && prompt.trim())
      : spawnMode === "codex" || spawnMode === "claude"
        ? Boolean(prompt.trim())
        : false;
  const canSpawn = Boolean(
    !pendingAction &&
      fanOutValid &&
      ((spawnMode === "template" && selectedTemplateId && templateParamsReady) ||
        (spawnMode !== "template" && directReady))
  );

  const buildTarget = () => {
    if (targetMode === "none") return undefined;
    const parsedWindow = Number(targetWindowHwnd.trim());
    if (!Number.isInteger(parsedWindow) || parsedWindow <= 0) {
      throw new Error("target window HWND must be a positive integer");
    }
    if (targetMode === "window") {
      return { kind: "window" as const, window_hwnd: parsedWindow };
    }
    const cdpTargetId = targetCdpId.trim();
    if (!cdpTargetId) {
      throw new Error("target CDP id is required");
    }
    return { kind: "cdp" as const, window_hwnd: parsedWindow, cdp_target_id: cdpTargetId };
  };

  const submitRegister = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError("");
    setPendingAction("register");
    try {
      const readback = await registerApiModel({
        name: registerForm.name,
        base_url: registerForm.base_url,
        model_id: registerForm.model_id,
        runtime_preset: registerForm.runtime_preset,
        api_key_env_var: registerForm.api_key_env_var,
        api_key: registerForm.api_key.trim() ? registerForm.api_key : undefined,
        context_length: parsePositiveInteger(registerForm.context_length, "context_length"),
        max_tools: parsePositiveInteger(registerForm.max_tools, "max_tools"),
        notes: registerForm.notes,
        probe_timeout_ms: 30000
      });
      setLastRegister(readback);
      const row = asRecord(asRecord(readback.register).row) as unknown as ModelRow;
      if (row.name) setSelectedModel(row.name);
      await modelsQuery.refetch();
    } catch (registerError) {
      setError(rawText(registerError) || "API model registration failed");
    } finally {
      setPendingAction("");
    }
  };

  const submitSpawn = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError("");
    setPendingAction("spawn");
    try {
      const fanOutValue = parsePositiveInteger(fanOut, "fan_out") ?? 1;
      if (fanOutValue > 5) {
        throw new Error("fan_out must be 1..=5");
      }
      const request = {
        fan_out: fanOutValue,
        wait_timeout_ms: parsePositiveInteger(waitTimeoutMs, "wait_timeout_ms") ?? 300000,
        hold_open_ms: parseNonNegativeInteger(holdOpenMs, "hold_open_ms") ?? 0
      };
      if (spawnMode === "template") {
        Object.assign(request, {
          template_id: selectedTemplateId,
          template_version: parsePositiveInteger(templateVersion, "template_version"),
          template_params: Object.fromEntries(
            (selectedTemplate?.required_params ?? []).map((param) => [param, templateParams[param]?.trim() ?? ""])
          )
        });
      } else {
        Object.assign(request, {
          kind: spawnMode,
          model: spawnMode === "local_model" ? undefined : directModel.trim() || undefined,
          model_ref: spawnMode === "local_model" ? selectedModel : undefined,
          prompt,
          target: buildTarget(),
          working_dir: workingDir.trim() || undefined
        });
      }
      const readback = await spawnAgent(request);
      setLastSpawn(readback);
      await modelsQuery.refetch();
      await templatesQuery.refetch();
      onSpawned();
      if (readback.failed_count > 0) {
        setError(`${readback.failed_count} spawn attempt failed; inspect readback rows.`);
      }
    } catch (spawnError) {
      setError(rawText(spawnError) || "Spawn failed");
    } finally {
      setPendingAction("");
    }
  };

  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(26rem,1fr)]">
      <div className="min-w-0 space-y-4">
        <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <h3 className="text-md font-semibold tracking-normal text-primary">Models</h3>
              <p className="mt-1 text-sm text-secondary">{modelsQuery.isFetching ? "Refreshing registry" : `${models.length} registry rows`}</p>
            </div>
            <Button size="sm" variant="ghost" onClick={() => modelsQuery.refetch()} disabled={modelsQuery.isFetching}>
              <RefreshCw aria-hidden="true" className="h-4 w-4" />
              Refresh
            </Button>
          </div>
          {models.length ? (
            <DataTable
              data={models}
              getRowId={(model) => model.name}
              columns={[
                {
                  id: "status",
                  header: "Status",
                  cell: ({ row }) => <StatusBadge status={modelFleetStatus(row.original)} />
                },
                {
                  accessorKey: "name",
                  header: "Name",
                  cell: ({ row }) => <span className="font-mono text-primary">{row.original.name}</span>
                },
                { accessorKey: "model_id", header: "Model" },
                {
                  id: "runtime",
                  header: "Runtime",
                  cell: ({ row }) => <span className="font-mono">{row.original.runtime_preset || "open_ai_compatible"}</span>
                },
                {
                  id: "env",
                  header: "Key env",
                  cell: ({ row }) => <span className="font-mono">{row.original.api_key_env_var || "none"}</span>
                },
                {
                  id: "key",
                  header: "API key",
                  cell: ({ row }) =>
                    row.original.has_api_key_secret ? (
                      <span className="font-mono text-success" title="Encrypted API key stored at rest (DPAPI)">🔑 stored</span>
                    ) : row.original.api_key_env_var ? (
                      <span className="font-mono text-warning" title="No stored key; resolves from daemon environment">env only</span>
                    ) : (
                      <span className="font-mono text-muted">none</span>
                    )
                }
              ]}
            />
          ) : (
            <EmptyStateArt title={modelsQuery.isError ? rawText(modelsQuery.error) || "Model registry unavailable" : "No model rows"} />
          )}
        </div>

        <form className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]" onSubmit={submitSpawn}>
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <Rocket aria-hidden="true" className="h-4 w-4 text-info" />
              <h3 className="text-md font-semibold tracking-normal text-primary">Spawn</h3>
            </div>
            <div className="inline-flex rounded-md border border-border bg-surface-2 p-1">
              {(["template", "local_model", "codex", "claude"] as SpawnMode[]).map((mode) => (
                <button
                  key={mode}
                  type="button"
                  className={cn("h-8 rounded px-3 text-sm text-secondary", spawnMode === mode && "bg-accent text-accent-fg")}
                  onClick={() => setSpawnMode(mode)}
                >
                  {mode === "local_model" ? "Local" : mode[0].toUpperCase() + mode.slice(1)}
                </button>
              ))}
            </div>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="block text-sm text-secondary">
              <span className="mb-1 block text-label font-medium uppercase text-muted">Fan-out</span>
              <input
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 font-mono text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                min={1}
                max={5}
                type="number"
                value={fanOut}
                onChange={(event) => setFanOut(event.target.value)}
              />
            </label>
            <TextField label="Wait timeout ms" value={waitTimeoutMs} onChange={setWaitTimeoutMs} mono type="number" />
            <TextField label="Hold open ms" value={holdOpenMs} onChange={setHoldOpenMs} mono type="number" />
            <label className="block text-sm text-secondary">
              <span className="mb-1 block text-label font-medium uppercase text-muted">Working dir</span>
              <input
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 font-mono text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring disabled:opacity-60"
                value={workingDir}
                onChange={(event) => setWorkingDir(event.target.value)}
                disabled={spawnMode === "template"}
              />
            </label>
          </div>
          {spawnMode === "template" ? (
            <div className="mt-3 grid gap-3 md:grid-cols-2">
              <label className="block text-sm text-secondary">
                <span className="mb-1 block text-label font-medium uppercase text-muted">Template</span>
                <select
                  className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                  value={selectedTemplateId}
                  onChange={(event) => setSelectedTemplateId(event.target.value)}
                >
                  {templates.map((template) => (
                    <option key={`${template.template_id}:${template.version}`} value={template.template_id}>
                      {template.name || template.template_id}
                    </option>
                  ))}
                </select>
              </label>
              <TextField label="Version" value={templateVersion} onChange={setTemplateVersion} mono />
              {(selectedTemplate?.required_params ?? []).map((param) => (
                <TextField
                  key={param}
                  label={param}
                  value={templateParams[param] ?? ""}
                  onChange={(value) => setTemplateParams((params) => ({ ...params, [param]: value }))}
                  mono
                />
              ))}
              {selectedTemplate ? (
                <div className="md:col-span-2 rounded-md border border-border bg-surface-2 p-3 text-sm text-secondary">
                  <div className="grid gap-2 md:grid-cols-3">
                    <span className="font-mono text-primary">{selectedTemplate.template_id}</span>
                    <span>{selectedTemplate.agent_kind}</span>
                    <span className="font-mono">{selectedTemplate.model_ref || selectedTemplate.model || "default model"}</span>
                  </div>
                  <RawValue value={selectedTemplate} label="Template row" />
                </div>
              ) : (
                <div className="md:col-span-2">
                  <EmptyStateArt title={templatesQuery.isError ? rawText(templatesQuery.error) || "Templates unavailable" : "No template rows"} />
                </div>
              )}
            </div>
          ) : (
            <>
              <div className="mt-3 grid gap-3 md:grid-cols-2">
                {spawnMode === "local_model" ? (
                  <label className="block text-sm text-secondary">
                    <span className="mb-1 block text-label font-medium uppercase text-muted">Model</span>
                    <select
                      className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                      value={selectedModel}
                      onChange={(event) => setSelectedModel(event.target.value)}
                    >
                      {models.map((model) => (
                        <option key={model.name} value={model.name}>
                          {model.name}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : (
                  <TextField label="Model" value={directModel} onChange={setDirectModel} mono />
                )}
                <label className="block text-sm text-secondary">
                  <span className="mb-1 block text-label font-medium uppercase text-muted">Target</span>
                  <select
                    className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                    value={targetMode}
                    onChange={(event) => setTargetMode(event.target.value as SpawnTargetMode)}
                  >
                    <option value="none">None</option>
                    <option value="window">Window</option>
                    <option value="cdp">CDP tab</option>
                  </select>
                </label>
                {targetMode !== "none" ? <TextField label="Window HWND" value={targetWindowHwnd} onChange={setTargetWindowHwnd} mono /> : null}
                {targetMode === "cdp" ? <TextField label="CDP target" value={targetCdpId} onChange={setTargetCdpId} mono /> : null}
              </div>
              <label className="mt-3 block text-sm text-secondary">
                <span className="mb-1 block text-label font-medium uppercase text-muted">Prompt</span>
                <textarea
                  className="min-h-28 w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                  value={prompt}
                  onChange={(event) => setPrompt(event.target.value)}
                />
              </label>
            </>
          )}
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0 text-sm text-secondary">
              {spawnMode === "template"
                ? selectedTemplate
                  ? `${selectedTemplate.agent_kind} template v${selectedTemplate.version}`
                  : `${templates.length} templates`
                : selected
                  ? `${selected.model_id} / ${selected.last_probe?.status || "unprobed"}`
                  : "No model selected"}
            </div>
            <Button type="submit" variant="primary" disabled={!canSpawn}>
              <Rocket aria-hidden="true" className="h-4 w-4" />
              {pendingAction === "spawn" ? "Spawning" : "Spawn"}
            </Button>
          </div>
        </form>

        <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <h3 className="text-md font-semibold tracking-normal text-primary">Templates</h3>
              <p className="mt-1 text-sm text-secondary">{templatesQuery.isFetching ? "Refreshing templates" : `${templates.length} current rows`}</p>
            </div>
            <Button size="sm" variant="ghost" onClick={() => templatesQuery.refetch()} disabled={templatesQuery.isFetching}>
              <RefreshCw aria-hidden="true" className="h-4 w-4" />
              Refresh
            </Button>
          </div>
          {templates.length ? (
            <DataTable
              data={templates}
              getRowId={(template) => `${template.template_id}:${template.version}`}
              columns={[
                {
                  accessorKey: "template_id",
                  header: "Template",
                  cell: ({ row }) => <span className="font-mono text-primary">{row.original.template_id}</span>
                },
                { accessorKey: "name", header: "Name" },
                { accessorKey: "agent_kind", header: "Kind" },
                {
                  id: "version",
                  header: "Version",
                  cell: ({ row }) => <span className="font-mono">{row.original.version}</span>
                },
                {
                  id: "params",
                  header: "Params",
                  cell: ({ row }) => <span className="font-mono">{row.original.required_params.join(", ") || "none"}</span>
                }
              ]}
            />
          ) : (
            <EmptyStateArt title={templatesQuery.isError ? rawText(templatesQuery.error) || "Templates unavailable" : "No template rows"} />
          )}
        </div>
      </div>

      <div className="min-w-0 space-y-4">
        <form className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]" onSubmit={submitRegister}>
          <div className="mb-3 flex items-center gap-2">
            <Plus aria-hidden="true" className="h-4 w-4 text-info" />
            <h3 className="text-md font-semibold tracking-normal text-primary">Add API Model</h3>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="mt-3 block text-sm text-secondary">
              <span className="mb-1 block text-label font-medium uppercase text-muted">Preset</span>
              <select
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                value={registerForm.runtime_preset}
                onChange={(event) => {
                  const preset = Object.values(deepSeekPresets).find((item) => item.runtime_preset === event.target.value);
                  if (preset) setRegisterForm({ ...preset });
                }}
              >
                {Object.values(deepSeekPresets).map((preset) => (
                  <option key={preset.runtime_preset} value={preset.runtime_preset}>
                    {preset.label}
                  </option>
                ))}
              </select>
            </label>
            <TextField label="Name" value={registerForm.name} onChange={(value) => setRegisterForm((form) => ({ ...form, name: value }))} />
            <TextField label="Model" value={registerForm.model_id} onChange={(value) => setRegisterForm((form) => ({ ...form, model_id: value }))} />
            <TextField label="Base URL" value={registerForm.base_url} onChange={(value) => setRegisterForm((form) => ({ ...form, base_url: value }))} mono />
            <TextField label="Key env" value={registerForm.api_key_env_var} onChange={(value) => setRegisterForm((form) => ({ ...form, api_key_env_var: value }))} mono />
            <TextField
              label="API key (stored encrypted)"
              value={registerForm.api_key}
              onChange={(value) => setRegisterForm((form) => ({ ...form, api_key: value }))}
              type="password"
              autoComplete="off"
              placeholder="sk-… (encrypted at rest via Windows DPAPI)"
              mono
            />
            <TextField label="Context" value={registerForm.context_length} onChange={(value) => setRegisterForm((form) => ({ ...form, context_length: value }))} mono />
            <TextField label="Max tools" value={registerForm.max_tools} onChange={(value) => setRegisterForm((form) => ({ ...form, max_tools: value }))} mono />
          </div>
          <TextField label="Notes" value={registerForm.notes} onChange={(value) => setRegisterForm((form) => ({ ...form, notes: value }))} />
          <div className="mt-3 flex justify-end">
            <Button type="submit" variant="secondary" disabled={pendingAction === "register"}>
              <Plus aria-hidden="true" className="h-4 w-4" />
              {pendingAction === "register" ? "Registering" : "Register"}
            </Button>
          </div>
        </form>
        {error ? <div className="rounded-lg border border-danger-border bg-danger-bg p-3 text-sm text-danger-fg">{error}</div> : null}
        {lastRegister ? <RawValue value={lastRegister} label="Register readback" /> : null}
        <SpawnReadbackStrip readback={lastSpawn} />
      </div>
    </div>
  );
}

function SpawnReadbackStrip({ readback }: { readback: SpawnAgentResponse | null }) {
  if (!readback) return null;
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-md font-semibold tracking-normal text-primary">Spawn Readbacks</h3>
          <p className="mt-1 text-sm text-secondary">
            {readback.succeeded_count} ok / {readback.failed_count} error / {readback.requested_count} requested
          </p>
        </div>
        <span className="font-mono text-xs text-muted">{readback.source_of_truth}</span>
      </div>
      <div className="space-y-3">
        {readback.attempts.map((attempt) => {
          const spawn = asRecord(attempt.spawn);
          const logPaths = asRecord(spawn.log_paths);
          const rows = ([
            ["Spawn", spawn.spawn_id],
            ["Session", spawn.session_id],
            ["Launcher PID", spawn.launcher_process_id],
            ["Agent PID", spawn.agent_process_id],
            ["Kind", spawn.kind],
            ["Template", spawn.template_id],
            ["Template version", spawn.template_version],
            ["Launch target", spawn.launch_target],
            ["Log dir", logPaths.log_dir],
            ["Prompt", logPaths.prompt_path],
            ["Task started", logPaths.task_started_path],
            ["Completion", logPaths.completion_status_path]
          ] as Array<[string, unknown]>).filter((row) => rawText(row[1]));
          return (
            <article key={attempt.index} className="rounded-md border border-border bg-surface-2 p-3">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                  <StatusBadge status={attempt.status === "ok" ? "done" : "stuck"} />
                  <span className="font-mono text-sm text-primary">attempt {attempt.index}</span>
                </div>
                {attempt.error_code ? <span className="font-mono text-xs text-danger-fg">{attempt.error_code}</span> : null}
              </div>
              {attempt.status === "ok" ? (
                <div className="grid gap-2 text-sm md:grid-cols-2">
                  {rows.map(([label, value]) => (
                    <div key={String(label)} className="min-w-0">
                      <div className="text-label font-medium uppercase text-muted">{label}</div>
                      <div className="truncate font-mono text-primary">{rawText(value)}</div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="space-y-2 text-sm text-danger-fg">
                  <div>{attempt.message || "spawn failed"}</div>
                  {attempt.data ? <RawValue value={attempt.data} label="Error data" /> : null}
                </div>
              )}
              <RawValue value={attempt} label="Attempt raw" />
            </article>
          );
        })}
      </div>
      <RawValue value={readback} label="Response raw" />
    </div>
  );
}

function TextField({
  label,
  value,
  onChange,
  mono = false,
  type = "text",
  placeholder,
  autoComplete
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  mono?: boolean;
  type?: "text" | "password" | "number";
  placeholder?: string;
  autoComplete?: string;
}) {
  return (
    <label className="mt-3 block text-sm text-secondary">
      <span className="mb-1 block text-label font-medium uppercase text-muted">{label}</span>
      <input
        className={`h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring ${mono ? "font-mono" : ""}`}
        type={type}
        placeholder={placeholder}
        autoComplete={autoComplete}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function parsePositiveInteger(value: string, field: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${field} must be a positive integer`);
  }
  return parsed;
}

function parseNonNegativeInteger(value: string, field: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error(`${field} must be a non-negative integer`);
  }
  return parsed;
}

function modelFleetStatus(model: ModelRow): FleetStatus {
  if (!model.enabled) return "idle";
  if (model.last_probe?.healthy) return "done";
  if (model.last_probe) return "stuck";
  return "needs_input";
}

function FleetList({
  agents,
  selectedId,
  onSelect,
  onKill,
  killingId
}: {
  agents: AgentSummary[];
  selectedId?: string;
  onSelect: (id: string) => void;
  onKill: (agent: AgentSummary) => void;
  killingId: string;
}) {
  if (agents.length === 0) return <EmptyStateArt title="No agent rows" />;
  return (
    <div className="rounded-lg border border-border bg-surface-1">
      {agents.map((agent) => {
        const pending = killingId === agent.killId;
        return (
          <div key={agent.id} className="grid grid-cols-[minmax(0,1fr)_auto] items-stretch border-b border-border-subtle last:border-b-0">
            <FleetRow agent={agent} selected={agent.id === selectedId} onSelect={() => onSelect(agent.id)} />
            <div className="flex items-center px-3">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={!agent.killable || pending}
                onClick={() => onKill(agent)}
                aria-label={`Kill ${agent.killId || agent.id} from fleet list`}
              >
                <X aria-hidden="true" className="h-4 w-4" />
                {pending ? "Killing" : "Kill"}
              </Button>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function FleetTable({
  agents,
  onSelect,
  onKill,
  killingId
}: {
  agents: AgentSummary[];
  onSelect: (id: string) => void;
  onKill: (agent: AgentSummary) => void;
  killingId: string;
}) {
  if (agents.length === 0) return <EmptyStateArt title="No fleet rows" />;
  return (
    <DataTable
      data={agents}
      getRowId={(agent) => agent.id}
      columns={[
        {
          id: "status",
          header: "Status",
          cell: ({ row }) => <StatusBadge status={row.original.status} />
        },
        {
          accessorKey: "id",
          header: "Agent",
          cell: ({ row }) => (
            <button className="truncate text-left text-primary underline-offset-4 hover:underline" type="button" onClick={() => onSelect(row.original.id)}>
              {row.original.id}
            </button>
          )
        },
        { accessorKey: "kind", header: "Kind" },
        { accessorKey: "lifecycle", header: "Lifecycle" },
        {
          id: "summary",
          header: "Summary",
          cell: ({ row }) => <span className="line-clamp-2">{row.original.summary}</span>
        },
        {
          id: "diff",
          header: "Diff",
          cell: ({ row }) => `${row.original.diffStats.actions}/${row.original.diffStats.transcripts}`
        },
        {
          id: "actions",
          header: "Actions",
          cell: ({ row }) => {
            const agent = row.original;
            const pending = killingId === agent.killId;
            return (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={!agent.killable || pending}
                    onClick={() => onKill(agent)}
                    aria-label={`Kill ${agent.killId || agent.id}`}
                  >
                    <X aria-hidden="true" className="h-4 w-4" />
                    {pending ? "Killing" : "Kill"}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{agent.killable ? `Kill ${agent.killId}` : "Historical row"}</TooltipContent>
              </Tooltip>
            );
          }
        }
      ]}
    />
  );
}

function ToolActivity({
  toolCalls,
  onAuditKeySelect
}: {
  toolCalls: ReturnType<typeof buildToolCalls>;
  onAuditKeySelect?: (keyHex: string) => void;
}) {
  return (
    <Section title="Tool Activity" tier="triage" questions={["Which tools are still running?", "Which calls failed?", "Where is the verification detail?"]}>
      {toolCalls.length ? (
        <div className="grid gap-3 lg:grid-cols-2">
          {toolCalls.slice(0, 6).map((call) => {
            const keyHex = rawText(asRecord(call.raw).key_hex);
            return (
              <div key={call.id} className="space-y-2">
                <ToolCallCard call={call} />
                {onAuditKeySelect && keyHex ? (
                  <Button type="button" variant="ghost" size="sm" onClick={() => onAuditKeySelect(keyHex)}>
                    <FileSearch aria-hidden="true" className="h-4 w-4" />
                    Audit
                  </Button>
                ) : null}
              </div>
            );
          })}
        </div>
      ) : (
        <EmptyStateArt title="No command audit rows" />
      )}
    </Section>
  );
}

function SystemShape({ state }: { state?: DashboardState }) {
  const storage = asRecord(panelData(state?.storage));
  const counts = asRecord(storage.cf_row_counts);
  const chartData = Object.entries(counts)
    .map(([name, value]) => ({ name: name.replace("CF_", ""), rows: Number(value) || 0 }))
    .sort((a, b) => b.rows - a.rows)
    .slice(0, 8);
  if (!chartData.length) return <EmptyStateArt title="No storage rows" />;
  return (
    <div className="space-y-4">
      <div className="h-64 rounded-lg border border-border bg-surface-1 p-3">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={chartData} margin={{ top: 8, right: 8, bottom: 8, left: 8 }}>
            <CartesianGrid stroke="var(--border-subtle)" vertical={false} />
            <XAxis dataKey="name" stroke="var(--text-muted)" tickLine={false} axisLine={false} />
            <YAxis stroke="var(--text-muted)" tickLine={false} axisLine={false} />
            <ChartTooltip contentStyle={{ background: "var(--surface-3)", border: "1px solid var(--border)", color: "var(--text-primary)" }} />
            <Bar dataKey="rows" fill="var(--info)" radius={[4, 4, 0, 0]} />
          </BarChart>
        </ResponsiveContainer>
      </div>
      <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
        <MetricRow label="Schema" value={rawText(storage.schema_version)} />
        <MetricRow label="Policy count" value={rawText(storage.audit_retention_policy_count)} />
        <MetricRow label="Generated" value={unixMsToTime(state?.generated_at_unix_ms)} />
      </div>
    </div>
  );
}

function TranscriptSamples({ state }: { state?: DashboardState }) {
  const rows = asArray<Record<string, unknown>>(asRecord(panelData(state?.agent_transcripts)).rows).slice(0, 4);
  return (
    <Section title="Transcript Samples" tier="drill-down" questions={["What did recent agents say?", "Was output sanitized before render?", "Where is the source row?"]}>
      {rows.length ? (
        <div className="grid gap-3 lg:grid-cols-2">
          {rows.map((row, index) => (
            <TranscriptTurn key={`${rawText(row.spawn_id)}-${rawText(row.line_no)}-${index}`} row={row} />
          ))}
        </div>
      ) : (
        <EmptyStateArt title="No transcript rows" />
      )}
    </Section>
  );
}

function EmptyStateArt({ title }: { title: string }) {
  return (
    <div className="overflow-hidden rounded-lg border border-border bg-surface-1">
      <img src={emptyStateUrl} alt="" className="h-32 w-full object-cover opacity-80" />
      <div className="border-t border-border-subtle p-3">
        <EmptyState title={title} />
      </div>
    </div>
  );
}

function routeFromHash(hash: string): DashboardRouteId | null {
  const raw = hash.replace(/^#\/?/, "").split(/[/?]/)[0];
  return isDashboardRouteId(raw) ? raw : null;
}

function isEditableShortcutTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tagName = target.tagName.toLowerCase();
  return tagName === "input" || tagName === "textarea" || tagName === "select";
}

function isDashboardRouteId(value: unknown): value is DashboardRouteId {
  return typeof value === "string" && routeDefinitions.some((item) => item.id === value);
}

function isDensity(value: unknown): value is Density {
  return value === "comfortable" || value === "compact";
}

function isTheme(value: unknown): value is Theme {
  return value === "dark" || value === "light";
}
