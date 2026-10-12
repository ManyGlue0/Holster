//! Holster's local API, for the apps in the dock. Loopback only, on a random
//! free port that apps discover through `%APPDATA%\Holster\holster.json`.
//!
//! - `GET  /status`            `{ "running": true, "version": "..." }`
//! - `POST /apps/{id}/signal`  `{ "kind": "activity" }` tints the tab in the app's color
//! - `POST /apps/{id}/reload`  re-reads the manifests, answers with this app's result
//! - `POST /apps/{id}/notify`  shows (or, same `id`, updates) a notification (see notify.rs)
//! - `DELETE /apps/{id}/notify/{nid}` takes a notification back
//! - `GET  /apps/{id}/preview/{file}` serves the app's on-disk preview folder
//!
//! Requests must name the loopback host (DNS-rebinding guard) and may only
//! come from loopback origins when a browser sends them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Path as UrlPath, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::apps::Registry;
use crate::notify::{self, Notifier};

type SignalFn = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
struct Ctx {
    port: u16,
    version: String,
    reg: Arc<Registry>,
    notifier: Arc<Notifier>,
    signal: SignalFn,
}

#[derive(Serialize)]
struct Discovery<'a> {
    version: &'a str,
    port: u16,
    pid: u32,
    api: String,
}

fn loopback_host(value: &str, port: u16) -> bool {
    let p = port.to_string();
    [format!("127.0.0.1:{p}"), format!("localhost:{p}")].iter().any(|h| h.eq_ignore_ascii_case(value))
}

fn loopback_origin(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("http://") else { return value == "null" };
    let host = rest.split(':').next().unwrap_or("");
    let port_ok = rest.split_once(':').map(|(_, p)| p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty()).unwrap_or(true);
    (host == "127.0.0.1" || host == "localhost") && port_ok
}

async fn guard(State(c): State<Ctx>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !loopback_host(host, c.port) {
        return (StatusCode::FORBIDDEN, "Holster: loopback only").into_response();
    }
    if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|h| h.to_str().ok()) {
        if !loopback_origin(origin) {
            return (StatusCode::FORBIDDEN, "Holster: loopback origins only").into_response();
        }
    }
    next.run(req).await
}

async fn status(State(c): State<Ctx>) -> Json<Value> {
    Json(json!({ "running": true, "version": c.version }))
}

fn unknown(id: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": format!("No app with id \"{id}\" is registered") })))
        .into_response()
}

async fn signal(State(c): State<Ctx>, UrlPath(id): UrlPath<String>, body: axum::body::Bytes) -> Response {
    let body: Option<Value> = serde_json::from_slice(&body).ok();
    let kind = body.as_ref().and_then(|b| b.get("kind")).and_then(Value::as_str).unwrap_or("");
    if kind != "activity" {
        return (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": "kind must be \"activity\"" }))).into_response();
    }
    if c.reg.manifest(&id).is_none() {
        return unknown(&id);
    }
    (c.signal)(&id);
    Json(json!({ "ok": true })).into_response()
}

async fn reload(State(c): State<Ctx>, UrlPath(id): UrlPath<String>) -> Response {
    let reg = c.reg.clone();
    let _ = tokio::task::spawn_blocking(move || reg.reload()).await;
    match c.reg.entries().into_iter().find(|e| e.id == id) {
        Some(e) => match e.manifest {
            Ok(_) => Json(json!({ "ok": true, "warnings": e.warnings })).into_response(),
            Err(err) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "ok": false, "error": err }))).into_response(),
        },
        None => unknown(&id),
    }
}

fn bad_request(error: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": error }))).into_response()
}

async fn notify(State(c): State<Ctx>, UrlPath(id): UrlPath<String>, body: axum::body::Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<Value>(&body) else {
        return bad_request("The body must be JSON".into());
    };
    let req = match notify::parse_request(&body) {
        Ok(r) => r,
        Err(e) => return bad_request(e),
    };
    if c.reg.manifest(&id).is_none() {
        return unknown(&id);
    }
    let posted = c.notifier.post(&id, req);
    Json(json!({ "ok": true, "id": posted.id, "muted": posted.muted })).into_response()
}

