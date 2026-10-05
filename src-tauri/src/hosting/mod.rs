//! Optional LAN transport. Storage, rendering, and Jobs remain owned by the desktop engine.
mod api;
mod listener;
mod projection;
mod security;

use crate::{Api, AppState, error};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tauri::{Emitter, Manager, State};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast},
};
use tokio_util::sync::CancellationToken;
use umanga_core::{library, store};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    version: u32,
    port: u16,
    password_hash: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            port: 8080,
            password_hash: String::new(),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    running: bool,
    port: u16,
    password_configured: bool,
    sessions: usize,
    urls: Vec<String>,
    error: Option<umanga_core::ui_message::UiMessage>,
}

pub struct HostManager {
    config: Mutex<Config>,
    failure: Mutex<Option<String>>,
    path: PathBuf,
    active: Mutex<Option<Arc<Context>>>,
    control: tokio::sync::Mutex<()>,
    epoch: AtomicU64,
}
pub(super) struct Context {
    app: tauri::AppHandle,
    root: PathBuf,
    password_hash: String,
    epoch: u64,
    stop: CancellationToken,
    sessions: security::Sessions,
    logins: security::LoginBudget,
    hashes: Arc<Semaphore>,
    requests: Arc<Semaphore>,
    images: Arc<Semaphore>,
    streams: Arc<Semaphore>,
    hosts: HashSet<String>,
    urls: Vec<String>,
    books: Mutex<HashMap<String, String>>,
    assets: HashMap<String, Arc<[u8]>>,
    events: broadcast::Sender<Value>,
    serial: AtomicU64,
}
impl HostManager {
    pub fn new(folder: &Path) -> Self {
        let path = folder.join("hosting.json");
        let loaded = if path.exists() {
            std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|b| {
                    let c: Config = serde_json::from_slice(&b)?;
                    anyhow::ensure!(
                        c.version == 1
                            && c.port != 0
                            && (c.password_hash.is_empty()
                                || security::valid_hash(&c.password_hash)),
                        "Invalid hosting configuration; set a new password before starting."
                    );
                    Ok(c)
                })
        } else {
            Ok(Config::default())
        };
        let (config, failure) = match loaded {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e.to_string())),
        };
        Self {
            config: Mutex::new(config),
            failure: Mutex::new(failure),
            path,
            active: Mutex::new(None),
            control: tokio::sync::Mutex::new(()),
            epoch: AtomicU64::new(0),
        }
    }
    pub fn stop(&self) {
        if let Some(ctx) = self.active.lock().take() {
            ctx.sessions.clear();
            ctx.stop.cancel();
            let _ = ctx.app.emit("hosting-changed", false);
        }
    }
    pub fn publish_jobs(&self, batch: &crate::jobs_api::JobsBatch) {
        if let Some(ctx) = self.active.lock().as_ref() {
            let value = projection::jobs(ctx, serde_json::to_value(batch).unwrap_or(Value::Null));
            if value["jobs"].as_array().is_some_and(|jobs| {
                jobs.iter()
                    .filter(|job| projection::is_terminal(job))
                    .count()
                    > 100
            }) {
                // A recovery/control snapshot must not grow every browser's history.
                ctx.publish("resync", json!({}));
            } else {
                ctx.publish("jobs", value);
            }
        }
    }
    pub fn publish_requirements(&self) {
        if let Some(ctx) = self.active.lock().as_ref() {
            ctx.publish("requirements", json!({}));
        }
    }
    pub fn content_changed(&self, path: &str, page_id: Option<&str>) {
        if let Some(ctx) = self.active.lock().as_ref() {
            let id = ctx
                .books
                .lock()
                .iter()
                .find(|(_, p)| p.as_str() == path)
                .map(|(id, _)| id.clone());
            ctx.publish("content", json!({"bookId": id, "pageId": page_id}));
        }
    }
    fn status(&self) -> HostStatus {
        let c = self.config.lock().clone();
        let running = self
            .active
            .lock()
            .as_ref()
            .filter(|a| !a.stop.is_cancelled())
            .cloned();
        HostStatus {
            running: running.is_some(),
            port: c.port,
            password_configured: !c.password_hash.is_empty(),
            sessions: running.as_ref().map_or(0, |c| c.sessions.count()),
            urls: running
                .as_ref()
                .map_or_else(|| addresses(c.port).0, |c| c.urls.clone()),
            error: self.failure.lock().as_ref().map(crate::error),
        }
    }
}
impl Context {
    fn publish(&self, kind: &str, value: Value) {
        // A lagged client gets a resync, never an unbounded backlog of manga/job data.
        let serial = self.serial.fetch_add(1, Ordering::SeqCst) + 1;
        if serde_json::to_vec(&value).is_ok_and(|b| b.len() <= 1024 * 1024) {
            let _ = self
                .events
                .send(json!({"kind":kind,"epoch":self.epoch,"serial":serial,"data":value}));
        } else {
            let _ = self
                .events
                .send(json!({"kind":"resync","epoch":self.epoch,"serial":serial}));
        }
    }
    fn replace_books(&self, books: &[library::BookSummary]) {
        *self.books.lock() = books
            .iter()
            .map(|b| (b.id.clone(), b.path.clone()))
            .collect();
    }
    fn book_path(&self, id: &str) -> Api<String> {
        umanga_core::safety::identity(id).map_err(error)?;
        if self.stop.is_cancelled() {
            return Err("Hosting has stopped.".into());
        }
        let path = self
            .books
            .lock()
            .get(id)
            .cloned()
            .ok_or_else(|| crate::error("Book is no longer available; refresh Library."))?;
        if Path::new(&path).parent() != Some(self.root.as_path()) {
            return Err(
                "Book does not belong to the active library; reopen it from Library".into(),
            );
        }
        Ok(path)
    }
}
fn addresses(port: u16) -> (Vec<String>, HashSet<String>) {
    let mut ips = vec![Ipv4Addr::LOCALHOST];
    if let Ok(entries) = if_addrs::get_if_addrs() {
        for e in entries {
            if let std::net::IpAddr::V4(ip) = e.ip()
                && security::private_peer(ip)
            {
                ips.push(ip);
            }
        }
    }
    ips.sort();
    ips.dedup();
    let mut hosts: HashSet<_> = ips.iter().map(|ip| format!("{ip}:{port}")).collect();
    hosts.insert(format!("localhost:{port}"));
    if let Ok(name) = std::env::var("COMPUTERNAME")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !name.is_empty()
    {
        hosts.insert(format!("{}:{port}", name.to_ascii_lowercase()));
        hosts.insert(format!("{}.local:{port}", name.to_ascii_lowercase()));
    }
    if port == 80 {
        for host in hosts.clone() {
            hosts.insert(host.trim_end_matches(":80").to_owned());
        }
    }
    (
        ips.into_iter()
            .filter(|ip| !ip.is_loopback())
            .chain([Ipv4Addr::LOCALHOST])
            .map(|ip| format!("http://{ip}:{port}"))
            .collect(),
        hosts,
    )
}
fn frontend_assets(app: &tauri::AppHandle) -> anyhow::Result<HashMap<String, Arc<[u8]>>> {
    let mut result = HashMap::new();
    // iter() yields compressed bytes in release builds. Resolve only enumerated,
    // allowlisted keys through Tauri's decompressor, never its missing-path fallback.
    let resolver = app.asset_resolver();
    for (key, _) in resolver.iter() {
        let key = key.trim_start_matches('/').to_owned();
        if allowed_asset(&key) {
            let asset = resolver
                .get(key.clone())
                .ok_or_else(|| anyhow::anyhow!("Bundled application asset is unavailable"))?;
            result.insert(key, Arc::from(asset.bytes));
        }
    }
    // Development uses the same built frontend; never serve arbitrary paths from disk.
    if !result.contains_key("index.html") {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist");
        fn collect(
            root: &Path,
            dir: &Path,
            out: &mut HashMap<String, Arc<[u8]>>,
        ) -> anyhow::Result<()> {
            for e in std::fs::read_dir(dir)? {
                let e = e?;
                umanga_core::safety::no_link(&e.path())?;
                if e.file_type()?.is_dir() {
                    collect(root, &e.path(), out)?;
                } else {
                    let key = e
                        .path()
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if allowed_asset(&key) {
                        out.insert(key, Arc::from(std::fs::read(e.path())?));
                    }
                }
            }
            Ok(())
        }
        collect(&root, &root, &mut result)?;
    }
    let html = result
        .get("index.html")
        .ok_or_else(|| anyhow::anyhow!("Build the frontend before starting hosting."))?;
    let html = std::str::from_utf8(html)?.replacen(
        "<head>",
        "<head><meta name=\"umanga-runtime\" content=\"hosted\">",
        1,
    );
    result.insert("index.html".into(), Arc::from(html.into_bytes()));
    Ok(result)
}
fn allowed_asset(key: &str) -> bool {
    !key.contains("..")
        && !key.contains('\\')
        && !key.starts_with('/')
        && matches!(
            Path::new(key).extension().and_then(|s| s.to_str()),
            Some(
                "html"
                    | "js"
                    | "css"
                    | "svg"
                    | "png"
                    | "jpg"
                    | "webp"
                    | "ico"
                    | "woff"
                    | "woff2"
                    | "ttf"
            )
        )
}
#[tauri::command]
pub async fn host_status(state: State<'_, AppState>) -> Api<HostStatus> {
    let host = state.host.clone();
    tauri::async_runtime::spawn_blocking(move || host.status())
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn host_stop(state: State<'_, AppState>) -> Api<HostStatus> {
    let _guard = state.host.control.lock().await;
    state.host.stop();
    drop(_guard);
    host_status(state).await
}
#[tauri::command]
pub async fn host_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    port: u16,
    password: Option<String>,
) -> Api<HostStatus> {
    let _control = state.host.control.lock().await;
    let _storage = state.storage_change.lock().await;
    if state.host.active.lock().is_some() {
        return Err("Stop hosting before changing its configuration.".into());
    }
    if port == 0 {
        return Err("Use a port from 1 to 65535.".into());
    }
    let root = PathBuf::from(&state.settings.lock().library_directory);
    if root.as_os_str().is_empty() {
        return Err("Choose a library folder on the PC before hosting.".into());
    }
    let root = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        root.canonicalize()?;
        let books = library::list(&root)?;
        Ok((root, books))
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    let old = state.host.config.lock().clone();
    let password_hash = match password.filter(|p| !p.is_empty()) {
        Some(p) => tauri::async_runtime::spawn_blocking(move || security::hash_password(&p))
            .await
            .map_err(error)?
            .map_err(error)?,
        None if !old.password_hash.is_empty() => old.password_hash,
        _ => return Err("Set a hosting password of at least 8 characters.".into()),
    };
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
        .await
        .map_err(|e| {
            let mut m = umanga_core::ui_message::UiMessage::from_text(
                "Could not start hosting on port {port}.",
            );
            m.args.insert("port".into(), port.to_string());
            m.fallback = format!("Could not start hosting on port {port}.");
            m.detail = Some(e.to_string());
            m
        })?;
    let assets_app = app.clone();
    let assets = tauri::async_runtime::spawn_blocking(move || frontend_assets(&assets_app))
        .await
        .map_err(error)?
        .map_err(error)?;
    let config = Config {
        version: 1,
        port,
        password_hash: password_hash.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&config).map_err(error)?;
    let file = state.host.path.clone();
    tauri::async_runtime::spawn_blocking(move || store::atomic_write(&file, &bytes))
        .await
        .map_err(error)?
        .map_err(error)?;
    let (events, _) = broadcast::channel(64);
    let (urls, hosts) = addresses(port);
    let ctx = Arc::new(Context {
        app,
        root: root.0,
        password_hash,
        epoch: state.host.epoch.fetch_add(1, Ordering::SeqCst) + 1,
        stop: CancellationToken::new(),
        sessions: Default::default(),
        logins: Default::default(),
        hashes: Arc::new(Semaphore::new(2)),
        requests: Arc::new(Semaphore::new(32)),
        images: Arc::new(Semaphore::new(4)),
        streams: Arc::new(Semaphore::new(16)),
        hosts,
        urls,
        books: Mutex::new(HashMap::new()),
        assets,
        events,
        serial: AtomicU64::new(0),
    });
    ctx.replace_books(&root.1);
    let router = api::router(ctx.clone());
    let token = ctx.stop.clone();
    let serve = listener::BoundedListener::new(listener, 96, token.clone());
    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(
            serve,
            router.into_make_service_with_connect_info::<listener::Peer>(),
        )
        .with_graceful_shutdown(token.cancelled_owned())
        .await;
    });
    *state.host.config.lock() = config;
    *state.host.failure.lock() = None;
    *state.host.active.lock() = Some(ctx);
    let _ = state.app.emit("hosting-changed", true);
    Ok(state.host.status())
}
#[tauri::command]
pub fn host_qr(state: State<'_, AppState>, url: String) -> Api<String> {
    let status = state.host.status();
    if !status.urls.contains(&url) {
        return Err("Choose one of the displayed hosting addresses.".into());
    }
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(error)?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .build())
}
pub fn notify(state: &AppState, path: &str, page_id: Option<&str>) {
    state.host.content_changed(path, page_id);
}
pub fn notify_clients(app: &tauri::AppHandle, path: &str, page_id: Option<&str>) {
    let _ = app.emit("content-changed", json!({"path":path,"pageId":page_id}));
    if let Some(state) = app.try_state::<AppState>() {
        notify(&state, path, page_id);
    }
}
