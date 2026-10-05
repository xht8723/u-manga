use super::{Context, listener::Peer, projection, security};
use crate::{AppState, error};
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Path as RoutePath, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::Manager;
use tokio::io::AsyncReadExt;
use umanga_core::{assets, documents, library, store, types::Page, ui_message::UiMessage};
type Result<T> = std::result::Result<T, ApiError>;
pub struct ApiError(StatusCode, UiMessage);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
impl From<UiMessage> for ApiError {
    fn from(e: UiMessage) -> Self {
        Self(StatusCode::CONFLICT, e)
    }
}
fn fail(status: StatusCode, text: &str) -> ApiError {
    ApiError(status, error(text))
}
fn invalid(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, error(e))
}
fn value<T: Serialize>(v: T) -> Result<Json<Value>> {
    Ok(Json(serde_json::to_value(v).map_err(invalid)?))
}
/// Once an edit enters native ownership, a dropped socket must not release its
/// storage gate or suppress the committed-save notification.
async fn durable<T: Send + 'static>(
    work: impl std::future::Future<Output = crate::Api<T>> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn(work)
        .await
        .map_err(invalid)?
        .map_err(ApiError::from)
}
pub fn router(ctx: Arc<Context>) -> Router {
    Router::new()
        .route("/api/v1/login", post(login))
        .route("/api/v1/session", get(session))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/bootstrap", get(bootstrap))
        .route("/api/v1/books", get(books))
        .route("/api/v1/books/{book}", get(book))
        .route(
            "/api/v1/books/{book}/glossary",
            get(book_glossary)
                .post(save_glossary)
                .layer(DefaultBodyLimit::max(
                    6 * umanga_core::glossary::MAX_FILE_BYTES + 26 * 10_000 + 2048,
                )),
        )
        .route("/api/v1/books/{book}/summary", get(summary))
        .route("/api/v1/books/{book}/chapters/{chapter}", get(chapter))
        .route(
            "/api/v1/books/{book}/chapters/{chapter}/completion",
            post(completion),
        )
        .route("/api/v1/books/{book}/pages/{page}", get(page))
        .route("/api/v1/books/{book}/pages/{page}/image", get(image))
        .route(
            "/api/v1/books/{book}/pages/{page}/thumbnail",
            get(thumbnail),
        )
        .route("/api/v1/books/{book}/cover", get(cover))
        .route("/api/v1/books/{book}/availability", post(availability))
        .route("/api/v1/books/{book}/translate", post(enqueue))
        .route(
            "/api/v1/books/{book}/batch",
            get(batch_preview).post(batch_submit),
        )
        .route("/api/v1/books/{book}/pages/{page}/restart", post(restart))
        .route("/api/v1/books/{book}/pages/{page}/prepare", post(prepare))
        .route(
            "/api/v1/books/{book}/pages/{page}/regions/{region}/text",
            post(edit_text),
        )
        .route(
            "/api/v1/books/{book}/pages/{page}/regions/{region}/delete",
            post(delete_region),
        )
        .route("/api/v1/jobs", get(jobs))
        .route("/api/v1/jobs/availability", post(jobs_availability))
        .route("/api/v1/jobs/control", post(control_all))
        .route("/api/v1/jobs/{id}/control", post(control))
        .route("/api/v1/events", get(events))
        .fallback(static_asset)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(ctx.clone(), boundary))
        .with_state(ctx)
}
/// No proxy headers, cross-origin access, native commands, or client-supplied paths.
async fn boundary(
    State(ctx): State<Arc<Context>>,
    ConnectInfo(Peer(peer)): ConnectInfo<Peer>,
    request: Request,
    next: Next,
) -> Response {
    let rejected = (|| -> Result<()> {
        if ctx.stop.is_cancelled() {
            return Err(fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "Hosting has stopped.",
            ));
        }
        if !peer.ip().is_ipv4()
            || !security::private_peer(
                peer.ip()
                    .to_canonical()
                    .to_string()
                    .parse()
                    .map_err(invalid)?,
            )
        {
            return Err(fail(
                StatusCode::FORBIDDEN,
                "Hosting is available only on a private network.",
            ));
        }
        security::request_site(request.headers(), &ctx.hosts)
            .map_err(|reason| fail(StatusCode::FORBIDDEN, reason))?;
        let path = request.uri().path();
        if path.starts_with("/api/") && path != "/api/v1/login" {
            let token = security::cookie_token(request.headers()).ok_or_else(|| {
                fail(
                    StatusCode::UNAUTHORIZED,
                    "Sign in to U-Manga on this browser.",
                )
            })?;
            let session = ctx.sessions.get(&token, Instant::now()).ok_or_else(|| {
                fail(
                    StatusCode::UNAUTHORIZED,
                    "Your session expired. Sign in again; your drafts are retained.",
                )
            })?;
            if request.method() != axum::http::Method::GET
                && request.method() != axum::http::Method::HEAD
                && request
                    .headers()
                    .get("x-umanga-csrf")
                    .and_then(|s| s.to_str().ok())
                    != Some(session.csrf.as_str())
            {
                return Err(fail(
                    StatusCode::FORBIDDEN,
                    "Refresh this browser session before saving.",
                ));
            }
        }
        Ok(())
    })();
    let mut response = match rejected {
        Err(e) => e.into_response(),
        Ok(()) => match ctx.requests.clone().try_acquire_owned() {
            Ok(_permit) => next.run(request).await,
            Err(_) => fail(
                StatusCode::TOO_MANY_REQUESTS,
                "The server is busy. Try again shortly.",
            )
            .into_response(),
        },
    };
    if (response.status().is_client_error() || response.status().is_server_error())
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_none_or(|v| {
                !v.to_str()
                    .unwrap_or_default()
                    .starts_with("application/json")
            })
    {
        response = fail(
            response.status(),
            if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
                "This request is too large."
            } else {
                "Invalid request. Check its fields and resource identities."
            },
        )
        .into_response();
    }
    let h = response.headers_mut();
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert("x-frame-options", "DENY".parse().unwrap());
    h.insert("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    h.entry(header::CACHE_CONTROL)
        .or_insert("no-store".parse().unwrap());
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    password: String,
}
async fn login(
    State(ctx): State<Arc<Context>>,
    ConnectInfo(Peer(peer)): ConnectInfo<Peer>,
    Json(input): Json<Login>,
) -> Result<Response> {
    let ip = match peer.ip() {
        std::net::IpAddr::V4(ip) => ip,
        _ => {
            return Err(fail(
                StatusCode::FORBIDDEN,
                "Private IPv4 network required.",
            ));
        }
    };
    if !ctx.logins.take(ip, Instant::now()) {
        return Err(fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many sign-in attempts. Wait one minute.",
        ));
    }
    let permit = ctx.hashes.clone().try_acquire_owned().map_err(|_| {
        fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Sign-in is busy. Try again shortly.",
        )
    })?;
    let hash = ctx.password_hash.clone();
    let valid = tauri::async_runtime::spawn_blocking(move || {
        let _p = permit;
        security::verify_password(&hash, &input.password)
    })
    .await
    .map_err(invalid)?;
    if !valid {
        return Err(fail(
            StatusCode::UNAUTHORIZED,
            "Incorrect hosting password.",
        ));
    }
    if ctx.stop.is_cancelled() {
        return Err(fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Hosting has stopped.",
        ));
    }
    let (token, session) = ctx.sessions.create(Instant::now()).ok_or_else(|| {
        fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many connected sessions. Stop and restart hosting on the PC.",
        )
    })?;
    let mut r = Json(json!({"csrf":session.csrf})).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400",
            security::COOKIE
        )
        .parse()
        .unwrap(),
    );
    Ok(r)
}
async fn session(State(ctx): State<Arc<Context>>, headers: HeaderMap) -> Result<Json<Value>> {
    let token = security::cookie_token(&headers).ok_or_else(|| {
        fail(
            StatusCode::UNAUTHORIZED,
            "Sign in to U-Manga on this browser.",
        )
    })?;
    let session = ctx.sessions.get(&token, Instant::now()).ok_or_else(|| {
        fail(
            StatusCode::UNAUTHORIZED,
            "Your session expired. Sign in again; your drafts are retained.",
        )
    })?;
    value(json!({"csrf":session.csrf,"epoch":ctx.epoch}))
}
async fn logout(State(ctx): State<Arc<Context>>, headers: HeaderMap) -> Response {
    if let Some(token) = security::cookie_token(&headers) {
        ctx.sessions.remove(&token);
    }
    let mut r = Json(json!({})).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
            security::COOKIE
        )
        .parse()
        .unwrap(),
    );
    r
}
async fn bootstrap(State(ctx): State<Arc<Context>>) -> Json<Value> {
    Json(projection::bootstrap(
        &ctx.app.state::<AppState>().settings.lock(),
    ))
}
async fn books(State(ctx): State<Arc<Context>>) -> Result<Json<Value>> {
    let b = crate::library_list(ctx.app.clone(), ctx.app.state()).await?;
    ctx.replace_books(&b);
    value(b.into_iter().map(projection::summary).collect::<Vec<_>>())
}
async fn book(
    State(ctx): State<Arc<Context>>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Value>> {
    let path = ctx.book_path(&id)?;
    value(projection::book(
        crate::book_open(ctx.app.clone(), ctx.app.state(), path).await?,
    ))
}
async fn summary(
    State(ctx): State<Arc<Context>>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Value>> {
    let path = ctx.book_path(&id)?;
    value(projection::summary(crate::book_refresh(path).await?))
}
async fn book_glossary(
    State(ctx): State<Arc<Context>>,
    RoutePath(id): RoutePath<String>,
) -> Result<Json<Value>> {
    let path = ctx.book_path(&id)?;
    value(projection::glossary_book(
        crate::book_open(ctx.app.clone(), ctx.app.state(), path).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlossarySave {
    expected: u64,
    glossary: umanga_core::glossary::BookGlossary,
}
async fn save_glossary(
    State(ctx): State<Arc<Context>>,
    RoutePath(id): RoutePath<String>,
    Json(input): Json<GlossarySave>,
) -> Result<Json<Value>> {
    let path = ctx.book_path(&id)?;
    let app = ctx.app.clone();
    let saved = durable(async move {
        crate::book_glossary_save(app.state(), path, input.expected, input.glossary).await
    })
    .await?;
    value(projection::glossary_book(saved))
}
async fn check_chapter(ctx: &Context, path: &str, id: &str) -> Result<()> {
    umanga_core::safety::identity(id).map_err(invalid)?;
    let b = crate::book_open(ctx.app.clone(), ctx.app.state(), path.to_owned()).await?;
    if !b.chapters.iter().any(|c| c.id == id) {
        return Err(fail(
            StatusCode::NOT_FOUND,
            "Chapter is no longer available.",
        ));
    }
    Ok(())
}
async fn chapter(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, chapter)): RoutePath<(String, String)>,
) -> Result<Json<Value>> {
    let path = ctx.book_path(&book)?;
    check_chapter(&ctx, &path, &chapter).await?;
    value(projection::project(
        crate::chapter_pages(ctx.app.state(), path, chapter, ctx.app.clone()).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Complete {
    read: bool,
}
async fn completion(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, chapter)): RoutePath<(String, String)>,
    Json(input): Json<Complete>,
) -> Result<Json<Value>> {
    let path = ctx.book_path(&book)?;
    check_chapter(&ctx, &path, &chapter).await?;
    let app = ctx.app.clone();
    let b =
        durable(
            async move { crate::chapter_complete(app.state(), path, chapter, input.read).await },
        )
        .await?;
    value(projection::book(b))
}
async fn trusted_page(ctx: &Context, book: &str, id: &str) -> Result<(String, Page)> {
    let path = ctx.book_path(book)?;
    umanga_core::safety::identity(id).map_err(invalid)?;
    let page = crate::page_get(path.clone(), id.to_owned()).await?;
    Ok((path, page))
}
async fn page(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, page)): RoutePath<(String, String)>,
) -> Result<Json<Value>> {
    let (_, p) = trusted_page(&ctx, &book, &page).await?;
    value(projection::page(p, &book))
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImageQuery {
    #[serde(default)]
    translated: bool,
    #[serde(default)]
    cleaned: bool,
    revision: Option<u64>,
}
async fn image_permit(ctx: &Context) -> Result<tokio::sync::OwnedSemaphorePermit> {
    tokio::select! {
        _ = ctx.stop.cancelled() => Err(fail(StatusCode::SERVICE_UNAVAILABLE, "Hosting has stopped.")),
        result = tokio::time::timeout(Duration::from_secs(5), ctx.images.clone().acquire_owned()) =>
            result.ok().and_then(std::result::Result::ok).ok_or_else(|| fail(StatusCode::TOO_MANY_REQUESTS, "Image loading is busy. Try again shortly.")),
    }
}
async fn image(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, page)): RoutePath<(String, String)>,
    Query(q): Query<ImageQuery>,
    headers: HeaderMap,
) -> Result<Response> {
    let path = ctx.book_path(&book)?;
    umanga_core::safety::identity(&page).map_err(invalid)?;
    let permit = image_permit(&ctx).await?;
    let (file, lease, revision, permit) =
        tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
            let permit = permit;
            let (p, lease) = assets::protect_snapshot(|| {
                let p = store::page(Path::new(&path), &page)?;
                let refs = assets::page_references(&p);
                Ok((p, refs))
            })?;
            documents::source_health(&p)?;
            let file = if q.cleaned {
                p.cleanup
                    .as_ref()
                    .map(|c| &c.path)
                    .filter(|f| Path::new(f).exists())
                    .map(PathBuf::from)
            } else if q.translated {
                p.rendered
                    .as_ref()
                    .filter(|f| Path::new(f).exists())
                    .map(PathBuf::from)
            } else {
                None
            };
            let file = file
                .map(Ok)
                .unwrap_or_else(|| documents::materialize(Path::new(&path), &p))?;
            Ok((file, lease, p.revision, permit))
        })
        .await
        .map_err(invalid)?
        .map_err(invalid)?;
    let _ = q.revision; // Versioned browser references; authorization always runs before streaming.
    stream_file(
        file,
        (lease, permit),
        ctx.stop.clone(),
        Some(revision),
        headers,
    )
    .await
}
async fn cover(
    State(ctx): State<Arc<Context>>,
    RoutePath(book): RoutePath<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let path = ctx.book_path(&book)?;
    let permit = image_permit(&ctx).await?;
    let (file, lease, permit) =
        tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
            let permit = permit;
            let (file, lease) = assets::protect_snapshot(|| {
                let book = library::open(Path::new(&path))?;
                let file = book
                    .cover
                    .ok_or_else(|| anyhow::anyhow!("Cover unavailable"))?;
                Ok((PathBuf::from(&file), vec![PathBuf::from(file)]))
            })?;
            Ok((file, lease, permit))
        })
        .await
        .map_err(invalid)?
        .map_err(invalid)?;
    stream_file(file, (lease, permit), ctx.stop.clone(), None, headers).await
}
async fn stream_file<L: Send + 'static>(
    path: PathBuf,
    lease: L,
    stop: tokio_util::sync::CancellationToken,
    revision: Option<u64>,
    headers: HeaderMap,
) -> Result<Response> {
    let mime = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    };
    let file = tokio::fs::File::open(&path).await.map_err(invalid)?;
    let metadata = file.metadata().await.map_err(invalid)?;
    // Weak file validators include the selected representation, not only page revision.
    // Authentication, source health and exact-path leases were checked before this point.
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    let etag = modified.map(|modified| {
        format!(
            "W/\"{}\"",
            store::digest(
                format!(
                    "{:?}:{revision:?}:{}:{}",
                    path,
                    metadata.len(),
                    modified.as_nanos()
                )
                .as_bytes()
            )
        )
    });
    if etag
        .as_ref()
        .is_some_and(|etag| matches_validator(&headers, etag))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        private_cache_headers(&mut response, etag.as_deref());
        return Ok(response);
    }
    let stream = futures_util::stream::try_unfold(
        (file, lease, stop),
        |(mut file, lease, stop)| async move {
            let mut buffer = vec![0; 64 * 1024];
            let n = tokio::select! { _ = stop.cancelled() => return Ok::<_, std::io::Error>(None), n = file.read(&mut buffer) => n? };
            if n == 0 {
                return Ok(None);
            }
            buffer.truncate(n);
            Ok(Some((buffer, (file, lease, stop))))
        },
    );
    let mut r = Body::from_stream(stream).into_response();
    r.headers_mut()
        .insert(header::CONTENT_TYPE, mime.parse().unwrap());
    private_cache_headers(&mut r, etag.as_deref());
    Ok(r)
}
fn matches_validator(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .any(|value| {
            value == "*"
                || value.strip_prefix("W/").unwrap_or(value)
                    == etag.strip_prefix("W/").unwrap_or(etag)
        })
}
fn private_cache_headers(response: &mut Response, etag: Option<&str>) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "private, no-cache".parse().unwrap());
    if let Some(etag) = etag {
        response
            .headers_mut()
            .insert(header::ETAG, etag.parse().unwrap());
    }
}
async fn thumbnail(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, page)): RoutePath<(String, String)>,
) -> Result<Json<Value>> {
    let permit = image_permit(&ctx).await?;
    let (path, _) = trusted_page(&ctx, &book, &page).await?;
    let app = ctx.app.clone();
    value(
        durable(async move {
            let _permit = permit;
            crate::thumbnail(app.state(), path, page).await
        })
        .await?,
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AvailabilityRequest {
    page_id: Option<String>,
    actions: Vec<umanga_core::requirements::Action>,
}
async fn availability(
    State(ctx): State<Arc<Context>>,
    RoutePath(book): RoutePath<String>,
    Json(input): Json<AvailabilityRequest>,
) -> Result<Json<Value>> {
    if input.actions.len() > 7 {
        return Err(invalid("Too many requested actions"));
    }
    let path = ctx.book_path(&book)?;
    let p = if let Some(id) = &input.page_id {
        Some(trusted_page(&ctx, &book, id).await?.1)
    } else {
        None
    };
    value(
        crate::action_readiness(
            ctx.app.state(),
            Some(path),
            p.clone(),
            p,
            None,
            Some(input.actions),
        )
        .await?,
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Submission {
    page_ids: Vec<String>,
    priority: Option<bool>,
}
fn ids(ids: &[String]) -> Result<()> {
    if ids.len() > 10_000 {
        return Err(invalid("Select at most 10,000 pages per request"));
    }
    for id in ids {
        umanga_core::safety::identity(id).map_err(invalid)?;
    }
    Ok(())
}
async fn enqueue(
    State(ctx): State<Arc<Context>>,
    RoutePath(book): RoutePath<String>,
    Json(input): Json<Submission>,
) -> Result<Json<Value>> {
    ids(&input.page_ids)?;
    let path = ctx.book_path(&book)?;
    let app = ctx.app.clone();
    value(projection::jobs(
        &ctx,
        serde_json::to_value(
            durable(async move {
                crate::enqueue(app.state(), path, input.page_ids, input.priority).await
            })
            .await?,
        )
        .map_err(invalid)?,
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchQuery {
    chapter_id: Option<String>,
}
async fn batch_preview(
    State(ctx): State<Arc<Context>>,
    RoutePath(book): RoutePath<String>,
    Query(q): Query<BatchQuery>,
) -> Result<Json<Value>> {
    let path = ctx.book_path(&book)?;
    if let Some(id) = &q.chapter_id {
        check_chapter(&ctx, &path, id).await?;
    }
    value(crate::translation_batch_preview(ctx.app.state(), path, q.chapter_id).await?)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchSubmit {
    chapter_id: Option<String>,
    pages: Vec<umanga_core::pipeline::PageVersion>,
    mode: umanga_core::pipeline::BatchMode,
}
async fn batch_submit(
    State(ctx): State<Arc<Context>>,
    RoutePath(book): RoutePath<String>,
    Json(input): Json<BatchSubmit>,
) -> Result<Json<Value>> {
    ids(&input.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>())?;
    let path = ctx.book_path(&book)?;
    if let Some(id) = &input.chapter_id {
        check_chapter(&ctx, &path, id).await?;
    }
    let app = ctx.app.clone();
    value(projection::jobs(
        &ctx,
        serde_json::to_value(
            durable(async move {
                crate::translation_batch_submit(
                    app.state(),
                    path,
                    book,
                    input.chapter_id,
                    input.pages,
                    input.mode,
                )
                .await
            })
            .await?,
        )
        .map_err(invalid)?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fresh {
    expected: u64,
    replace: bool,
}
async fn restart(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, page)): RoutePath<(String, String)>,
    Json(input): Json<Fresh>,
) -> Result<Json<Value>> {
    let (path, _) = trusted_page(&ctx, &book, &page).await?;
    let app = ctx.app.clone();
    value(projection::jobs(
        &ctx,
        serde_json::to_value(
            durable(async move {
                crate::restart_enqueue(app.state(), path, page, input.expected, input.replace).await
            })
            .await?,
        )
        .map_err(invalid)?,
    ))
}
async fn prepare(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, page)): RoutePath<(String, String)>,
    Json(input): Json<Fresh>,
) -> Result<Json<Value>> {
    let (path, _) = trusted_page(&ctx, &book, &page).await?;
    let app = ctx.app.clone();
    value(projection::jobs(
        &ctx,
        serde_json::to_value(
            durable(async move {
                crate::prepare_enqueue(app.state(), path, page, input.expected, input.replace).await
            })
            .await?,
        )
        .map_err(invalid)?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextEdit {
    expected: u64,
    source: String,
    target: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    expected: u64,
}
async fn edit_text(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, page, region)): RoutePath<(String, String, String)>,
    Json(input): Json<TextEdit>,
) -> Result<Json<Value>> {
    let (path, base) = trusted_page(&ctx, &book, &page).await?;
    if base.revision != input.expected {
        return Err(fail(
            StatusCode::CONFLICT,
            "Page changed; refresh and review your draft before saving.",
        ));
    }
    if region.is_empty() || region.len() > 256 {
        return Err(invalid("Invalid resource identity."));
    }
    let mut draft = base.clone();
    let r = draft
        .regions
        .iter_mut()
        .find(|r| r.id == region)
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "Region is no longer available."))?;
    umanga_core::safety::response_text(&input.source, &input.target).map_err(invalid)?;
    r.source = input.source;
    r.target = input.target;
    let app = ctx.app.clone();
    let p = durable(async move {
        crate::commit_edit(
            &app.state::<AppState>(),
            path,
            draft,
            base,
            input.expected,
            true,
        )
        .await
    })
    .await?;
    value(projection::page(p, &book))
}
async fn delete_region(
    State(ctx): State<Arc<Context>>,
    RoutePath((book, page, region)): RoutePath<(String, String, String)>,
    Json(input): Json<Delete>,
) -> Result<Json<Value>> {
    let (path, base) = trusted_page(&ctx, &book, &page).await?;
    if base.revision != input.expected {
        return Err(fail(
            StatusCode::CONFLICT,
            "Page changed; refresh and review your draft before saving.",
        ));
    }
    if region.is_empty() || region.len() > 256 {
        return Err(invalid("Invalid resource identity."));
    }
    let mut draft = base.clone();
    draft.regions.retain(|r| r.id != region);
    if draft.regions.len() == base.regions.len() {
        return Err(fail(
            StatusCode::NOT_FOUND,
            "Region is no longer available.",
        ));
    }
    let app = ctx.app.clone();
    let p = durable(async move {
        crate::commit_edit(
            &app.state::<AppState>(),
            path,
            draft,
            base,
            input.expected,
            true,
        )
        .await
    })
    .await?;
    value(projection::page(p, &book))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobsQuery {
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_size")]
    limit: usize,
}
fn page_size() -> usize {
    50
}
async fn jobs(State(ctx): State<Arc<Context>>, Query(q): Query<JobsQuery>) -> Result<Json<Value>> {
    if q.limit == 0 || q.limit > 100 {
        return Err(invalid("Jobs page size must be 1–100"));
    }
    let batch = crate::jobs_list(ctx.app.state()).await?;
    let mut v = projection::jobs(&ctx, serde_json::to_value(batch).map_err(invalid)?);
    projection::history(&mut v, q.offset, q.limit);
    value(v)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobIds {
    ids: Vec<String>,
}
async fn jobs_availability(
    State(ctx): State<Arc<Context>>,
    Json(input): Json<JobIds>,
) -> Result<Json<Value>> {
    ids(&input.ids)?;
    let state = ctx.app.state::<AppState>();
    for id in &input.ids {
        let j = state
            .engine
            .job(id)
            .ok_or_else(|| fail(StatusCode::NOT_FOUND, "Job not found"))?;
        crate::require_managed(&state, &j.project)?;
    }
    value(crate::jobs_readiness(state, input.ids).await?)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobControl {
    action: umanga_core::job_state::JobAction,
}
async fn control(
    State(ctx): State<Arc<Context>>,
    RoutePath(id): RoutePath<String>,
    Json(input): Json<JobControl>,
) -> Result<Json<Value>> {
    umanga_core::safety::identity(&id).map_err(invalid)?;
    let state = ctx.app.state::<AppState>();
    let j = state
        .engine
        .job(&id)
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "Job not found"))?;
    crate::require_managed(&state, &j.project)?;
    let app = ctx.app.clone();
    let v = projection::jobs(
        &ctx,
        serde_json::to_value(
            durable(async move { crate::job_control(app.state(), id, input.action).await }).await?,
        )
        .map_err(invalid)?,
    );
    value(v)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllControl {
    action: AllAction,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AllAction {
    Start,
    Stop,
}
async fn control_all(
    State(ctx): State<Arc<Context>>,
    Json(input): Json<AllControl>,
) -> Result<Json<Value>> {
    let app = ctx.app.clone();
    let mut v = projection::jobs(
        &ctx,
        serde_json::to_value(
            durable(async move {
                crate::jobs_control_all(
                    app.state(),
                    match input.action {
                        AllAction::Start => "start",
                        AllAction::Stop => "stop",
                    }
                    .into(),
                )
                .await
            })
            .await?,
        )
        .map_err(invalid)?,
    );
    projection::history(&mut v["state"], 0, 50);
    value(v)
}
async fn events(State(ctx): State<Arc<Context>>, headers: HeaderMap) -> Result<Response> {
    let token = security::cookie_token(&headers).ok_or_else(|| {
        fail(
            StatusCode::UNAUTHORIZED,
            "Sign in to U-Manga on this browser.",
        )
    })?;
    let permit = ctx.streams.clone().try_acquire_owned().map_err(|_| {
        fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many live connections. Close another U-Manga tab.",
        )
    })?;
    let rx = ctx.events.subscribe();
    let stream = futures_util::stream::unfold(
        (ctx, token, rx, permit, true),
        |(ctx, token, mut rx, permit, first)| async move {
            if ctx.stop.is_cancelled() || token.is_empty() {
                return None;
            }
            if first {
                return Some((
                    Ok::<_, Infallible>(
                        Event::default()
                            .event("update")
                            .data("{\"kind\":\"resync\"}"),
                    ),
                    (ctx, token, rx, permit, false),
                ));
            }
            let v = tokio::select! {
                _ = ctx.stop.cancelled() => return None,
                result = rx.recv() => match result { Ok(v)=>v, Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>json!({"kind":"resync"}), Err(_)=>return None },
                _ = tokio::time::sleep(Duration::from_secs(15)) => json!({"kind":"heartbeat"}),
            };
            if ctx.sessions.get(&token, Instant::now()).is_none() {
                return Some((
                    Ok(Event::default().event("expired").data("{}")),
                    (ctx, String::new(), rx, permit, false),
                ));
            }
            Some((
                Ok(Event::default().event("update").data(v.to_string())),
                (ctx, token, rx, permit, false),
            ))
        },
    );
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}
async fn static_asset(State(ctx): State<Arc<Context>>, request: Request) -> Response {
    if request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let key = if request.uri().path() == "/" {
        "index.html"
    } else {
        request.uri().path().trim_start_matches('/')
    };
    if !super::allowed_asset(key) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(bytes) = ctx.assets.get(key) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = match Path::new(key)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    };
    let mut r = ([(header::CONTENT_TYPE, mime)], bytes.to_vec()).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        if key == "index.html" {
            "no-cache"
        } else {
            "public, max-age=86400"
        }
        .parse()
        .unwrap(),
    );
    r
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unchanged_images_revalidate_without_a_body_and_changed_files_do_not() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test-output")
            .join(format!("http-image-{}.png", umanga_core::types::uid()));
        std::fs::write(&path, b"image fixture").unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let initial = stream_file(path.clone(), (), cancel.clone(), Some(3), HeaderMap::new())
            .await
            .unwrap_or_else(|_| panic!("initial image"));
        let etag = initial.headers()[header::ETAG].clone();
        assert_eq!(
            axum::body::to_bytes(initial.into_body(), 100)
                .await
                .unwrap(),
            "image fixture"
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag);
        let cached = stream_file(path.clone(), (), cancel.clone(), Some(3), headers.clone())
            .await
            .unwrap_or_else(|_| panic!("revalidation"));
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(cached.headers()[header::CACHE_CONTROL], "private, no-cache");
        assert!(
            axum::body::to_bytes(cached.into_body(), 100)
                .await
                .unwrap()
                .is_empty()
        );
        std::fs::write(&path, b"different image fixture").unwrap();
        let changed = stream_file(path.clone(), (), cancel.clone(), Some(3), headers.clone())
            .await
            .unwrap_or_else(|_| panic!("changed image"));
        assert_eq!(changed.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(changed.into_body(), 100)
                .await
                .unwrap(),
            "different image fixture"
        );
        let revision = stream_file(path.clone(), (), cancel, Some(4), headers)
            .await
            .unwrap_or_else(|_| panic!("changed revision"));
        assert_eq!(revision.status(), StatusCode::OK);
        drop(revision);
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn accepted_edits_finish_after_the_http_waiter_is_dropped() {
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let began = started.clone();
        let finish = release.clone();
        let waiter = tokio::spawn(async move {
            durable(async move {
                began.notify_one();
                finish.notified().await;
                let _ = done_tx.send(true);
                Ok(())
            })
            .await
        });
        started.notified().await;
        waiter.abort();
        release.notify_one();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), done_rx)
                .await
                .unwrap()
                .unwrap()
        );
    }
    #[test]
    fn remote_edits_accept_only_revision_and_text() {
        assert!(
            serde_json::from_value::<TextEdit>(
                json!({"expected":1,"source":"原文","target":"译文"})
            )
            .is_ok()
        );
        for field in [
            "bbox",
            "page",
            "path",
            "provider",
            "style",
            "background",
            "allowFill",
        ] {
            let mut v = json!({"expected":1,"source":"原文","target":"译文"});
            v[field] = json!("attack");
            assert!(serde_json::from_value::<TextEdit>(v).is_err());
        }
        assert!(serde_json::from_value::<Delete>(json!({"expected":1,"regions":[]})).is_err());
    }
}
