//! The phone remote's server: the phone page and a token-guarded API, served
//! from this GUI process when `remote.enabled` is on. Reads are always on;
//! acting on a session needs `remote.actions` too. Decisions
//! (token check, status, bind rules) are `clash::application::remote`; the
//! design and the security model are `docs/remote.md`.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use axum::extract::{Path as AxPath, State as AxState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use clash::application::remote as core;
use clash::infrastructure::config::Config;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::GuiState;

/// When this process started serving the GUI (epoch ms).
static STARTED_AT: OnceLock<i64> = OnceLock::new();
/// The server currently listening, if any.
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
/// Why the last start failed, shown in Settings.
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// The pairing token every `/api/*` call must present. Empty = nobody.
static TOKEN: Mutex<String> = Mutex::new(String::new());

struct Running {
    addr: SocketAddr,
    stop: tokio::sync::oneshot::Sender<()>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Record the process start; called once from `setup`.
pub(crate) fn mark_started() {
    let _ = STARTED_AT.set(now_ms());
}

fn token_path() -> PathBuf {
    Config::clash_data_dir().join("remote-token")
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname NUL-terminates
    // on success (and we cap at the buffer either way).
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return "this Mac".to_string();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// The pairing token, created on first use: 32 bytes from the OS RNG, written
/// 0600 through a temp file and a rename so a reader never sees half of it.
fn load_or_create_token(rotate: bool) -> std::io::Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = token_path();
    if !rotate {
        if let Ok(s) = std::fs::read_to_string(&path) {
            let s = s.trim().to_string();
            if core::valid_token(&s) {
                return Ok(s);
            }
        }
    }
    let mut bytes = [0u8; core::TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token = core::encode_token(&bytes);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(token.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, &path)?;
    Ok(token)
}

fn stop_running() {
    if let Some(r) = RUNNING.lock().unwrap().take() {
        let _ = r.stop.send(());
    }
}

/// Make the server match the settings: start it, stop it, or move it to a new
/// address. Safe to call from any thread, any number of times.
pub(crate) fn apply(app: &AppHandle) {
    apply_with(app, false);
}

fn apply_with(app: &AppHandle, rotate: bool) {
    let cfg = app.state::<GuiState>().config.get().remote;
    *LAST_ERROR.lock().unwrap() = None;
    if !cfg.enabled {
        stop_running();
        return;
    }
    let addr = match core::parse_bind(&cfg.bind) {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    // No token, no server: an API nobody can authenticate to is useless on
    // loopback and must not exist anywhere else.
    match load_or_create_token(rotate) {
        Ok(t) => *TOKEN.lock().unwrap() = t,
        Err(e) => {
            stop_running();
            return fail(format!("could not create the pairing token: {e}"));
        }
    }
    if RUNNING
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|r| r.addr == addr)
    {
        return;
    }
    stop_running();
    // Bound synchronously so a taken port is reported now, not lost in a task.
    let listener = match std::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => return fail(format!("could not listen on {addr}: {e}")),
    };
    if let Err(e) = listener.set_nonblocking(true) {
        return fail(format!("could not listen on {addr}: {e}"));
    }
    let (stop, stopped) = tokio::sync::oneshot::channel();
    *RUNNING.lock().unwrap() = Some(Running { addr, stop });
    let router = router(gui_source(app.clone()));
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => return fail(format!("could not listen on {addr}: {e}")),
        };
        tracing::info!("remote: serving the phone page on {addr}");
        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
        if let Err(e) = served {
            fail(format!("remote server stopped: {e}"));
        }
        tracing::info!("remote: stopped listening on {addr}");
    });
}

fn fail(msg: String) {
    tracing::warn!("remote: {msg}");
    *LAST_ERROR.lock().unwrap() = Some(msg);
}

// ── HTTP ────────────────────────────────────────────────────────────────

const PAGE: &str = include_str!("../../dist/remote/index.html");
const PAGE_JS: &str = include_str!("../../dist/remote/remote.js");
const MODEL_JS: &str = include_str!("../../dist/remote-model.js");
const ATTENTION_JS: &str = include_str!("../../dist/attention.js");

