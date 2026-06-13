import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";
import {
  Bell,
  CheckCircle2,
  Gauge,
  LayoutDashboard,
  LogIn,
  LogOut,
  Moon,
  RefreshCw,
  Rows3,
  Sun,
  TerminalSquare
} from "lucide-react";
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
  Section,
  StatCard,
  ToolCallCard,
  TranscriptTurn
} from "@/primitives";
import {
  buildAgents,
  buildToolCalls,
  fetchDashboardAuthStatus,
  fetchDashboardState,
  loginDashboard,
  logoutDashboard,
  panelData,
  type AgentSummary,
  type DashboardAuthStatus,
  type DashboardState,
  type FleetStatus
} from "@/lib/dashboard-state";
import { asArray, asRecord, nsToTime, rawText, timeAgo, unixMsToTime } from "@/lib/utils";
import { useUiStore } from "@/store/ui-store";

export function App() {
  const density = useUiStore((state) => state.density);
  const setDensity = useUiStore((state) => state.setDensity);
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);
  const selectedAgentId = useUiStore((state) => state.selectedAgentId);
  const setSelectedAgentId = useUiStore((state) => state.setSelectedAgentId);
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

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.density = density;
  }, [theme, density]);

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

  const advanceAttention = () => {
    if (attentionAgents.length === 0) return;
    const current = attentionAgents.findIndex((agent) => agent.id === selectedAgent?.id);
    const next = attentionAgents[(current + 1 + attentionAgents.length) % attentionAgents.length];
    setSelectedAgentId(next.id);
  };

  const state = query.data;
  const freshnessMs = state ? Date.now() - state.generated_at_unix_ms : undefined;
  const stale = query.isError || (freshnessMs !== undefined && freshnessMs > 10000);

  if (authQuery.data?.authenticated !== true) {
    return (
      <AppShell sidebar={<Sidebar state={state} auth={authQuery.data} />}>
        <LoginView
          auth={authQuery.data}
          pending={authQuery.isLoading}
          onAuthenticated={() => {
            authQuery.refetch();
            query.refetch();
          }}
        />
      </AppShell>
    );
  }

  return (
    <AppShell sidebar={<Sidebar state={state} auth={authQuery.data} />}>
      <PageHeader
        title="Fleet Overview"
        subtitle={
          <span className={stale ? "text-warning-fg" : "text-secondary"}>
            {query.isError ? rawText(query.error) : `Updated ${freshnessMs === undefined ? "pending" : timeAgo(freshnessMs)} ago`}
          </span>
        }
        actions={
          <>
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

      <OverviewBand state={state} agents={agents} attentionCount={attentionAgents.length} stale={stale} />

      <div className="grid gap-6 xl:grid-cols-[minmax(0,1fr)_minmax(20rem,0.42fr)]">
        <div className="min-w-0">
          <Section
            title="Attention Groups"
            tier="triage"
            questions={[
              "Which agents need a human now?",
              "Which session should I inspect first?",
              "What changed since the last refresh?"
            ]}
            actions={
              <Button variant="secondary" size="sm" onClick={advanceAttention} disabled={attentionAgents.length === 0}>
                <Bell aria-hidden="true" className="h-4 w-4" />
                Next
              </Button>
            }
          >
            <FleetList agents={attentionAgents.length ? attentionAgents : agents} selectedId={selectedAgent?.id} onSelect={setSelectedAgentId} />
          </Section>

          <Section
            title="Tool Activity"
            tier="triage"
            questions={[
              "Which tools are still running?",
              "Which calls failed?",
              "Where is the verification detail?"
            ]}
          >
            {toolCalls.length ? (
              <div className="grid gap-3 lg:grid-cols-2">
                {toolCalls.slice(0, 6).map((call) => (
                  <ToolCallCard call={call} key={call.id} />
                ))}
              </div>
            ) : (
              <EmptyState title="No command audit rows" />
            )}
          </Section>

          <Section
            title="Fleet Table"
            tier="drill-down"
            questions={[
              "Which sessions are live?",
              "Which rows are stale?",
              "Which row links to detail?"
            ]}
          >
            <FleetTable agents={agents} onSelect={setSelectedAgentId} />
          </Section>
        </div>

        <aside className="min-w-0">
          <Section
            title="Peek Panel"
            tier="drill-down"
            questions={[
              "Why is this agent in its current state?",
              "Which detail surface proves it?",
              "Is raw verification available without flooding the page?"
            ]}
          >
            <AgentPeek agent={selectedAgent} />
          </Section>

          <Section
            title="System Shape"
            tier="overview"
            questions={[
              "Is storage pressure rising?",
              "Which column family is largest?",
              "Is the daemon still local?"
            ]}
          >
            <SystemShape state={state} />
          </Section>
        </aside>
      </div>

      <Section
        title="Transcript Samples"
        tier="drill-down"
        questions={[
          "What did recent agents say?",
          "Was output sanitized before render?",
          "Where is the source row?"
        ]}
      >
        <TranscriptSamples state={state} />
      </Section>
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
        questions={[
          "Is a dashboard session active?",
          "Can the operator mint a cookie session?",
          "Did the login fail closed?"
        ]}
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

function Sidebar({ state, auth }: { state?: DashboardState; auth?: DashboardAuthStatus }) {
  const health = asRecord(panelData(state?.daemon));
  return (
    <nav className="space-y-4" aria-label="Dashboard">
      <div className="flex items-center gap-3">
        <div className="flex h-9 w-9 items-center justify-center rounded-lg border border-border bg-surface-2">
          <LayoutDashboard aria-hidden="true" className="h-5 w-5 text-accent" />
        </div>
        <div>
          <div className="text-md font-semibold text-primary">Synapse</div>
          <div className="text-xs text-muted">{rawText(health.version || "dashboard")}</div>
        </div>
      </div>
      <div className="grid gap-2">
        <SidebarItem icon={<Gauge aria-hidden="true" />} label="Fleet" active />
        <SidebarItem icon={<Rows3 aria-hidden="true" />} label="Actions" />
        <SidebarItem icon={<TerminalSquare aria-hidden="true" />} label="Terminal" />
        <SidebarItem icon={<CheckCircle2 aria-hidden="true" />} label="Approvals" />
      </div>
      <div className="rounded-lg border border-border bg-surface-2 p-3">
        <div className="text-label font-medium uppercase text-muted">Loopback</div>
        <div className="mt-1 truncate font-mono text-sm text-primary">{state?.bind_addr || "pending"}</div>
      </div>
      <div className="rounded-lg border border-border bg-surface-2 p-3">
        <div className="text-label font-medium uppercase text-muted">Auth</div>
        <div className="mt-1 truncate font-mono text-sm text-primary">{auth?.authenticated ? auth.method : "locked"}</div>
      </div>
    </nav>
  );
}

function SidebarItem({ icon, label, active = false }: { icon: ReactNode; label: string; active?: boolean }) {
  return (
    <a
      href="#"
      className={`flex min-h-10 items-center gap-2 rounded-md px-3 text-sm ${active ? "bg-surface-2 text-primary" : "text-secondary hover:bg-surface-2 hover:text-primary"}`}
    >
      <span className="h-4 w-4">{icon}</span>
      {label}
    </a>
  );
}

function DensityControl({
  density,
  setDensity
}: {
  density: "comfortable" | "compact";
  setDensity: (density: "comfortable" | "compact") => void;
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
    <Section
      title="Overview"
      tier="overview"
      questions={[
        "Is anything wrong?",
        "How many agents are live?",
        "Is the daemon stale?"
      ]}
    >
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <StatCard label="Attention" value={attentionCount} status={attentionCount ? "needs_input" : "done"} delta={attentionCount ? "human review queued" : "quiet"} />
        <StatCard label="Live Agents" value={liveAgents} status={liveAgents ? "working" : "idle"} delta={`${agents.length} total rows`} />
        <StatCard label="Tools" value={toolCount} status={toolCount ? "done" : "stuck"} delta="strict client surface" />
        <StatCard label="Freshness" value={stale ? "stale" : "live"} status={stale ? "stuck" : "working"} delta={storagePressure} />
      </div>
    </Section>
  );
}

function FleetList({
  agents,
  selectedId,
  onSelect
}: {
  agents: AgentSummary[];
  selectedId?: string;
  onSelect: (id: string) => void;
}) {
  if (agents.length === 0) return <EmptyState title="No agent rows" />;
  return (
    <div className="rounded-lg border border-border bg-surface-1">
      {agents.map((agent) => (
        <FleetRow key={agent.id} agent={agent} selected={agent.id === selectedId} onSelect={() => onSelect(agent.id)} />
      ))}
    </div>
  );
}

function FleetTable({ agents, onSelect }: { agents: AgentSummary[]; onSelect: (id: string) => void }) {
  if (agents.length === 0) return <EmptyState title="No fleet rows" />;
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
        }
      ]}
    />
  );
}

function SystemShape({ state }: { state?: DashboardState }) {
  const storage = asRecord(panelData(state?.storage));
  const counts = asRecord(storage.cf_row_counts);
  const chartData = Object.entries(counts)
    .map(([name, value]) => ({ name: name.replace("CF_", ""), rows: Number(value) || 0 }))
    .sort((a, b) => b.rows - a.rows)
    .slice(0, 8);
  if (!chartData.length) return <EmptyState title="No storage rows" />;
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
  if (!rows.length) return <EmptyState title="No transcript rows" />;
  return (
    <div className="grid gap-3 lg:grid-cols-2">
      {rows.map((row, index) => (
        <TranscriptTurn key={`${rawText(row.spawn_id)}-${rawText(row.line_no)}-${index}`} row={row} />
      ))}
    </div>
  );
}
