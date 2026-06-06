//! CDP probe + attach for Chromium-family foregrounds.
//!
//! The diagnostic *types* ([`CdpDiagnostics`], [`CdpStatus`], [`CdpCapability`])
//! live in `synapse-core` because they are embedded in every `Observation`.
//! This module owns the *behaviour*: detecting a Chromium foreground, probing
//! for a reachable remote-debugging port, and attaching a `chromiumoxide`
//! client. It also owns the launched-port registry that ties
//! `act_launch` (#684) to the probe so a Synapse-launched browser is found
//! without the agent remembering manual flags.
//!
//! Background (research, 2026-06): since Chrome 136 the `--remote-debugging-port`
//! switch is ignored unless paired with a non-default `--user-data-dir`, so a
//! normally-launched Chrome on the user's primary profile can *never* expose a
//! debug port. That is why a normal launch probes `Unreachable` and why #684
//! must launch with a dedicated automation profile.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    sync::{Mutex, OnceLock},
    time::Duration,
};

use synapse_core::error_codes;
use tokio::{net::TcpStream as TokioTcpStream, time::timeout};

pub use synapse_core::{CdpCapability, CdpDiagnostics, CdpStatus};

#[cfg(windows)]
use crate::{A11yError, A11yResult};

/// Default remote-debugging port probed when no launched port is registered for
/// the foreground process. 9222 is Chrome's conventional debug port.
pub const DEFAULT_CDP_PORT: u16 = 9222;

/// Environment override for the probed port list, e.g. `9222,9333`.
const CDP_PORTS_ENV: &str = "SYNAPSE_CDP_PORTS";

#[must_use]
pub fn cdp_capabilities() -> Vec<CdpCapability> {
    vec![
        CdpCapability::DomSnapshot,
        CdpCapability::AccessibilityFullAxTree,
        CdpCapability::DomQuerySelector,
        CdpCapability::PageCaptureScreenshot,
    ]
}

#[must_use]
pub fn is_chromium_family(process_name: &str) -> bool {
    let lower = process_name.to_ascii_lowercase();
    [
        "chrome.exe",
        "chromium.exe",
        "msedge.exe",
        "brave.exe",
        "vivaldi.exe",
        "opera.exe",
        "chrome",
        "chromium",
        "msedge",
        "brave",
        "vivaldi",
        "opera",
    ]
    .iter()
    .any(|candidate| lower.ends_with(candidate))
}

// === Launched-port registry =================================================
//
// `act_launch` (#684) registers the ephemeral debug port it opened for a
// browser it launched, keyed by the browser's process id. The observe/find
// probe consults this registry first so a Synapse-launched browser is attached
// without any default-port collision or manual flag.

fn registry() -> &'static Mutex<HashMap<u32, u16>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u32, u16>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Records the CDP debug `port` that `act_launch` opened for browser process
/// `pid`, so a later `observe`/`find` can find it.
pub fn register_launched_port(pid: u32, port: u16) {
    if let Ok(mut map) = registry().lock() {
        map.insert(pid, port);
        tracing::info!(
            code = "A11Y_CDP_PORT_REGISTERED",
            pid,
            port,
            "registered Synapse-launched CDP debug port"
        );
    }
}

/// Removes a registered port (e.g. when the browser process exits).
pub fn forget_launched_port(pid: u32) {
    if let Ok(mut map) = registry().lock() {
        map.remove(&pid);
    }
}

/// The CDP debug port registered for `pid` by `act_launch`, if any.
#[must_use]
pub fn launched_port_for_pid(pid: u32) -> Option<u16> {
    registry()
        .lock()
        .ok()
        .and_then(|map| map.get(&pid).copied())
}