type Fut<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
type Act<A, T> = std::sync::Arc<dyn Fn(String, A) -> Fut<Result<T, String>> + Send + Sync>;

/// What the API serves and does — the GUI's caches and commands in the app,
/// fixed values in tests.
#[derive(Clone)]
struct Source {
    status: std::sync::Arc<dyn Fn() -> core::InstanceStatus + Send + Sync>,
    state: std::sync::Arc<dyn Fn() -> serde_json::Value + Send + Sync>,
    /// `remote.actions`, read on every call so a change applies at once.
    actions: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
    /// Whether the GUI's session list holds this id (a phone acts only on a
    /// session it was shown).
    known: std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>,
    screen: Act<(), Vec<String>>,
    keys: Act<&'static [u8], ()>,
    followup: Act<String, usize>,
    stash: Act<(), ()>,
}

/// The GUI's own caches: what the frontend was last given, never rebuilt here,
/// so a poll cannot run the session merge or drain the prompt queue. Actions
/// go through the same paths the desktop uses: the control client's input
/// (as queued follow-ups are delivered), the prompt queue, `stash_session`.
fn gui_source(app: AppHandle) -> Source {
    let for_state = app.clone();
    let for_actions = app.clone();
    let for_known = app.clone();
    let for_screen = app.clone();
    let for_keys = app.clone();
    let for_followup = app.clone();
    let for_stash = app.clone();
    Source {
        actions: std::sync::Arc::new(move || {
            for_actions.state::<GuiState>().config.get().remote.actions
        }),
        known: std::sync::Arc::new(move |id| {
            for_known
                .state::<GuiState>()
                .previous
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.id == id)
        }),
        screen: std::sync::Arc::new(move |id, ()| {
            let app = for_screen.clone();
            Box::pin(async move {
                let state = app.state::<GuiState>();
                let mut control = state.control.lock().await;
                crate::ensure_connected(&mut control).await;
                control.screen(&id).await.map_err(|e| e.to_string())
            })
        }),
        keys: std::sync::Arc::new(move |id, bytes| {
            let app = for_keys.clone();
            Box::pin(async move {
                let state = app.state::<GuiState>();
                let mut control = state.control.lock().await;
                crate::ensure_connected(&mut control).await;
                control
                    .send_input(&id, bytes)
                    .await
                    .map_err(|e| e.to_string())
            })
        }),
        followup: std::sync::Arc::new(move |id, text| {
            let app = for_followup.clone();
            Box::pin(async move { crate::queue_prompt(app.state::<GuiState>(), id, text) })
        }),
        stash: std::sync::Arc::new(move |id, ()| {
            let app = for_stash.clone();
            Box::pin(async move { crate::stash_session(app.state::<GuiState>(), id, None).await })
        }),
        status: std::sync::Arc::new(move || status_now(&app)),
        state: std::sync::Arc::new(move || {
            let state = for_state.state::<GuiState>();
            serde_json::json!({
                "status": status_now(&for_state),
                "sessions": *state.previous.lock().unwrap(),
                "workflows": *state.last_workflows.lock().unwrap(),
            })
        }),
    }
}

fn router(source: Source) -> Router {
    Router::new()
        .route("/", get(|| async { redirect_to_page() }))
        .route("/remote", get(|| async { redirect_to_page() }))
        .route(
            "/remote/",
            get(|| async { page(PAGE, "text/html; charset=utf-8") }),
        )
        .route("/remote/remote.js", get(|| async { page(PAGE_JS, JS) }))
        .route(
            "/remote/remote-model.js",
            get(|| async { page(MODEL_JS, JS) }),
        )
        .route(
            "/remote/attention.js",
            get(|| async { page(ATTENTION_JS, JS) }),
        )
        .route("/api/status", get(api_status).options(preflight))
        .route("/api/state", get(api_state).options(preflight))
        .route(
            "/api/sessions/{id}/screen",
            get(api_screen).options(preflight),
        )
        .route("/api/sessions/{id}/keys", post(api_keys).options(preflight))
        .route(
            "/api/sessions/{id}/followup",
            post(api_followup).options(preflight),
        )
        .route(
            "/api/sessions/{id}/stash",
            post(api_stash).options(preflight),
        )
        .with_state(source)
}

