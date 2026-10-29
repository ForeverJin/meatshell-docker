//! Operator web UI for managing saved connections.
//!
//! Served by the same listener as the MCP endpoint, but behind its own login:
//! the operator pastes an access token once, gets an HttpOnly session cookie,
//! and then manages sessions over a small JSON API. Passwords and inline keys
//! are write-only from the browser's point of view — responses only report
//! whether a credential is stored.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    config::{AuthMethod, ConfigStore, Secret, Session},
    ssh,
};

use super::http::Boundary;

const COOKIE: &str = "ms_web";
const COOKIE_TTL: u64 = 12 * 60 * 60;
const PENDING_TTL: Duration = Duration::from_secs(5 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HOST_LEN: usize = 255;
const MAX_TEXT_LEN: usize = 4096;

/// Identity attached by the auth middleware; the pending-host-key store binds
/// confirmations to the subject that saw the fingerprint.
#[derive(Clone)]
struct WebSubject(String);

struct PendingHostKey {
    subject: String,
    host: String,
    port: u16,
    key_line: String,
    created: Instant,
}

#[derive(Default)]
struct WebState {
    /// Session cookie id -> (subject, expires at).
    sessions: Mutex<HashMap<String, (String, Instant)>>,
    /// One-time tokens for host keys shown in the UI.
    pending_host_keys: Mutex<HashMap<String, PendingHostKey>>,
}

fn state() -> &'static WebState {
    static STATE: OnceLock<WebState> = OnceLock::new();
    STATE.get_or_init(WebState::default)
}

