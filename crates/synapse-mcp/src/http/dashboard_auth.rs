use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq;
use synapse_core::error_codes;
use synapse_storage::{Db, cf};
use uuid::Uuid;

use super::auth::HttpAuth;

const SESSION_SCHEMA_VERSION: u32 = 1;
const FAILURE_SCHEMA_VERSION: u32 = 1;
const SESSION_PREFIX: &str = "dashboard-auth/v1/session/";
const FAILURE_PREFIX: &str = "dashboard-auth/v1/failure/";
const SESSION_TTL_ENV: &str = "SYNAPSE_DASHBOARD_SESSION_TTL_SECS";
const DEFAULT_SESSION_TTL_SECS: u64 = 12 * 60 * 60;
const MAX_RECENT_FAILURES: usize = 50;
pub(super) const SESSION_COOKIE_NAME: &str = "synapse_dashboard_session";
const SOURCE_OF_TRUTH: &str = "CF_KV dashboard-auth/v1";

#[derive(Clone)]
pub(super) struct DashboardAuth {
    db: Arc<Db>,
    bearer: Arc<HttpAuth>,
    bind_addr: SocketAddr,
    ttl: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CsrfPolicy {
    NotRequired,
    Required,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DashboardAuthMethod {
    Bearer,
    Cookie,
}

#[derive(Clone, Debug)]
pub(super) struct DashboardAuthContext {
    pub method: DashboardAuthMethod,
    pub session: Option<DashboardSessionRow>,
}

#[derive(Clone, Debug)]
pub(super) struct DashboardLoginSuccess {
    pub session_cookie_value: String,
    pub csrf_token: String,
    pub expires_unix_ms: u64,
}

#[derive(Clone, Debug)]
pub(super) struct DashboardLogoutSuccess {
    pub revoked_row_key: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct DashboardStatusSuccess {
    pub authenticated: bool,
    pub method: DashboardAuthMethod,
    pub csrf_token: Option<String>,
    pub expires_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct DashboardSessionRow {
    pub schema_version: u32,
    pub row_key: String,
    pub session_digest: String,
    pub csrf_digest: String,
    pub created_unix_ms: u64,
    pub expires_unix_ms: u64,
    pub last_seen_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct DashboardAuthFailureRow {
    schema_version: u32,
    row_key: String,
    at_unix_ms: u64,
    method: String,
    path: String,
    reason: String,
    authorization_present: bool,
    cookie_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(super) struct DashboardAuthSnapshot {
    pub source_of_truth: &'static str,
    pub session_row_count: usize,
    pub active_session_count: usize,
    pub revoked_session_count: usize,
    pub expired_session_count: usize,
    pub failure_count: usize,
    pub corrupt_session_rows: usize,
    pub corrupt_failure_rows: usize,
    pub recent_failures: Vec<DashboardAuthFailurePublic>,
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(super) struct DashboardAuthFailurePublic {
    pub row_key: String,
    pub at_unix_ms: u64,
    pub method: String,
    pub path: String,
    pub reason: String,
    pub authorization_present: bool,
    pub cookie_present: bool,
}

impl DashboardAuth {
    pub(super) fn new(db: Arc<Db>, bearer: Arc<HttpAuth>, bind_addr: SocketAddr) -> Self {
        Self {
            db,
            bearer,
            bind_addr,
            ttl: load_session_ttl(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_ttl_for_test(
        db: Arc<Db>,
        bearer: Arc<HttpAuth>,
        bind_addr: SocketAddr,
        ttl: Duration,
    ) -> Self {
        Self {
            db,
            bearer,
            bind_addr,
            ttl,
        }
    }

    pub(super) fn login(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
        credential: &str,
    ) -> Result<DashboardLoginSuccess, Response> {
        self.validate_origin_or_record(headers, method, path)?;
        if !self.bearer.token_matches(credential.trim()) {
            self.record_failure(headers, method, path, "login_token_invalid");
            return Err(error_response(
                StatusCode::UNAUTHORIZED,
                error_codes::HTTP_TOKEN_INVALID,
                "dashboard login token rejected",
            ));
        }

        let now = unix_time_ms();
        let session_cookie_value = random_secret();
        let csrf_token = random_secret();
        let session_digest = digest_hex(session_cookie_value.as_bytes());
        let row_key = session_row_key(&session_digest);
        let row = DashboardSessionRow {
            schema_version: SESSION_SCHEMA_VERSION,
            row_key: row_key.clone(),
            session_digest,
            csrf_digest: digest_hex(csrf_token.as_bytes()),
            created_unix_ms: now,
            expires_unix_ms: now.saturating_add(duration_millis_u64(self.ttl)),
            last_seen_unix_ms: now,
            revoked_unix_ms: None,
        };
        self.write_session_row(&row)?;
        tracing::info!(
            code = "DASHBOARD_AUTH_SESSION_CREATED",
            row_key,
            expires_unix_ms = row.expires_unix_ms,
            source_of_truth = SOURCE_OF_TRUTH,
            "dashboard cookie session row written"
        );
        Ok(DashboardLoginSuccess {
            session_cookie_value,
            csrf_token,
            expires_unix_ms: row.expires_unix_ms,
        })
    }

    pub(super) fn status(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
    ) -> Result<DashboardStatusSuccess, Response> {
        let auth = self.authenticate(headers, method, path, CsrfPolicy::NotRequired)?;
        match auth.method {
            DashboardAuthMethod::Bearer => Ok(DashboardStatusSuccess {
                authenticated: true,
                method: DashboardAuthMethod::Bearer,
                csrf_token: None,
                expires_unix_ms: None,
            }),
            DashboardAuthMethod::Cookie => {
                let Some(row) = auth.session else {
                    self.record_failure(headers, method, path, "session_row_missing");
                    return Err(error_response(
                        StatusCode::UNAUTHORIZED,
                        error_codes::HTTP_SESSION_INVALID,
                        "dashboard session row missing",
                    ));
                };
                let (csrf_token, updated) = self.rotate_csrf(row)?;
                Ok(DashboardStatusSuccess {
                    authenticated: true,
                    method: DashboardAuthMethod::Cookie,
                    csrf_token: Some(csrf_token),
                    expires_unix_ms: Some(updated.expires_unix_ms),
                })
            }
        }
    }

    pub(super) fn logout(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
    ) -> Result<DashboardLogoutSuccess, Response> {
        let auth = self.authenticate(headers, method, path, CsrfPolicy::Required)?;
        let Some(mut row) = auth.session else {
            return Ok(DashboardLogoutSuccess {
                revoked_row_key: None,
            });
        };
        row.revoked_unix_ms = Some(unix_time_ms());
        self.write_session_row(&row)?;
        tracing::info!(
            code = "DASHBOARD_AUTH_SESSION_REVOKED",
            row_key = row.row_key,
            source_of_truth = SOURCE_OF_TRUTH,
            "dashboard cookie session row revoked"
        );
        Ok(DashboardLogoutSuccess {
            revoked_row_key: Some(row.row_key),
        })
    }

    pub(super) fn authenticate(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
        csrf_policy: CsrfPolicy,
    ) -> Result<DashboardAuthContext, Response> {
        self.validate_origin_or_record(headers, method, path)?;

        if headers.contains_key(header::AUTHORIZATION) {
            return match self.bearer.authorize(headers) {
                Ok(()) => Ok(DashboardAuthContext {
                    method: DashboardAuthMethod::Bearer,
                    session: None,
                }),
                Err(failure) => {
                    self.record_failure(
                        headers,
                        method,
                        path,
                        &format!("bearer_{failure:?}").to_ascii_lowercase(),
                    );
                    Err(error_response(
                        StatusCode::UNAUTHORIZED,
                        error_codes::HTTP_TOKEN_INVALID,
                        "dashboard bearer token rejected",
                    ))
                }
            };
        }

        let Some(session_cookie) = cookie_value(headers, SESSION_COOKIE_NAME) else {
            self.record_failure(headers, method, path, "session_cookie_missing");
            return Err(error_response(
                StatusCode::UNAUTHORIZED,
                error_codes::HTTP_SESSION_INVALID,
                "dashboard session cookie missing",
            ));
        };
        if !session_cookie.chars().all(|ch| ch.is_ascii_hexdigit()) || session_cookie.len() < 32 {
            self.record_failure(headers, method, path, "session_cookie_malformed");
            return Err(error_response(
                StatusCode::UNAUTHORIZED,
                error_codes::HTTP_SESSION_INVALID,
                "dashboard session cookie malformed",
            ));
        }
        let mut row = match self.read_session_by_cookie(session_cookie)? {
            Some(row) => row,
            None => {
                self.record_failure(headers, method, path, "session_cookie_unknown");
                return Err(error_response(
                    StatusCode::UNAUTHORIZED,
                    error_codes::HTTP_SESSION_INVALID,
                    "dashboard session cookie unknown",
                ));
            }
        };
        if row.revoked_unix_ms.is_some() {
            self.record_failure(headers, method, path, "session_cookie_revoked");
            return Err(error_response(
                StatusCode::UNAUTHORIZED,
                error_codes::HTTP_SESSION_INVALID,
                "dashboard session cookie revoked",
            ));
        }
        let now = unix_time_ms();
        if row.expires_unix_ms <= now {
            self.record_failure(headers, method, path, "session_cookie_expired");
            return Err(error_response(
                StatusCode::UNAUTHORIZED,
                error_codes::HTTP_SESSION_INVALID,
                "dashboard session cookie expired",
            ));
        }
        if csrf_policy == CsrfPolicy::Required {
            self.validate_csrf(headers, &row, method, path)?;
        }
        row.last_seen_unix_ms = now;
        self.write_session_row(&row)?;
        Ok(DashboardAuthContext {
            method: DashboardAuthMethod::Cookie,
            session: Some(row),
        })
    }

    pub(super) fn snapshot(&self) -> DashboardAuthSnapshot {
        let now = unix_time_ms();
        let mut sessions = Vec::new();
        let mut corrupt_session_rows = 0;
        if let Ok(rows) = self.db.scan_cf_prefix(cf::CF_KV, SESSION_PREFIX.as_bytes()) {
            for (_key, value) in rows {
                match serde_json::from_slice::<DashboardSessionRow>(&value) {
                    Ok(row) => sessions.push(row),
                    Err(_error) => corrupt_session_rows += 1,
                }
            }
        }

        let mut failures = Vec::new();
        let mut corrupt_failure_rows = 0;
        if let Ok(rows) = self.db.scan_cf_prefix(cf::CF_KV, FAILURE_PREFIX.as_bytes()) {
            for (_key, value) in rows {
                match serde_json::from_slice::<DashboardAuthFailureRow>(&value) {
                    Ok(row) => failures.push(row),
                    Err(_error) => corrupt_failure_rows += 1,
                }
            }
        }
        failures.sort_by(|left, right| left.row_key.cmp(&right.row_key));
        let recent_failures = failures
            .iter()
            .rev()
            .take(MAX_RECENT_FAILURES)
            .map(DashboardAuthFailurePublic::from)
            .collect::<Vec<_>>();

        DashboardAuthSnapshot {
            source_of_truth: SOURCE_OF_TRUTH,
            session_row_count: sessions.len(),
            active_session_count: sessions
                .iter()
                .filter(|row| row.revoked_unix_ms.is_none() && row.expires_unix_ms > now)
                .count(),
            revoked_session_count: sessions
                .iter()
                .filter(|row| row.revoked_unix_ms.is_some())
                .count(),
            expired_session_count: sessions
                .iter()
                .filter(|row| row.expires_unix_ms <= now)
                .count(),
            failure_count: failures.len(),
            corrupt_session_rows,
            corrupt_failure_rows,
            recent_failures,
        }
    }

    fn validate_origin_or_record(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
    ) -> Result<(), Response> {
        if !self.bind_addr.ip().is_loopback() {
            self.record_failure(headers, method, path, "bind_non_loopback");
            return Err(error_response(
                StatusCode::FORBIDDEN,
                error_codes::HTTP_ORIGIN_REFUSED,
                "dashboard requires a loopback bind",
            ));
        }
        match self.bearer.validate_origin_and_host(headers) {
            Ok(()) => Ok(()),
            Err(failure) => {
                self.record_failure(
                    headers,
                    method,
                    path,
                    &format!("origin_{failure:?}").to_ascii_lowercase(),
                );
                Err(error_response(
                    StatusCode::FORBIDDEN,
                    error_codes::HTTP_ORIGIN_REFUSED,
                    "dashboard host or origin refused",
                ))
            }
        }
    }

    fn validate_csrf(
        &self,
        headers: &HeaderMap,
        row: &DashboardSessionRow,
        method: &str,
        path: &str,
    ) -> Result<(), Response> {
        let Some(raw) = headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
        else {
            self.record_failure(headers, method, path, "csrf_missing");
            return Err(error_response(
                StatusCode::FORBIDDEN,
                error_codes::HTTP_SESSION_INVALID,
                "dashboard CSRF token missing",
            ));
        };
        let candidate = digest_hex(raw.as_bytes());
        if bool::from(candidate.as_bytes().ct_eq(row.csrf_digest.as_bytes())) {
            Ok(())
        } else {
            self.record_failure(headers, method, path, "csrf_invalid");
            Err(error_response(
                StatusCode::FORBIDDEN,
                error_codes::HTTP_SESSION_INVALID,
                "dashboard CSRF token invalid",
            ))
        }
    }

    fn rotate_csrf(
        &self,
        mut row: DashboardSessionRow,
    ) -> Result<(String, DashboardSessionRow), Response> {
        let csrf_token = random_secret();
        row.csrf_digest = digest_hex(csrf_token.as_bytes());
        row.last_seen_unix_ms = unix_time_ms();
        self.write_session_row(&row)?;
        Ok((csrf_token, row))
    }

    fn read_session_by_cookie(
        &self,
        session_cookie: &str,
    ) -> Result<Option<DashboardSessionRow>, Response> {
        let digest = digest_hex(session_cookie.as_bytes());
        let key = session_row_key(&digest);
        let rows = self
            .db
            .scan_cf_prefix(cf::CF_KV, key.as_bytes())
            .map_err(|error| storage_error_response("dashboard session read failed", error))?;
        let Some((_row_key, value)) = rows
            .into_iter()
            .find(|(row_key, _value)| row_key == key.as_bytes())
        else {
            return Ok(None);
        };
        let row = serde_json::from_slice::<DashboardSessionRow>(&value).map_err(|error| {
            tracing::error!(
                code = error_codes::STORAGE_CORRUPTED,
                row_key = key,
                detail = %error,
                "dashboard session row decode failed"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_codes::STORAGE_CORRUPTED,
                "dashboard session row corrupt",
            )
        })?;
        Ok(Some(row))
    }

    fn write_session_row(&self, row: &DashboardSessionRow) -> Result<(), Response> {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            tracing::error!(
                code = error_codes::STORAGE_WRITE_FAILED,
                row_key = row.row_key,
                detail = %error,
                "dashboard session row encode failed"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_codes::STORAGE_WRITE_FAILED,
                "dashboard session row encode failed",
            )
        })?;
        self.db
            .put_batch_pressure_bypass(cf::CF_KV, [(row.row_key.as_bytes().to_vec(), encoded)])
            .map_err(|error| storage_error_response("dashboard session row write failed", error))
    }

    fn record_failure(&self, headers: &HeaderMap, method: &str, path: &str, reason: &str) {
        let now = unix_time_ms();
        let row_key = format!("{FAILURE_PREFIX}{now:013}/{}", Uuid::new_v4().simple());
        let row = DashboardAuthFailureRow {
            schema_version: FAILURE_SCHEMA_VERSION,
            row_key: row_key.clone(),
            at_unix_ms: now,
            method: method.to_owned(),
            path: path.to_owned(),
            reason: reason.to_owned(),
            authorization_present: headers.contains_key(header::AUTHORIZATION),
            cookie_present: cookie_value(headers, SESSION_COOKIE_NAME).is_some(),
            host: header_text(headers, header::HOST.as_str()),
            origin: header_text(headers, header::ORIGIN.as_str()),
        };
        match serde_json::to_vec(&row) {
            Ok(encoded) => {
                if let Err(error) = self
                    .db
                    .put_batch_pressure_bypass(cf::CF_KV, [(row_key.as_bytes().to_vec(), encoded)])
                {
                    tracing::error!(
                        code = error_codes::STORAGE_WRITE_FAILED,
                        reason,
                        detail = %error,
                        "dashboard auth failure row write failed"
                    );
                }
            }
            Err(error) => {
                tracing::error!(
                    code = error_codes::STORAGE_WRITE_FAILED,
                    reason,
                    detail = %error,
                    "dashboard auth failure row encode failed"
                );
            }
        }
        tracing::warn!(
            code = "DASHBOARD_AUTH_REJECTED",
            reason,
            method,
            path,
            source_of_truth = SOURCE_OF_TRUTH,
            "dashboard auth request rejected"
        );
    }
}

impl From<&DashboardAuthFailureRow> for DashboardAuthFailurePublic {
    fn from(row: &DashboardAuthFailureRow) -> Self {
        Self {
            row_key: row.row_key.clone(),
            at_unix_ms: row.at_unix_ms,
            method: row.method.clone(),
            path: row.path.clone(),
            reason: row.reason.clone(),
            authorization_present: row.authorization_present,
            cookie_present: row.cookie_present,
        }
    }
}

pub(super) fn session_cookie_header(value: &str, max_age_ms: u64) -> Result<HeaderValue, Response> {
    let max_age_secs = if max_age_ms == 0 {
        0
    } else {
        max_age_ms.div_ceil(1000)
    };
    let header = format!(
        "{SESSION_COOKIE_NAME}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age_secs}"
    );
    HeaderValue::from_str(&header).map_err(|error| {
        tracing::error!(
            code = error_codes::TOOL_INTERNAL_ERROR,
            detail = %error,
            "dashboard session cookie header encode failed"
        );
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            error_codes::TOOL_INTERNAL_ERROR,
            "dashboard session cookie header encode failed",
        )
    })
}

pub(super) fn clear_session_cookie_header() -> HeaderValue {
    HeaderValue::from_static(
        "synapse_dashboard_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
    )
}

pub(super) fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "ok": false,
            "code": code,
            "message": message,
        })),
    )
        .into_response()
}

fn storage_error_response(message: &str, error: synapse_storage::StorageError) -> Response {
    tracing::error!(
        code = error_codes::STORAGE_READ_FAILED,
        detail = %error,
        "dashboard auth storage operation failed"
    );
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        error_codes::STORAGE_READ_FAILED,
        message,
    )
}

fn load_session_ttl() -> Duration {
    match std::env::var(SESSION_TTL_ENV) {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(value) if value > 0 => Duration::from_secs(value),
            _ => {
                tracing::warn!(
                    code = "DASHBOARD_AUTH_TTL_INVALID",
                    env = SESSION_TTL_ENV,
                    value = raw,
                    default_secs = DEFAULT_SESSION_TTL_SECS,
                    "dashboard session TTL env value invalid; using default"
                );
                Duration::from_secs(DEFAULT_SESSION_TTL_SECS)
            }
        },
        Err(_missing) => Duration::from_secs(DEFAULT_SESSION_TTL_SECS),
    }
}