const JS: &str = "text/javascript; charset=utf-8";

fn redirect_to_page() -> Response {
    (StatusCode::FOUND, [(header::LOCATION, "/remote/")]).into_response()
}

fn page(body: &'static str, kind: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "no-cache"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                 connect-src *; img-src 'self' data:; frame-ancestors 'none'",
            ),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        body,
    )
        .into_response()
}

/// A paired page polls every machine it knows, so the API answers any origin:
/// the bearer token — never a cookie — is what authorizes a call.
fn cors(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        "access-control-allow-private-network",
        HeaderValue::from_static("true"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn preflight() -> Response {
    cors(StatusCode::NO_CONTENT.into_response())
}

fn authorized(headers: &HeaderMap) -> bool {
    let presented = core::bearer(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    )
    .unwrap_or("");
    core::token_matches(&TOKEN.lock().unwrap(), presented)
}

fn unauthorized() -> Response {
    cors((StatusCode::UNAUTHORIZED, "pairing token required").into_response())
}

fn status_now(app: &AppHandle) -> core::InstanceStatus {
    let state = app.state::<GuiState>();
    let sessions = state.previous.lock().unwrap().clone();
    let items = state.last_workflows.lock().unwrap().clone();
    let host = hostname();
    core::instance_status(
        core::InstanceFacts {
            hostname: &host,
            version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
            started_at: STARTED_AT.get().copied().unwrap_or(0),
            now: now_ms(),
        },
        &sessions,
        &items,
    )
}

async fn api_status(AxState(src): AxState<Source>, headers: HeaderMap) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    cors(axum::Json((src.status)()).into_response())
}

/// What the phone page renders: this instance's status plus the session and
/// workflow lists the GUI last built.
async fn api_state(AxState(src): AxState<Source>, headers: HeaderMap) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    cors(axum::Json((src.state)()).into_response())
}

/// Why an action route refused, or `None` to go ahead: the token first, then
/// `remote.actions`, then a session the GUI actually lists.
fn refuse(src: &Source, headers: &HeaderMap, id: &str) -> Option<Response> {
    if !authorized(headers) {
        return Some(unauthorized());
    }
    if !(src.actions)() {
        return Some(cors(
            (
                StatusCode::FORBIDDEN,
                "actions are off on this clash (Settings → Remote → Allow actions)",
            )
                .into_response(),
        ));
    }
    if !(src.known)(id) {
        return Some(cors(
            (StatusCode::NOT_FOUND, "no such session").into_response(),
        ));
    }
    None
}

fn failed(e: String) -> Response {
    cors((StatusCode::BAD_GATEWAY, e).into_response())
}

/// A session's visible screen, and the numbered prompt on it if any.
async fn api_screen(
    AxState(src): AxState<Source>,
    AxPath(id): AxPath<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = refuse(&src, &headers, &id) {
        return r;
    }
    match (src.screen)(id, ()).await {
        Ok(lines) => {
            let prompt = core::screen_prompt(&lines);
            cors(
                axum::Json(serde_json::json!({ "lines": lines, "prompt": prompt })).into_response(),
            )
        }
        Err(e) => failed(e),
    }
}

#[derive(serde::Deserialize)]
struct KeyBody {
    key: String,
}

/// Press one named key (`core::key_bytes` — never raw bytes).
async fn api_keys(
    AxState(src): AxState<Source>,
    AxPath(id): AxPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<KeyBody>,
) -> Response {
    if let Some(r) = refuse(&src, &headers, &id) {
        return r;
    }
    let Some(bytes) = core::key_bytes(&body.key) else {
        return cors((StatusCode::BAD_REQUEST, "unknown key").into_response());
    };
    match (src.keys)(id, bytes).await {
        Ok(()) => cors(StatusCode::NO_CONTENT.into_response()),
        Err(e) => failed(e),
    }
}

