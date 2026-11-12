//! Member session + Fabric search_addr verify for nginx auth_request.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use dashmap::DashMap;
use fabric::TrustId;
use fabric::client::Fabric;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

type HmacSha256 = Hmac<Sha256>;

const COOKIE_NAME: &str = "bv_member";

#[derive(Clone)]
struct AppState {
    fabric: Arc<RwLock<Fabric>>,
    secret: Vec<u8>,
    session_max_age: u64,
    verify_cache_ttl: Duration,
    allowed_handles: Option<Vec<String>>,
    verify_cache: Arc<DashMap<String, CachedVerify>>,
    search_timeout: Duration,
    seeds: Vec<String>,
    index_client: reqwest::Client,
    dev_mode: bool,
}

#[derive(Clone)]
struct CachedVerify {
    handle: String,
    until: SystemTime,
}

#[derive(Debug, Deserialize)]
struct AddrsQuery {
    name: String,
    addr: String,
}

#[derive(Debug, Serialize)]
struct AddrsResponse {
    address: String,
    handles: Vec<HandleEntry>,
}

#[derive(Debug, Serialize)]
struct HandleEntry {
    handle: String,
}

#[derive(Debug, Deserialize)]
struct SessionBody {
    npub: String,
    #[serde(default)]
    pubkey: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionOk {
    npub: String,
    handle: String,
    handles: Vec<String>,
}

#[derive(Debug, Serialize)]
struct VerifyOk {
    npub: String,
    handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pubkey: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CookiePayload {
    npub: String,
    pubkey: Option<String>,
    handle: String,
    exp: u64,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("fabric-lookupd: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let secret = std::env::var("SESSION_SECRET")
        .unwrap_or_else(|_| "dev-insecure-change-in-production".into());
    let session_max_age: u64 = std::env::var("SESSION_MAX_AGE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(86_400);
    let verify_cache_ttl = Duration::from_secs(
        std::env::var("VERIFY_CACHE_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
    );
    let search_timeout = Duration::from_secs(
        std::env::var("FABRIC_SEARCH_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(90),
    );
    let index_timeout = Duration::from_secs(
        std::env::var("FABRIC_INDEX_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15),
    );
    let index_client = reqwest::Client::builder()
        .timeout(index_timeout)
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let seeds: Vec<String> = std::env::var("FABRIC_SEEDS")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_else(|_| {
            vec!["https://relay-atlas.spacesprotocol.org".to_string()]
        });
    let dev_mode = std::env::var("FABRIC_DEV_MODE")
        .map(|s| matches!(s.as_str(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false);
    let trust_id = std::env::var("FABRIC_TRUST_ID")
        .ok()
        .and_then(|s| TrustId::from_str(&s).ok());

    let allowed_handles = std::env::var("FABRIC_ALLOWED_HANDLES").ok().map(|s| {
        s.split(',')
            .map(|h| normalize_handle(h.trim()))
            .filter(|h| !h.is_empty())
            .collect()
    });

    let fabric = build_fabric(&seeds, dev_mode, trust_id).await?;

    let state = AppState {
        fabric: Arc::new(RwLock::new(fabric)),
        secret: secret.into_bytes(),
        session_max_age,
        verify_cache_ttl,
        allowed_handles,
        verify_cache: Arc::new(DashMap::new()),
        search_timeout,
        seeds: seeds.clone(),
        index_client,
        dev_mode,
    };

    let app = Router::new()
        .route("/addrs", get(get_addrs))
        .route("/auth/session", post(post_session))
        .route("/auth/verify", get(get_verify))
        .route("/auth/logout", post(post_logout))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|e| format!("bind {bind}: {e}"))?;
    eprintln!("fabric-lookupd listening on http://{bind} (seeds: {seeds:?})");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("serve: {e}"))?;
    Ok(())
}

async fn build_fabric(
    seeds: &[String],
    dev_mode: bool,
    trust_id: Option<TrustId>,
) -> Result<Fabric, String> {
    let refs: Vec<&str> = seeds.iter().map(|s| s.as_str()).collect();
    let mut fabric = Fabric::with_seeds(&refs);
    if dev_mode {
        fabric = fabric.with_dev_mode();
    }
    if let Some(id) = trust_id {
        fabric
            .trust(id)
            .await
            .map_err(|e| format!("trust id: {e}"))?;
    }
    Ok(fabric)
}

async fn relay_index_rev_handles(state: &AppState, name: &str, addr: &str) -> Vec<String> {
    let mut out = Vec::new();
    for seed in &state.seeds {
        let url = format!("{seed}/addrs");
        let resp = match state
            .index_client
            .get(&url)
            .query(&[("name", name), ("addr", addr)])
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => continue,
        };
        if !resp.status().is_success() {
            continue;
        }
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(handles) = body.get("handles").and_then(|h| h.as_array()) else {
            continue;
        };
        for entry in handles {
            if let Some(rev) = entry.get("rev").and_then(|v| v.as_str()) {
                out.push(rev.to_string());
            } else if let Some(handle) = entry.get("handle").and_then(|v| v.as_str()) {
                out.push(handle.to_string());
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out.sort();
    out.dedup();
    out
}

fn fabric_cli_bin() -> String {
    std::env::var("FABRIC_BIN").unwrap_or_else(|_| {
        format!(
            "{}/../../certrelay/target/release/fabric",
            env!("CARGO_MANIFEST_DIR")
        )
    })
}

fn json_zone_has_addr(zone: &serde_json::Value, name: &str, addr: &str) -> bool {
    let Some(records) = zone.get("records").and_then(|v| v.as_array()) else {
        return false;
    };
    records.iter().any(|r| {
        r.get("type").and_then(|t| t.as_str()) == Some("addr")
            && r.get("key").and_then(|k| k.as_str()) == Some(name)
            && r.get("value")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                == Some(addr)
    })
}

async fn resolve_handle_via_cli(
    state: &AppState,
    rev: &str,
    name: &str,
    addr: &str,
) -> Result<Option<String>, String> {
    let bin = fabric_cli_bin();
    if !std::path::Path::new(&bin).exists() {
        return Err(format!(
            "fabric CLI missing at {bin} (run: cd ../../certrelay && cargo build -p fabric-resolver --release --bin fabric)"
        ));
    }
    let seeds_arg = state.seeds.join(",");
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(["resolve", "--seeds", &seeds_arg]);
    if state.dev_mode {
        cmd.arg("--dev-mode");
    }
    cmd.arg(rev);
    let timeout = Duration::from_secs(state.search_timeout.as_secs().max(90));
    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("fabric resolve spawn: {e}")),
        Err(_) => {
            return Err(format!(
                "fabric resolve timed out for {rev} after {}s",
                timeout.as_secs()
            ));
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            format!("fabric resolve failed for {rev}")
        } else {
            stderr.trim().to_string()
        });
    }
    let zone: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("fabric json: {e}"))?;
    if !json_zone_has_addr(&zone, name, addr) {
        return Ok(None);
    }
    Ok(zone
        .get("handle")
        .and_then(|h| h.as_str())
        .map(String::from))
}

fn normalize_handle(h: &str) -> String {
    let lower = h.to_lowercase();
    if lower.contains('@') {
        lower
    } else {
        format!("{lower}@space")
    }
}

async fn verified_handles(state: &AppState, name: &str, addr: &str) -> Result<Vec<String>, String> {
    if let Some(cached) = state.verify_cache.get(addr) {
        if cached.until > SystemTime::now() {
            return Ok(vec![cached.handle.clone()]);
        }
        drop(cached);
        state.verify_cache.remove(addr);
    }

    let index_revs = relay_index_rev_handles(state, name, addr).await;
    if index_revs.is_empty() {
        return Ok(vec![]);
    }

    let mut handles = Vec::new();
    for rev in &index_revs {
        if let Some(handle) = resolve_handle_via_cli(state, rev, name, addr).await? {
            handles.push(handle);
        }
    }

    handles.sort();
    handles.dedup();

    if let Some(allow) = &state.allowed_handles {
        handles.retain(|h| allow.iter().any(|a| a == &normalize_handle(h)));
    }

    if let Some(primary) = handles.first() {
        state.verify_cache.insert(
            addr.to_string(),
            CachedVerify {
                handle: primary.clone(),
                until: SystemTime::now() + state.verify_cache_ttl,
            },
        );
    }

    Ok(handles)
}

async fn get_addrs(State(state): State<AppState>, Query(q): Query<AddrsQuery>) -> Response {
    match verified_handles(&state, &q.name, &q.addr).await {
        Ok(handles) => {
            let body = AddrsResponse {
                address: q.addr.clone(),
                handles: handles
                    .into_iter()
                    .map(|handle| HandleEntry { handle })
                    .collect(),
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn post_session(State(state): State<AppState>, Json(body): Json<SessionBody>) -> Response {
    let npub = body.npub.trim();
    if !npub.starts_with("npub1") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid npub" })),
        )
            .into_response();
    }

    let handles = match verified_handles(&state, "nostr", npub).await {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    if handles.is_empty() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "no verified @space handle for this npub"
            })),
        )
            .into_response();
    }

    let primary = handles[0].clone();
    let exp = now_secs() + state.session_max_age;
    let payload = CookiePayload {
        npub: npub.to_string(),
        pubkey: body.pubkey.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        handle: primary.clone(),
        exp,
    };

    let cookie = match seal_cookie(&state.secret, &payload) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    let mut headers = HeaderMap::new();
    let set_cookie = format!(
        "{COOKIE_NAME}={cookie}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        state.session_max_age
    );
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie).unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    let body = SessionOk {
        npub: npub.to_string(),
        handle: primary,
        handles,
    };
    (StatusCode::OK, headers, Json(body)).into_response()
}

async fn get_verify(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(payload) = open_cookie(&state.secret, cookie_from_headers(&headers)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    if payload.exp < now_secs() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    match verified_handles(&state, "nostr", &payload.npub).await {
        Ok(handles) if !handles.is_empty() => {
            let handle = handles[0].clone();
            let mut resp = (
                StatusCode::OK,
                Json(VerifyOk {
                    npub: payload.npub,
                    handle: handle.clone(),
                    pubkey: payload.pubkey,
                }),
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&handle) {
                resp.headers_mut().insert("X-Verified-Handle", v);
            }
            resp
        }
        Ok(_) => StatusCode::FORBIDDEN.into_response(),
        Err(_) => StatusCode::FORBIDDEN.into_response(),
    }
}

async fn post_logout() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "bv_member=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        ),
    );
    (StatusCode::NO_CONTENT, headers).into_response()
}

fn cookie_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix(&format!("{COOKIE_NAME}=")) {
            return Some(val.to_string());
        }
    }
    None
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn seal_cookie(secret: &[u8], payload: &CookiePayload) -> Result<String, String> {
    let json = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    let b64 = base64_url_encode(&json);
    let sig = sign(secret, b64.as_bytes());
    Ok(format!("{b64}.{sig}"))
}

fn open_cookie(secret: &[u8], token: Option<String>) -> Option<CookiePayload> {
    let token = token?;
    let (b64, sig) = token.split_once('.')?;
    if !constant_time_eq(sign(secret, b64.as_bytes()).as_bytes(), sig.as_bytes()) {
        return None;
    }
    let json = base64_url_decode(b64)?;
    serde_json::from_slice(&json).ok()
}

fn sign(secret: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(data);
    base64_url_encode(mac.finalize().into_bytes().as_slice())
}

fn base64_url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn base64_url_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