fn session_row_key(session_digest: &str) -> String {
    format!("{SESSION_PREFIX}{session_digest}")
}

fn random_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn digest_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn duration_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let mut pieces = part.trim().splitn(2, '=');
        let key = pieces.next()?.trim();
        let value = pieces.next()?.trim();
        (key == name && !value.is_empty()).then_some(value)
    })
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use synapse_core::SCHEMA_VERSION;
    use synapse_storage::{Db, cf};
    use tempfile::TempDir;

    use super::*;

    fn test_auth(ttl: Duration) -> anyhow::Result<(DashboardAuth, TempDir)> {
        let temp = TempDir::new()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), SCHEMA_VERSION)?);
        let bearer = Arc::new(HttpAuth::from_token("dashboard-secret"));
        let auth = DashboardAuth::with_ttl_for_test(
            db,
            bearer,
            SocketAddr::from(([127, 0, 0, 1], 7700)),
            ttl,
        );
        Ok((auth, temp))
    }

    fn loopback_headers() -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:7700"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:7700"),
        );
        Ok(headers)
    }

    fn login(auth: &DashboardAuth) -> anyhow::Result<DashboardLoginSuccess> {
        let headers = loopback_headers()?;
        auth.login(
            &headers,
            "POST",
            "/dashboard/auth/login",
            "dashboard-secret",
        )
        .map_err(response_error)
    }

    fn cookie_headers(value: &str) -> anyhow::Result<HeaderMap> {
        let mut headers = loopback_headers()?;
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE_NAME}={value}"))?,
        );
        Ok(headers)
    }

    fn response_error(response: Response) -> anyhow::Error {
        anyhow::anyhow!("response status {}", response.status())
    }

    #[test]
    fn login_writes_digest_only_session_row_and_cookie_attrs() -> anyhow::Result<()> {
        let (auth, _temp) = test_auth(Duration::from_secs(300))?;
        let login = login(&auth)?;
        let snapshot = auth.snapshot();
        assert_eq!(snapshot.session_row_count, 1);
        assert_eq!(snapshot.active_session_count, 1);

        let rows = auth
            .db
            .scan_cf_prefix(cf::CF_KV, SESSION_PREFIX.as_bytes())?;
        assert_eq!(rows.len(), 1);
        let row_text = String::from_utf8(rows[0].1.clone())?;
        assert!(!row_text.contains(&login.session_cookie_value));
        assert!(!row_text.contains(&login.csrf_token));

        let cookie =
            session_cookie_header(&login.session_cookie_value, 300_000).map_err(response_error)?;
        let cookie_text = cookie.to_str()?;
        assert!(cookie_text.contains("HttpOnly"));
        assert!(cookie_text.contains("SameSite=Strict"));
        assert!(cookie_text.contains("Path=/"));
        assert!(!cookie_text.contains("Domain="));
        Ok(())
    }

    #[test]
    fn bearer_fallback_authorizes_without_cookie() -> anyhow::Result<()> {
        let (auth, _temp) = test_auth(Duration::from_secs(300))?;
        let mut headers = loopback_headers()?;
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer dashboard-secret"),
        );
        let context = auth
            .authenticate(
                &headers,
                "GET",
                "/dashboard/state.json",
                CsrfPolicy::NotRequired,
            )
            .map_err(response_error)?;
        assert_eq!(context.method, DashboardAuthMethod::Bearer);
        assert!(context.session.is_none());
        Ok(())
    }

    #[test]
    fn mutating_cookie_request_requires_csrf() -> anyhow::Result<()> {
        let (auth, _temp) = test_auth(Duration::from_secs(300))?;
        let login = login(&auth)?;
        let headers = cookie_headers(&login.session_cookie_value)?;
        let response = auth
            .authenticate(
                &headers,
                "POST",
                "/dashboard/local-model-spawn",
                CsrfPolicy::Required,
            )
            .err()
            .ok_or_else(|| anyhow::anyhow!("missing csrf should fail"))?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let snapshot = auth.snapshot();
        assert_eq!(snapshot.failure_count, 1);
        assert_eq!(snapshot.recent_failures[0].reason, "csrf_missing");
        Ok(())
    }

    #[test]
    fn cookie_with_valid_csrf_authorizes_mutation() -> anyhow::Result<()> {
        let (auth, _temp) = test_auth(Duration::from_secs(300))?;
        let login = login(&auth)?;
        let mut headers = cookie_headers(&login.session_cookie_value)?;
        headers.insert("x-csrf-token", HeaderValue::from_str(&login.csrf_token)?);
        let context = auth
            .authenticate(
                &headers,
                "POST",
                "/dashboard/local-model-spawn",
                CsrfPolicy::Required,
            )
            .map_err(response_error)?;
        assert_eq!(context.method, DashboardAuthMethod::Cookie);
        assert!(context.session.is_some());
        Ok(())
    }

    #[test]
    fn expired_cookie_is_rejected_and_queryable() -> anyhow::Result<()> {
        let (auth, _temp) = test_auth(Duration::from_millis(1))?;
        let login = login(&auth)?;
        std::thread::sleep(Duration::from_millis(5));
        let headers = cookie_headers(&login.session_cookie_value)?;
        let response = auth
            .authenticate(
                &headers,
                "GET",
                "/dashboard/state.json",
                CsrfPolicy::NotRequired,
            )
            .err()
            .ok_or_else(|| anyhow::anyhow!("expired cookie should fail"))?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let snapshot = auth.snapshot();
        assert_eq!(snapshot.expired_session_count, 1);
        assert_eq!(snapshot.failure_count, 1);
        assert_eq!(snapshot.recent_failures[0].reason, "session_cookie_expired");
        Ok(())
    }

    #[test]
    fn revoked_session_reuse_is_rejected() -> anyhow::Result<()> {
        let (auth, _temp) = test_auth(Duration::from_secs(300))?;
        let login = login(&auth)?;
        let mut headers = cookie_headers(&login.session_cookie_value)?;
        headers.insert("x-csrf-token", HeaderValue::from_str(&login.csrf_token)?);
        let logout = auth
            .logout(&headers, "POST", "/dashboard/auth/logout")
            .map_err(response_error)?;
        assert!(logout.revoked_row_key.is_some());

        let response = auth
            .authenticate(
                &headers,
                "GET",
                "/dashboard/state.json",
                CsrfPolicy::NotRequired,
            )
            .err()
            .ok_or_else(|| anyhow::anyhow!("revoked cookie should fail"))?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let snapshot = auth.snapshot();
        assert_eq!(snapshot.revoked_session_count, 1);
        assert_eq!(snapshot.recent_failures[0].reason, "session_cookie_revoked");
        Ok(())
    }

    #[test]
    fn wrong_origin_is_rejected_and_recorded() -> anyhow::Result<()> {
        let (auth, _temp) = test_auth(Duration::from_secs(300))?;
        let mut headers = loopback_headers()?;
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://evil.example"),
        );
        let response = auth
            .login(
                &headers,
                "POST",
                "/dashboard/auth/login",
                "dashboard-secret",
            )
            .err()
            .ok_or_else(|| anyhow::anyhow!("wrong origin should fail"))?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let snapshot = auth.snapshot();
        assert_eq!(snapshot.failure_count, 1);
        assert!(snapshot.recent_failures[0].reason.starts_with("origin_"));
        Ok(())
    }
}