#[derive(serde::Deserialize)]
struct FollowupBody {
    text: String,
}

/// Queue a follow-up, delivered when the session is next idle at its prompt.
async fn api_followup(
    AxState(src): AxState<Source>,
    AxPath(id): AxPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<FollowupBody>,
) -> Response {
    if let Some(r) = refuse(&src, &headers, &id) {
        return r;
    }
    match (src.followup)(id, body.text).await {
        Ok(queued) => cors(axum::Json(serde_json::json!({ "queued": queued })).into_response()),
        Err(e) => cors((StatusCode::BAD_REQUEST, e).into_response()),
    }
}

/// Stash the session: stop it, keep it resumable.
async fn api_stash(
    AxState(src): AxState<Source>,
    AxPath(id): AxPath<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = refuse(&src, &headers, &id) {
        return r;
    }
    match (src.stash)(id, ()).await {
        Ok(()) => cors(StatusCode::NO_CONTENT.into_response()),
        Err(e) => failed(e),
    }
}

// ── Settings commands ───────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteInfo {
    enabled: bool,
    bind: String,
    actions: bool,
    /// The address actually listening, when the server runs.
    listening: Option<String>,
    error: Option<String>,
    /// The link that pairs a phone — carries the token in its fragment.
    pairing_url: Option<String>,
}

fn info(app: &AppHandle) -> RemoteInfo {
    let cfg = app.state::<GuiState>().config.get().remote;
    let listening = RUNNING.lock().unwrap().as_ref().map(|r| r.addr);
    let token = TOKEN.lock().unwrap().clone();
    RemoteInfo {
        enabled: cfg.enabled,
        bind: cfg.bind,
        actions: cfg.actions,
        listening: listening.map(|a| a.to_string()),
        error: LAST_ERROR.lock().unwrap().clone(),
        pairing_url: listening
            .filter(|_| !token.is_empty())
            .map(|a| core::pairing_url(&a, &hostname(), &token)),
    }
}

#[tauri::command]
pub(crate) fn get_remote_info(app: AppHandle) -> RemoteInfo {
    info(&app)
}

/// Patch `remote.enabled` / `remote.bind` / `remote.actions` in config.toml
/// and apply at once.
#[tauri::command]
pub(crate) fn set_remote_settings(
    app: AppHandle,
    state: State<'_, GuiState>,
    enabled: Option<bool>,
    bind: Option<String>,
    actions: Option<bool>,
) -> Result<RemoteInfo, String> {
    let mut patch = Vec::new();
    if let Some(a) = actions {
        patch.push(("remote.actions", serde_json::Value::Bool(a)));
    }
    if let Some(b) = bind {
        let b = b.trim().to_string();
        core::parse_bind(&b)?;
        patch.push(("remote.bind", serde_json::Value::String(b)));
    }
    if let Some(e) = enabled {
        patch.push(("remote.enabled", serde_json::Value::Bool(e)));
    }
    state.config.set_json(&patch).map_err(|e| e.to_string())?;
    apply(&app);
    Ok(info(&app))
}

/// Replace the pairing token: every paired phone has to pair again.
#[tauri::command]
pub(crate) fn rotate_remote_token(app: AppHandle) -> RemoteInfo {
    if app.state::<GuiState>().config.get().remote.enabled {
        stop_running();
        apply_with(&app, true);
    } else {
        let _ = std::fs::remove_file(token_path());
        TOKEN.lock().unwrap().clear();
    }
    info(&app)
}

// ── Instances (the Machines view) ───────────────────────────────────────

fn peers_path() -> PathBuf {
    Config::clash_data_dir().join("remote-peers.json")
}

