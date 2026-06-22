import assert from "node:assert/strict";
import { describe, test } from "node:test";

import { buildAgents, type DashboardState } from "../src/lib/dashboard-state";

function panel(data: unknown) {
  return { status: "ok" as const, source: "test", data };
}

function dashboardState(sessionsData: unknown): DashboardState {
  return {
    schema_version: 1,
    generated_at_unix_ms: 1,
    bind_addr: "127.0.0.1:7700",
    token_policy: "test",
    auth: panel({}),
    daemon: panel({}),
    sessions: panel(sessionsData),
    lease: panel({}),
    storage: panel({}),
    target_claims: panel({}),
    timeline: panel({}),
    events: panel({}),
    hidden_desktops: panel({}),
    cdp_attachments: panel({}),
    shell_jobs: panel({}),
    command_audit: panel({ rows: [] }),
    approvals: panel({}),
    suggestions: panel({}),
    armed_runs: panel({}),
    agent_transcripts: panel({ rows: [] }),
    hygiene: panel({}),
    local_models: panel({})
  };
}

function liveSession(state: string, reason = "session_initialized", lastSeenMs = 900_000) {
  return {
    session_id: `session-${state}-${reason}`,
    lifecycle: "live",
    agent_kind: "codex",
    last_seen_ms_ago: lastSeenMs,
    last_action: "tools/call:health",
    agent_state: {
      state,
      reason_code: reason
    }
  };
}

describe("buildAgents live session status", () => {
  test("keeps stale idle live sessions out of actionable attention", () => {
    const agents = buildAgents(
      dashboardState({
        sessions: [liveSession("idle", "session_initialized", 900_000)],
        unbound_agent_states: [],
        terminal_unbound_agent_states: []
      })
    );

    assert.equal(agents.length, 1);
    assert.equal(agents[0].status, "idle");
    assert.equal(agents[0].lastSeenMs, 900_000);
  });

  test("still surfaces explicit backend attention states", () => {
    const agents = buildAgents(
      dashboardState({
        sessions: [
          liveSession("stuck", "silent_timeout", 10),
          liveSession("needs_input", "operator_reply", 10),
          liveSession("awaiting_approval", "approval", 10),
          liveSession("ready_for_review", "review", 10)
        ],
        unbound_agent_states: []
      })
    );

    assert.deepEqual(agents.map((agent) => agent.status), [
      "stuck",
      "needs_input",
      "awaiting_approval",
      "ready_for_review"
    ]);
  });

  test("preserves terminal failure classification", () => {
    const agents = buildAgents(
      dashboardState({
        sessions: [liveSession("dead", "agent_spawn_failed", 900_000)],
        unbound_agent_states: []
      })
    );

    assert.equal(agents[0].status, "failed");
  });

  test("does not make terminal attached registry failures actionable", () => {
    const agents = buildAgents(
      dashboardState({
        attached_agent_registry: {
          rows: [
            {
              registry_id: "agent-spawn-local-dead",
              kind: "local-model",
              lifecycle: "dead",
              state: "dead",
              reason_code: "local_model_registry_row_missing",
              counts_as_live: false,
              kill_handle: {
                available: true,
                target_id: "agent-spawn-local-dead"
              }
            }
          ]
        },
        sessions: [],
        unbound_agent_states: [],
        terminal_unbound_agent_states: []
      })
    );

    assert.equal(agents.length, 1);
    assert.equal(agents[0].status, "failed");
    assert.equal(agents[0].killable, false);
  });

  test("falls back to stale timing only for unknown live lifecycle rows", () => {
    const agents = buildAgents(
      dashboardState({
        sessions: [
          {
            session_id: "legacy-live-row",
            lifecycle: "live",
            agent_kind: "legacy",
            last_seen_ms_ago: 900_000,
            last_action: ""
          }
        ],
        unbound_agent_states: []
      })
    );

    assert.equal(agents[0].status, "stuck");
  });

  test("derives latest transcript summary and usage for fleet rows", () => {
    const state = dashboardState({
      sessions: [
        {
          session_id: "session-with-spawn",
          spawn_id: "agent-spawn-usage",
          lifecycle: "live",
          agent_kind: "codex",
          last_seen_ms_ago: 50,
          last_action: "tools/call:health",
          agent_state: {
            state: "live",
            reason_code: "working"
          }
        }
      ],
      unbound_agent_states: []
    });
    state.agent_transcripts = panel({
      rows: [
        {
          spawn_id: "agent-spawn-usage",
          line_no: 1,
          record: {
            content_summary: "Older turn",
            usage: {
              input_tokens: 10,
              output_tokens: 2
            }
          }
        },
        {
          spawn_id: "agent-spawn-usage",
          line_no: 2,
          record: {
            content_summary: "Latest agent_query-style activity summary",
            usage: {
              input_tokens: 1000,
              output_tokens: 200,
              cache_read_input_tokens: 5000,
              cache_creation_input_tokens: 300,
              reasoning_output_tokens: 50,
              total_cost_micro_usd: 12345
            }
          }
        }
      ]
    });

    const [agent] = buildAgents(state);

    assert.equal(agent.summary, "Latest agent_query-style activity summary");
    assert.equal(agent.diffStats.transcripts, 2);
    assert.deepEqual(agent.usage, {
      inputTokens: 1000,
      outputTokens: 200,
      cacheReadInputTokens: 5000,
      cacheCreationInputTokens: 300,
      reasoningOutputTokens: 50,
      totalCostMicroUsd: 12345,
      sourceLine: 2
    });
  });
});