async fn withdraw(State(c): State<Ctx>, UrlPath((id, nid)): UrlPath<(String, String)>) -> Response {
    let Some(nid) = decode(&nid) else { return bad_request("Bad notification id".into()) };
    if c.notifier.withdraw(&id, &nid) {
        Json(json!({ "ok": true })).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": format!("No notification \"{nid}\" from \"{id}\"") })))
            .into_response()
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Files from an app's preview folder, never outside it.
async fn preview_file(State(c): State<Ctx>, UrlPath((id, file)): UrlPath<(String, String)>) -> Response {
    let Some(dir) = c.reg.preview_dir(&id) else { return unknown(&id) };
    let Some(rel) = decode(&file) else { return (StatusCode::BAD_REQUEST, "Bad path").into_response() };
    if rel.split(['/', '\\']).any(|seg| seg == ".." || seg.contains(':')) {
        return (StatusCode::FORBIDDEN, "Outside the preview folder").into_response();
    }
    let path = dir.join(rel.replace('/', "\\"));
    let inside = match (std::fs::canonicalize(&dir), std::fs::canonicalize(&path)) {
        (Ok(d), Ok(p)) => p.starts_with(d),
        _ => false,
    };
    if !inside {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, content_type(&path)), (header::CACHE_CONTROL, "no-store")],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

pub fn discovery_path(dir: &Path) -> PathBuf {
    dir.join("holster.json")
}

/// Binds a random loopback port, writes the discovery file and serves.
pub async fn start(
    dir: PathBuf,
    version: String,
    reg: Arc<Registry>,
    notifier: Arc<Notifier>,
    on_signal: impl Fn(&str) + Send + Sync + 'static,
) -> anyhow::Result<u16> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let ctx = Ctx { port, version: version.clone(), reg, notifier, signal: Arc::new(on_signal) };
    let router = Router::new()
        .route("/status", get(status))
        .route("/apps/{id}/signal", post(signal))
        .route("/apps/{id}/reload", post(reload))
        .route("/apps/{id}/notify", post(notify))
        .route("/apps/{id}/notify/{nid}", delete(withdraw))
        .route("/apps/{id}/preview/{*file}", get(preview_file))
        .fallback(|| async { (StatusCode::NOT_FOUND, "Not found") })
        .layer(middleware::from_fn_with_state(ctx.clone(), guard))
        .with_state(ctx);

    let file = discovery_path(&dir);
    let tmp = file.with_extension("json.tmp");
    let body = Discovery { version: &version, port, pid: std::process::id(), api: format!("http://127.0.0.1:{port}") };
    std::fs::write(&tmp, serde_json::to_vec_pretty(&body)?)?;
    std::fs::rename(&tmp, &file)?;

    tokio::spawn(async move {
        let _ = axum::serve(listener, router.into_make_service()).await;
    });
    Ok(port)
}

/// Removes the discovery file on exit, if it is still ours.
pub fn remove_discovery(dir: &Path) {
    let file = discovery_path(dir);
    let ours = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("pid").and_then(Value::as_u64))
        == Some(std::process::id() as u64);
    if ours {
        let _ = std::fs::remove_file(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_origin_rules() {
        assert!(loopback_host("127.0.0.1:5000", 5000));
        assert!(loopback_host("localhost:5000", 5000));
        assert!(!loopback_host("evil.com:5000", 5000));
        assert!(!loopback_host("127.0.0.1:5001", 5000));
        assert!(loopback_origin("http://127.0.0.1:47321"));
        assert!(loopback_origin("http://localhost"));
        assert!(!loopback_origin("https://evil.com"));
        assert!(!loopback_origin("http://127.0.0.1.evil.com"));
    }
}