fn load_peers() -> Vec<core::RemotePeer> {
    std::fs::read_to_string(peers_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Written 0600 like the token: it holds other machines' tokens.
fn save_peers(peers: &[core::RemotePeer]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = peers_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let body = serde_json::to_vec_pretty(peers).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(&body).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// A paired instance as the webview sees it: no token.
#[derive(Serialize)]
pub(crate) struct PeerRow {
    name: String,
    url: String,
}

fn peer_rows(peers: &[core::RemotePeer]) -> Vec<PeerRow> {
    peers
        .iter()
        .map(|p| PeerRow {
            name: p.name.clone(),
            url: p.url.clone(),
        })
        .collect()
}

/// This instance's status, built from the GUI's own state — no HTTP, no token.
#[tauri::command]
pub(crate) fn local_instance_status(app: AppHandle) -> core::InstanceStatus {
    status_now(&app)
}

#[tauri::command]
pub(crate) async fn list_remote_peers() -> Vec<PeerRow> {
    tauri::async_runtime::spawn_blocking(|| peer_rows(&load_peers()))
        .await
        .unwrap_or_default()
}

/// Pair with another instance from its pairing link.
#[tauri::command]
pub(crate) async fn add_remote_peer(
    link: String,
    name: Option<String>,
) -> Result<Vec<PeerRow>, String> {
    let (url, token) = core::parse_pairing_link(&link)
        .ok_or("That is not a clash pairing link (it ends in #token=…).")?;
    tauri::async_runtime::spawn_blocking(move || {
        let peers = core::add_peer(&load_peers(), &url, &token, name.as_deref());
        save_peers(&peers)?;
        Ok(peer_rows(&peers))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn remove_remote_peer(url: String) -> Result<Vec<PeerRow>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let peers: Vec<_> = load_peers().into_iter().filter(|p| p.url != url).collect();
        save_peers(&peers)?;
        Ok(peer_rows(&peers))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One poll of a paired instance — the shape `recordPoll` in remote-model.js
/// folds: `{ ok, status }`, `{ unauthorized }` or `{ error }`.
#[derive(Serialize, Default)]
pub(crate) struct PeerPoll {
    ok: bool,
    status: Option<serde_json::Value>,
    unauthorized: bool,
    error: Option<String>,
}

#[tauri::command]
pub(crate) async fn poll_remote_peer(url: String) -> PeerPoll {
    let peer = tauri::async_runtime::spawn_blocking(move || {
        load_peers().into_iter().find(|p| p.url == url)
    })
    .await
    .ok()
    .flatten();
    let Some(peer) = peer else {
        return PeerPoll {
            error: Some("not paired".into()),
            ..Default::default()
        };
    };
    match http_get(&peer.url, "/api/status", &peer.token).await {
        Ok((200, body)) => match serde_json::from_slice(&body) {
            Ok(status) => PeerPoll {
                ok: true,
                status: Some(status),
                ..Default::default()
            },
            Err(_) => PeerPoll {
                error: Some("not a clash answer".into()),
                ..Default::default()
            },
        },
        Ok((401, _)) => PeerPoll {
            unauthorized: true,
            ..Default::default()
        },
        Ok((code, _)) => PeerPoll {
            error: Some(format!("HTTP {code}")),
            ..Default::default()
        },
        Err(e) => PeerPoll {
            error: Some(e),
            ..Default::default()
        },
    }
}

/// A plain-HTTP GET with a bearer token, bounded in time and size. HTTPS is
/// not spoken here: peers are reached over the tailnet, which encrypts.
async fn http_get(base: &str, path: &str, token: &str) -> Result<(u16, Vec<u8>), String> {
    use http_body_util::{BodyExt, Empty, Limited};
    use hyper::body::Bytes;
    const WAIT: std::time::Duration = std::time::Duration::from_secs(4);

    let authority = base
        .strip_prefix("http://")
        .ok_or("only http:// instances can be polled")?
        .trim_end_matches('/');
    let addr = if authority
        .rsplit_once(':')
        .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
        && !authority.ends_with(']')
    {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    let stream = tokio::time::timeout(WAIT, tokio::net::TcpStream::connect(addr))
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|_| "no answer".to_string())?;
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(|e| e.to_string())?;
    // On the caller's runtime: the stream is registered with its reactor.
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = hyper::Request::get(path)
        .header(hyper::header::HOST, authority)
        .header(hyper::header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Empty::<Bytes>::new())
        .map_err(|e| e.to_string())?;
    let res = tokio::time::timeout(WAIT, sender.send_request(req))
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let code = res.status().as_u16();
    let body = tokio::time::timeout(WAIT, Limited::new(res.into_body(), 8 << 20).collect())
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?
        .to_bytes()
        .to_vec();
    Ok((code, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server and the peer client against each other over a real socket:
    /// the token is required on every API route and the page is public.
    #[tokio::test]
    async fn the_api_answers_only_the_paired_token() {
        let token = core::encode_token(&[7; core::TOKEN_BYTES]);
        *TOKEN.lock().unwrap() = token.clone();
        let source = Source {
            status: std::sync::Arc::new(|| {
                core::instance_status(
                    core::InstanceFacts {
                        hostname: "test-mac",
                        version: "9.9.9",
                        pid: 1,
                        started_at: 1,
                        now: 2,
                    },
                    &[],
                    &[],
                )
            }),
            state: std::sync::Arc::new(|| serde_json::json!({ "sessions": [], "workflows": [] })),
            ..test_actions(false)
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(source)).await;
        });

        let (code, body) = http_get(&base, "/api/status", &token).await.unwrap();
        assert_eq!(code, 200);
        let st: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(st["app"], "clash");
        assert_eq!(st["hostname"], "test-mac");
        assert_eq!(st["needsYou"], 0);

        let wrong = core::encode_token(&[8; core::TOKEN_BYTES]);
        assert_eq!(http_get(&base, "/api/status", &wrong).await.unwrap().0, 401);
        assert_eq!(http_get(&base, "/api/state", "").await.unwrap().0, 401);
        let (code, body) = http_get(&base, "/api/state", &token).await.unwrap();
        assert_eq!(code, 200);
        assert!(String::from_utf8_lossy(&body).contains("\"workflows\""));

        let (code, page) = http_get(&base, "/remote/", "").await.unwrap();
        assert_eq!(code, 200);
        assert!(String::from_utf8_lossy(&page).contains("src=\"remote.js\""));
        assert_eq!(
            http_get(&base, "/remote/remote.js", "").await.unwrap().0,
            200
        );
        assert_eq!(
            http_get(&base, "/remote/attention.js", "").await.unwrap().0,
            200
        );
        assert_eq!(http_get(&base, "/remote", "").await.unwrap().0, 302);
        assert_eq!(http_get(&base, "/nope", "").await.unwrap().0, 404);
        // Nothing listens there: an error, not a hang.
        assert!(http_get("http://127.0.0.1:1", "/api/status", &token)
            .await
            .is_err());
        assert!(http_get("https://h", "/api/status", &token).await.is_err());
    }

    /// Actions that record what they were asked, for a session called "s1".
    fn test_actions(on: bool) -> Source {
        use std::sync::{Arc, Mutex};
        static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
        Source {
            status: Arc::new(|| {
                core::instance_status(
                    core::InstanceFacts {
                        hostname: "h",
                        version: "0",
                        pid: 1,
                        started_at: 1,
                        now: 2,
                    },
                    &[],
                    &[],
                )
            }),
            state: Arc::new(|| serde_json::json!({})),
            actions: Arc::new(move || on),
            known: Arc::new(|id| id == "s1"),
            screen: Arc::new(|_, ()| {
                Box::pin(async {
                    Ok(vec![
                        "Do you want to proceed?".to_string(),
                        "❯ 1. Yes".to_string(),
                        "  2. No".to_string(),
                    ])
                })
            }),
            keys: Arc::new(|id, bytes| {
                LOG.lock().unwrap().push(format!("keys {id} {bytes:?}"));
                Box::pin(async { Ok(()) })
            }),
            followup: Arc::new(|id, text| {
                LOG.lock().unwrap().push(format!("followup {id} {text}"));
                Box::pin(async { Ok(1) })
            }),
            stash: Arc::new(|id, ()| {
                LOG.lock().unwrap().push(format!("stash {id}"));
                Box::pin(async { Ok(()) })
            }),
        }
    }

    /// One raw HTTP/1.1 request; returns the status code and the body.
    async fn raw(base: &str, method: &str, path: &str, token: &str, body: &str) -> (u16, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = base.strip_prefix("http://").unwrap();
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        let code = out[9..12].parse().unwrap();
        let body = out
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (code, body)
    }

    async fn serve(source: Source) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(source)).await;
        });
        base
    }

    /// Acting needs the token, then `remote.actions`, then a session the GUI
    /// lists; and only named keys reach the session.
    #[tokio::test]
    async fn actions_need_the_token_the_setting_and_a_listed_session() {
        // The same token as the other server test: TOKEN is process-wide and
        // the tests run in parallel.
        let token = core::encode_token(&[7; core::TOKEN_BYTES]);
        *TOKEN.lock().unwrap() = token.clone();
        let off = serve(test_actions(false)).await;
        assert_eq!(
            raw(&off, "GET", "/api/sessions/s1/screen", &token, "")
                .await
                .0,
            403
        );
        assert_eq!(
            raw(
                &off,
                "POST",
                "/api/sessions/s1/keys",
                &token,
                r#"{"key":"1"}"#
            )
            .await
            .0,
            403
        );

        let on = serve(test_actions(true)).await;
        assert_eq!(
            raw(&on, "GET", "/api/sessions/s1/screen", "nope", "")
                .await
                .0,
            401
        );
        assert_eq!(
            raw(&on, "GET", "/api/sessions/zz/screen", &token, "")
                .await
                .0,
            404
        );
        let (code, body) = raw(&on, "GET", "/api/sessions/s1/screen", &token, "").await;
        assert_eq!(code, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["prompt"]["question"], "Do you want to proceed?");
        assert_eq!(v["prompt"]["choices"][1]["key"], "2");

        assert_eq!(
            raw(
                &on,
                "POST",
                "/api/sessions/s1/keys",
                &token,
                r#"{"key":"rm -rf"}"#
            )
            .await
            .0,
            400
        );
        assert_eq!(
            raw(
                &on,
                "POST",
                "/api/sessions/s1/keys",
                &token,
                r#"{"key":"1"}"#
            )
            .await
            .0,
            204
        );
        let (code, body) = raw(
            &on,
            "POST",
            "/api/sessions/s1/followup",
            &token,
            r#"{"text":"go on"}"#,
        )
        .await;
        assert_eq!((code, body.contains("\"queued\":1")), (200, true));
        assert_eq!(
            raw(&on, "POST", "/api/sessions/s1/stash", &token, "")
                .await
                .0,
            204
        );
        // The preflight lets a page on another instance POST with a JSON body.
        let (code, _) = raw(&on, "OPTIONS", "/api/sessions/s1/keys", "", "").await;
        assert_eq!(code, 204);
    }

    /// The API reads the GUI's caches; rebuilding a list here would run the
    /// session merge or drain the prompt queue from a phone poll.
    #[test]
    fn the_api_never_rebuilds_the_lists_it_serves() {
        const SRC: &str = include_str!("remote.rs");
        let http = SRC
            .split_once("// ── HTTP")
            .unwrap()
            .1
            .split_once("// ── Settings commands")
            .unwrap()
            .0;
        for forbidden in [
            "list_sessions(",
            "list_workflow_items(",
            "load_workflow_items(",
        ] {
            assert!(!http.contains(forbidden), "{forbidden}");
        }
        // Every API route checks the token first; every action route goes
        // through `refuse` (token, then remote.actions, then a listed session).
        for handler in ["async fn api_status(", "async fn api_state("] {
            let body = http.split_once(handler).unwrap().1;
            let first = body.split_once('}').unwrap().0;
            assert!(first.contains("if !authorized(&headers)"), "{handler}");
        }
        for handler in [
            "async fn api_screen(",
            "async fn api_keys(",
            "async fn api_followup(",
            "async fn api_stash(",
        ] {
            let body = http.split_once(handler).unwrap().1;
            let first = body.split_once("if let Some(r) = refuse(").map(|(a, _)| a);
            assert!(
                first.is_some_and(|a| !a.contains("(src.")),
                "{handler} must refuse before it reads or acts"
            );
        }
    }
}