/// The ordered list of ports to probe for `pid`: the registered launched port
/// (if any) first, then the env-configured / default port list. De-duplicated,
/// order-preserving.
#[must_use]
pub fn candidate_ports_for_pid(pid: u32) -> Vec<u16> {
    let mut ports = Vec::new();
    if let Some(port) = launched_port_for_pid(pid) {
        ports.push(port);
    }
    for port in configured_ports() {
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    ports
}

fn configured_ports() -> Vec<u16> {
    std::env::var(CDP_PORTS_ENV).map_or_else(
        |_| vec![DEFAULT_CDP_PORT],
        |raw| {
            let parsed: Vec<u16> = raw
                .split(',')
                .filter_map(|token| token.trim().parse::<u16>().ok())
                .filter(|port| *port != 0)
                .collect();
            if parsed.is_empty() {
                vec![DEFAULT_CDP_PORT]
            } else {
                parsed
            }
        },
    )
}

// === Probing ================================================================

fn endpoint_for_port(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn endpoints_for_ports(ports: &[u16]) -> Vec<String> {
    ports.iter().map(|port| endpoint_for_port(*port)).collect()
}

fn ok_diagnostics(process_name: &str, port: u16, checked_ports: Vec<u16>) -> CdpDiagnostics {
    CdpDiagnostics {
        process_name: process_name.to_owned(),
        status: CdpStatus::Ok,
        endpoint: Some(endpoint_for_port(port)),
        checked_endpoints: endpoints_for_ports(&checked_ports),
        checked_ports,
        reason_code: None,
        detail: None,
        capabilities: cdp_capabilities(),
        attached_node_count: None,
        selected_target_id: None,
        selected_session_id: None,
        target_selection_reason: None,
        target_candidate_count: None,
    }
}

/// Synchronous CDP reachability probe.
///
/// Used from the perception `platform_input` path so both `observe` and `find`
/// surface `cdp.status` without an async runtime. Connection-refused on loopback
/// returns immediately, so the common "no debug port" case costs microseconds,
/// not the full `connect_timeout`.
#[must_use]
pub fn probe_chromium_cdp_blocking(
    process_name: &str,
    ports: &[u16],
    connect_timeout: Duration,
) -> CdpDiagnostics {
    if !is_chromium_family(process_name) {
        return CdpDiagnostics::not_chromium(process_name);
    }
    let mut checked_ports = Vec::new();
    for port in ports {
        checked_ports.push(*port);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *port);
        if TcpStream::connect_timeout(&addr, connect_timeout).is_ok() {
            return ok_diagnostics(process_name, *port, checked_ports);
        }
    }
    CdpDiagnostics::unreachable_with_probe(
        process_name,
        error_codes::A11Y_CDP_UNREACHABLE,
        checked_ports,
        "no reachable loopback CDP endpoint on checked ports; existing Chrome attach requires remote debugging to be enabled for that running browser instance",
    )
}

/// Async CDP reachability probe (used by tests and the async attach path).
pub async fn probe_chromium_cdp(
    process_name: &str,
    ports: &[u16],
    connect_timeout: Duration,
) -> CdpDiagnostics {
    if !is_chromium_family(process_name) {
        return CdpDiagnostics::not_chromium(process_name);
    }

    let mut checked_ports = Vec::new();
    for port in ports {
        checked_ports.push(*port);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *port);
        if timeout(connect_timeout, TokioTcpStream::connect(addr))
            .await
            .is_ok_and(|result| result.is_ok())
        {
            return ok_diagnostics(process_name, *port, checked_ports);
        }
    }

    CdpDiagnostics::unreachable_with_probe(
        process_name,
        error_codes::A11Y_CDP_UNREACHABLE,
        checked_ports,
        "no reachable loopback CDP endpoint on checked ports; existing Chrome attach requires remote debugging to be enabled for that running browser instance",
    )
}

/// Resolves a reachable CDP endpoint for the browser window `hwnd`.
///
/// Used by action routing (#686). Looks up the window's pid, gathers its
/// candidate debug ports (launched-port registry first, then defaults), and
/// returns the first reachable `http://127.0.0.1:<port>`. `None` if the window
/// is gone, not a Chromium browser, or has no reachable debug port.
#[cfg(windows)]
#[must_use]
pub fn endpoint_for_window(hwnd: i64) -> Option<String> {
    let context = crate::foreground_context(hwnd).ok()?;
    let ports = candidate_ports_for_pid(context.pid);
    probe_chromium_cdp_blocking(&context.process_name, &ports, Duration::from_millis(250)).endpoint
}

#[cfg(windows)]
#[derive(Debug)]
pub struct CdpAttachment {
    pub browser: chromiumoxide::Browser,
    pub handler: chromiumoxide::Handler,
    pub endpoint: String,
}

/// Attaches a `chromiumoxide` browser client to a reachable CDP endpoint.
///
/// # Errors
///
/// Returns `A11Y_CDP_UNREACHABLE` when `chromiumoxide` cannot connect to the
/// supplied endpoint.
#[cfg(windows)]
pub async fn attach_chromiumoxide(endpoint: &str) -> A11yResult<CdpAttachment> {
    let (browser, handler) = chromiumoxide::Browser::connect(endpoint)
        .await
        .map_err(|err| A11yError::CdpUnreachable {
            detail: err.to_string(),
        })?;
    Ok(CdpAttachment {
        browser,
        handler,
        endpoint: endpoint.to_owned(),
    })
}