pub(crate) fn routes(boundary: std::sync::Arc<Boundary>, enabled: bool) -> Router {
    if !enabled {
        return Router::new();
    }
    let guarded: Router<std::sync::Arc<Boundary>> = Router::new()
        .route("/api/me", get(me))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", put(update_session).delete(delete_session))
        .route("/api/sessions/{id}/test", post(test_saved))
        .route("/api/test", post(test_draft))
        .route("/api/hostkeys/trust", post(trust_host_key))
        .layer(middleware::from_fn_with_state(
            boundary.clone(),
            require_auth,
        ));

    let router: Router<std::sync::Arc<Boundary>> = Router::new()
        .route("/", get(index))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .merge(guarded);
    router.with_state(boundary)
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

async fn login(
    State(boundary): State<std::sync::Arc<Boundary>>,
    Json(body): Json<Value>,
) -> Response {
    let Some(token) = body.get("token").and_then(Value::as_str) else {
        return error(StatusCode::BAD_REQUEST, "missing token");
    };
    let principal = match boundary.oauth().verify(token.trim()) {
        Ok(principal) => principal,
        Err(_) => return error(StatusCode::UNAUTHORIZED, "invalid or expired token"),
    };
    let id = Uuid::new_v4().to_string();
    let now = Instant::now();
    let ttl = token_ttl(&principal);
    if let Ok(mut sessions) = state().sessions.lock() {
        sessions.retain(|_, (_, expires)| *expires > now);
        sessions.insert(id.clone(), (principal.subject.clone(), now + ttl));
    }
    let cookie = format!(
        "{COOKIE}={id}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        ttl.as_secs().min(COOKIE_TTL)
    );
    let mut response = Json(json!({
        "subject": principal.subject,
        "expires_in": ttl.as_secs().min(COOKIE_TTL),
    }))
    .into_response();
    if let Ok(value) = header::HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

async fn logout(headers: HeaderMap) -> Response {
    if let Some(id) = cookie_id(&headers) {
        if let Ok(mut sessions) = state().sessions.lock() {
            sessions.remove(&id);
        }
    }
    let mut response = Json(json!({"ok": true})).into_response();
    if let Ok(value) =
        header::HeaderValue::from_str(&format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"))
    {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

fn token_ttl(principal: &super::oauth::Principal) -> Duration {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let remaining = principal.expires_at.saturating_sub(now).min(COOKIE_TTL);
    Duration::from_secs(remaining.max(60))
}

fn cookie_id(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_string())
}

async fn require_auth(
    State(boundary): State<std::sync::Arc<Boundary>>,
    mut request: Request,
    next: Next,
) -> Response {
    let subject = match cookie_id(request.headers()) {
        Some(id) => {
            let mut found = None;
            if let Ok(mut sessions) = state().sessions.lock() {
                let now = Instant::now();
                sessions.retain(|_, (_, expires)| *expires > now);
                found = sessions.get(&id).map(|(subject, _)| subject.clone());
            }
            match found {
                Some(subject) => subject,
                None => return error(StatusCode::UNAUTHORIZED, "login required"),
            }
        }
        None => {
            // Also accept a bearer token so `curl` and scripts can use the API.
            let token = request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split_once(' '))
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
                .map(|(_, token)| token.to_string());
            match token.as_deref().map(|token| boundary.oauth().verify(token)) {
                Some(Ok(principal)) => principal.subject,
                _ => return error(StatusCode::UNAUTHORIZED, "login required"),
            }
        }
    };

    if request.method() != Method::GET {
        // SameSite=Strict already blocks cross-site cookies; the custom header
        // additionally forces a preflight for any cross-origin scripted write.
        if request.headers().get("x-ms-web").is_none() {
            return error(StatusCode::FORBIDDEN, "missing request header");
        }
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            let host = request
                .headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            let allowed = format!("http://{host}");
            let allowed_tls = format!("https://{host}");
            match origin.to_str() {
                Ok(origin) if origin == allowed || origin == allowed_tls => {}
                _ => return error(StatusCode::FORBIDDEN, "origin is not allowed"),
            }
        }
    }

    request.extensions_mut().insert(WebSubject(subject));
    next.run(request).await
}

// ---------------------------------------------------------------------------
// Pages and meta
// ---------------------------------------------------------------------------

async fn index() -> Response {
    let mut response = Html(include_str!("webui.html")).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline';              img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn me(
    Extension(WebSubject(subject)): Extension<WebSubject>,
) -> Response {
    let count = ConfigStore::load().map(|store| store.sessions().len()).unwrap_or(0);
    Json(json!({
        "subject": subject,
        "version": env!("CARGO_PKG_VERSION"),
        "data_dir": crate::config::data_dir(),
        "session_count": count,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Session CRUD
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionInput {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    private_key_path: Option<String>,
    #[serde(default)]
    private_key_inline: Option<String>,
    #[serde(default)]
    proxy: Option<String>,
    #[serde(default)]
    jump_session_id: Option<String>,
    #[serde(default)]
    mcp_access: Option<bool>,
    #[serde(default)]
    clear_password: bool,
    #[serde(default)]
    clear_inline_key: bool,
}

fn session_dto(session: &Session) -> Value {
    json!({
        "id": session.id,
        "name": session.name,
        "group": session.group,
        "kind": session.kind.as_str(),
        "host": session.host,
        "port": session.port,
        "user": session.user,
        "auth": session.auth.as_str(),
        "has_password": !session.password.is_empty(),
        "has_inline_key": !session.private_key_inline.is_empty(),
        "private_key_path": session.private_key_path,
        "has_proxy": !session.proxy.trim().is_empty(),
        "jump_session_id": session.jump_session_id,
        "mcp_access": session.mcp_access,
    })
}

async fn list_sessions() -> Response {
    match ConfigStore::load() {
        Ok(store) => Json(json!({
            "sessions": store.sessions().iter().map(session_dto).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    }
}

async fn create_session(Json(input): Json<SessionInput>) -> Response {
    let mut store = match ConfigStore::load() {
        Ok(store) => store,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    };
    let mut session = Session::new_empty();
    session.id = Uuid::new_v4().to_string();
    match apply_input(&mut session, input, true) {
        Ok(()) => {}
        Err(message) => return error(StatusCode::BAD_REQUEST, &message),
    }
    store.upsert(session.clone());
    if let Err(e) = store.save() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}"));
    }
    Json(session_dto(&session)).into_response()
}

async fn update_session(Path(id): Path<String>, Json(input): Json<SessionInput>) -> Response {
    let mut store = match ConfigStore::load() {
        Ok(store) => store,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    };
    let Some(mut session) = store.sessions().iter().find(|s| s.id == id).cloned() else {
        return error(StatusCode::NOT_FOUND, "session not found");
    };
    match apply_input(&mut session, input, false) {
        Ok(()) => {}
        Err(message) => return error(StatusCode::BAD_REQUEST, &message),
    }
    store.upsert(session.clone());
    if let Err(e) = store.save() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}"));
    }
    Json(session_dto(&session)).into_response()
}

async fn delete_session(Path(id): Path<String>) -> Response {
    let mut store = match ConfigStore::load() {
        Ok(store) => store,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    };
    if !store.sessions().iter().any(|s| s.id == id) {
        return error(StatusCode::NOT_FOUND, "session not found");
    }
    store.remove(&id);
    if let Err(e) = store.save() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}"));
    }
    Json(json!({"ok": true})).into_response()
}

/// Merge request fields into a session. Secrets are only touched when the
/// request actually carries them, so a rename can never wipe a password.
fn apply_input(session: &mut Session, input: SessionInput, creating: bool) -> Result<(), String> {
    if let Some(host) = input.host {
        let host = host.trim().to_string();
        if host.is_empty() || host.len() > MAX_HOST_LEN || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("invalid host".into());
        }
        session.host = host;
    } else if creating {
        return Err("host is required".into());
    }

    if let Some(port) = input.port {
        if port == 0 {
            return Err("invalid port".into());
        }
        session.port = port;
    } else if creating {
        session.port = 22;
    }

    if let Some(user) = input.user {
        let user = user.trim().to_string();
        if user.len() > MAX_HOST_LEN || user.chars().any(|c| c.is_control()) {
            return Err("invalid user".into());
        }
        session.user = user;
    }

    if let Some(auth) = input.auth {
        session.auth = match auth.as_str() {
            "password" => AuthMethod::Password,
            "keyboard-interactive" => AuthMethod::KeyboardInteractive,
            "key" => AuthMethod::Key,
            _ => return Err("invalid auth method".into()),
        };
    }

    if let Some(name) = input.name {
        let name = name.trim();
        session.name = if name.is_empty() {
            session.host.clone()
        } else {
            clamp(name)
        };
    }
    if let Some(group) = input.group {
        session.group = clamp(group.trim());
    }
    if let Some(proxy) = input.proxy {
        session.proxy = clamp(proxy.trim());
    }
    if let Some(jump) = input.jump_session_id {
        session.jump_session_id = clamp(jump.trim());
    }
    if let Some(access) = input.mcp_access {
        session.mcp_access = access;
    }
    if input.clear_password {
        session.password = Secret::default();
    }
    if let Some(password) = input.password {
        if !password.is_empty() {
            session.password = Secret::new(password);
        }
    }
    if input.clear_inline_key {
        session.private_key_inline = Secret::default();
    }
    if let Some(key) = input.private_key_inline {
        if !key.is_empty() {
            session.private_key_inline = Secret::new(key);
        }
    }
    if let Some(path) = input.private_key_path {
        session.private_key_path = clamp(path.trim());
    }

    if creating && session.name.is_empty() {
        session.name = session.host.clone();
    }
    match session.auth {
        AuthMethod::Password | AuthMethod::KeyboardInteractive => {
            if session.password.is_empty() && creating {
                return Err("password is required for password authentication".into());
            }
        }
        AuthMethod::Key => {
            if session.private_key_inline.is_empty() && session.private_key_path.trim().is_empty() {
                return Err("a private key path or inline key is required".into());
            }
        }
    }
    Ok(())
}

fn clamp(value: &str) -> String {
    value.chars().take(MAX_TEXT_LEN).collect()
}

// ---------------------------------------------------------------------------
// Connection probe
// ---------------------------------------------------------------------------

async fn test_saved(
    Extension(WebSubject(subject)): Extension<WebSubject>,
    Path(id): Path<String>,
) -> Response {
    let (session, jump) = match ConfigStore::load() {
        Ok(store) => {
            let Some(session) = store.sessions().iter().find(|s| s.id == id).cloned() else {
                return error(StatusCode::NOT_FOUND, "session not found");
            };
            let jump = match store.resolve_jump_chain(&session) {
                Ok(jump) => jump,
                Err(e) => return error(StatusCode::BAD_REQUEST, &format!("{e:#}")),
            };
            (session, jump)
        }
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    };
    probe(session, jump, subject).await
}

async fn test_draft(
    Extension(WebSubject(subject)): Extension<WebSubject>,
    Json(input): Json<SessionInput>,
) -> Response {
    let mut session = Session::new_empty();
    session.id = "draft".into();
    if let Err(message) = apply_input(&mut session, input, true) {
        return error(StatusCode::BAD_REQUEST, &message);
    }
    if session.kind.as_str() != "ssh" {
        return error(StatusCode::BAD_REQUEST, "only SSH sessions can be tested");
    }
    let jump = match ConfigStore::load().and_then(|store| store.resolve_jump_chain(&session)) {
        Ok(jump) => jump,
        Err(e) => return error(StatusCode::BAD_REQUEST, &format!("{e:#}")),
    };
    probe(session, jump, subject).await
}

async fn probe(session: Session, jump: Vec<Session>, subject: String) -> Response {
    match ssh::probe_connection(session.clone(), jump, PROBE_TIMEOUT).await {
        Ok(ssh::ProbeOutcome::Ok) => Json(json!({"status": "ok"})).into_response(),
        Ok(ssh::ProbeOutcome::HostKey {
            host,
            port,
            key_type,
            fingerprint,
            key_line,
            changed,
        }) => {
            let token = Uuid::new_v4().to_string();
            if let Ok(mut pending) = state().pending_host_keys.lock() {
                let now = Instant::now();
                pending.retain(|_, entry| now.duration_since(entry.created) < PENDING_TTL);
                pending.insert(
                    token.clone(),
                    PendingHostKey {
                        subject,
                        host: host.clone(),
                        port,
                        key_line,
                        created: now,
                    },
                );
            }
            Json(json!({
                "status": "host_key_unknown",
                "token": token,
                "host": host,
                "port": port,
                "key_type": key_type,
                "fingerprint": fingerprint,
                "changed": changed,
            }))
            .into_response()
        }
        Ok(ssh::ProbeOutcome::MissingCredentials {
            need_user,
            need_password,
        }) => Json(json!({
            "status": "missing_credentials",
            "need_user": need_user,
            "need_password": need_password,
        }))
        .into_response(),
        Ok(ssh::ProbeOutcome::MfaRequired { prompt }) => {
            Json(json!({"status": "mfa_required", "prompt": prompt})).into_response()
        }
        Err(e) => Json(json!({"status": "error", "message": format!("{e:#}")})).into_response(),
    }
}

async fn trust_host_key(
    Extension(WebSubject(subject)): Extension<WebSubject>,
    Json(body): Json<Value>,
) -> Response {
    let Some(token) = body.get("token").and_then(Value::as_str) else {
        return error(StatusCode::BAD_REQUEST, "missing token");
    };
    let pending = {
        let mut pending = match state().pending_host_keys.lock() {
            Ok(pending) => pending,
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "state unavailable"),
        };
        let Some(entry) = pending.remove(token) else {
            return error(StatusCode::NOT_FOUND, "confirmation expired; run the test again");
        };
        if entry.created.elapsed() > PENDING_TTL {
            return error(StatusCode::NOT_FOUND, "confirmation expired; run the test again");
        }
        if entry.subject != subject {
            return error(StatusCode::FORBIDDEN, "confirmation belongs to another subject");
        }
        entry
    };
    if let Err(e) = ssh::known_hosts::remember_line(&pending.host, pending.port, &pending.key_line) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}"));
    }
    Json(json!({"ok": true, "host": pending.host, "port": pending.port})).into_response()
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}
