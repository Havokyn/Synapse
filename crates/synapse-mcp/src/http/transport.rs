use std::{io, net::SocketAddr, process::ExitCode, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{Json, Router, extract::State, middleware, routing::get};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use synapse_core::Health;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    http::auth::{self, HttpAuth},
    http::session,
    server::SynapseService,
};

type McpHttpService = StreamableHttpService<SynapseService, LocalSessionManager>;

#[derive(Clone)]
struct HttpState {
    health_service: Arc<SynapseService>,
}

pub(super) async fn serve(bind: &str, allow_non_loopback: bool) -> anyhow::Result<ExitCode> {
    synapse_action::install_panic_hook();
    let addr = bind
        .parse::<SocketAddr>()
        .with_context(|| format!("parse HTTP bind address {bind}"))?;
    if !addr.ip().is_loopback() {
        if !allow_non_loopback {
            tracing::error!(
                code = synapse_core::error_codes::HTTP_BIND_NON_LOOPBACK_REFUSED,
                bind = %addr,
                "refusing non-loopback HTTP bind without --allow-non-loopback"
            );
            return Ok(ExitCode::from(2));
        }
        tracing::warn!(
            code = "MCP_HTTP_NON_LOOPBACK_BIND_ALLOWED",
            bind = %addr,
            "non-loopback HTTP bind allowed by explicit operator flag"
        );
    }
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind HTTP MCP transport to {addr}"))?;
    let local_addr = listener
        .local_addr()
        .context("read HTTP listener address")?;
    let shutdown_cancel = CancellationToken::new();
    let connection_closed_cancel = CancellationToken::new();
    let app = router(
        shutdown_cancel.clone(),
        connection_closed_cancel.clone(),
        local_addr,
    )
    .context("build HTTP MCP router")?;

    tracing::info!(
        code = "MCP_HTTP_STARTED",
        bind = %local_addr,
        "starting streamable HTTP MCP transport"
    );

    let mut server_task = spawn_server(listener, app, shutdown_cancel.clone());
    let code = tokio::select! {
        result = &mut server_task => {
            result.context("join HTTP MCP transport")?
                .context("serve HTTP MCP transport")?;
            ExitCode::SUCCESS
        }
        signal = wait_for_shutdown_signal("http") => {
            signal?;
            tracing::info!(code = "MCP_SHUTDOWN_GRACEFUL", "HTTP shutdown signal received");
            shutdown_cancel.cancel();
            connection_closed_cancel.cancel();
            wait_for_server_stop(&mut server_task).await?;
            ExitCode::SUCCESS
        }
    };
    Ok(code)
}

fn router(
    shutdown_cancel: CancellationToken,
    connection_closed_cancel: CancellationToken,
    bind_addr: SocketAddr,
) -> anyhow::Result<Router> {
    let auth = Arc::new(HttpAuth::load(bind_addr).context("load HTTP bearer token")?);
    tracing::info!(
        code = "MCP_HTTP_AUTH_CONFIGURED",
        source = auth.source_label(),
        "HTTP bearer token configured"
    );
    let health_service = Arc::new(
        http_service(shutdown_cancel.clone(), connection_closed_cancel.clone())
            .context("initialize HTTP health service state")?,
    );
    let mcp_service = streamable_service(shutdown_cancel, connection_closed_cancel)
        .context("initialize HTTP MCP session state")?;
    let state = HttpState { health_service };
    Ok(Router::new()
        .route("/health", get(health))
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn(session::require_mcp_session))
        .layer(middleware::from_fn_with_state(
            auth,
            auth::require_http_security,
        ))
        .with_state(state))
}

fn streamable_service(
    shutdown_cancel: CancellationToken,
    connection_closed_cancel: CancellationToken,
) -> anyhow::Result<McpHttpService> {
    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(shutdown_cancel.child_token());
    let mut session_manager = LocalSessionManager::default();
    session_manager.session_config =
        session::load_session_config().context("load HTTP session config")?;
    Ok(StreamableHttpService::new(
        move || http_service(shutdown_cancel.clone(), connection_closed_cancel.clone()),
        Arc::new(session_manager),
        config,
    ))
}

fn http_service(
    shutdown_cancel: CancellationToken,
    connection_closed_cancel: CancellationToken,
) -> io::Result<SynapseService> {
    SynapseService::try_with_m2_shutdown_reason(shutdown_cancel, "http", connection_closed_cancel)
        .map_err(|error| io::Error::other(format!("{error:#}")))
}

async fn health(State(state): State<HttpState>) -> Json<Health> {
    tracing::info!(
        code = "MCP_HTTP_HEALTH",
        "tool.invocation kind=health transport=http"
    );
    Json(state.health_service.health_payload())
}

fn spawn_server(
    listener: TcpListener,
    app: Router,
    shutdown_cancel: CancellationToken,
) -> JoinHandle<io::Result<()>> {
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown_cancel.cancelled_owned().await })
            .await
    })
}

async fn wait_for_server_stop(server_task: &mut JoinHandle<io::Result<()>>) -> anyhow::Result<()> {
    match tokio::time::timeout(Duration::from_secs(2), &mut *server_task).await {
        Ok(result) => {
            result
                .context("join stopped HTTP MCP transport")?
                .context("stop HTTP MCP transport")?;
        }
        Err(_elapsed) => {
            server_task.abort();
            tracing::warn!(
                code = "MCP_HTTP_SHUTDOWN_TIMEOUT",
                "HTTP transport did not stop within shutdown timeout"
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn wait_for_shutdown_signal(phase: &'static str) -> anyhow::Result<()> {
    let mut ctrl_break = tokio::signal::windows::ctrl_break()
        .with_context(|| format!("register ctrl-break handler {phase}"))?;
    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal.with_context(|| format!("wait for ctrl-c {phase}"))?;
        }
        received = ctrl_break.recv() => {
            if received.is_none() {
                anyhow::bail!("ctrl-break stream ended while waiting for shutdown signal {phase}");
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
async fn wait_for_shutdown_signal(phase: &'static str) -> anyhow::Result<()> {
    tokio::signal::ctrl_c()
        .await
        .with_context(|| format!("wait for ctrl-c {phase}"))
}
