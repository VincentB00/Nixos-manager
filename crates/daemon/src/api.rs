use crate::{static_files, AppCtx};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use nixos_manager_core::types::*;
use nixos_manager_core::{catalog, channels, deploy, discover, disks, edit, flake, fsbrowse, fsops, nix, organize, remote, validate, CoreError};
use serde::Deserialize;
use std::path::PathBuf;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub struct ApiErr(CoreError);

impl From<CoreError> for ApiErr {
    fn from(e: CoreError) -> Self {
        ApiErr(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            CoreError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            CoreError::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid"),
            CoreError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            CoreError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            CoreError::Parse(_) => (StatusCode::UNPROCESSABLE_ENTITY, "parse"),
            CoreError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "io"),
            CoreError::Json(_) => (StatusCode::BAD_REQUEST, "json"),
            CoreError::Other(_) => (StatusCode::INTERNAL_SERVER_ERROR, "other"),
        };
        if status.is_server_error() {
            // tower_http's failure line has no path or message; make 500s diagnosable
            tracing::error!(code, %status, "request failed: {}", self.0);
        }
        (status, Json(ApiError { error: self.0.to_string(), code: code.to_string() })).into_response()
    }
}

/// Logs method + path for every 5xx so the console line is actionable.
async fn log_server_errors(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let res = next.run(req).await;
    if res.status().is_server_error() {
        tracing::error!(%method, %uri, status = %res.status(), "server error");
    }
    res
}

type ApiResult<T> = Result<Json<T>, ApiErr>;

pub fn router(ctx: AppCtx, cors_origin: Option<&str>) -> Router {
    let mut api = Router::new()
        .route("/health", get(health))
        .route("/state", get(get_state))
        .route("/state/forget", post(forget_location))
        .route("/fs/browse", get(fs_browse))
        .route("/fs/suggestions", get(fs_suggestions))
        .route("/location/open", post(open_location))
        .route("/location/hardware-sync", post(hardware_sync))
        .route("/search", get(search_files))
        .route("/location/start-from-system", post(start_from_system))
        .route("/location/host", post(set_host))
        .route("/deploy/status", get(build_status))
        .route("/packages/popular", get(packages_popular))
        .route("/system/store", get(system_store))
        .route("/system/rollback", post(system_rollback))
        .route("/system/gc", post(system_gc))
        .route("/system/other-systems", get(system_other))
        .route("/system/delete-paths", post(system_delete_paths))
        .route("/system/remote", get(system_remote))
        .route("/system/remote/other-systems", get(system_remote_other))
        .route("/system/remote/delete-paths", post(system_remote_delete_paths))
        .route("/system/remote/rollback", post(system_remote_rollback))
        .route("/system/remote/gc", post(system_remote_gc))
        .route("/git/init", post(git_init))
        .route("/git/discard", post(git_discard))
        .route("/git/info", get(git_info))
        .route("/git/branches", get(git_branches))
        .route("/git/diff", get(git_diff))
        .route("/git/suggest", get(git_suggest))
        .route("/git/switch", post(git_switch))
        .route("/git/branch", post(git_branch))
        .route("/git/commit", post(git_commit))
        .route("/git/push", post(git_push))
        .route("/git/fetch", post(git_fetch))
        .route("/git/pull", post(git_pull))
        .route("/git/identity", post(git_identity))
        .route("/git/remote", post(git_remote_add))
        .route("/remote/pull-hardware", post(hardware_pull))
        .route("/config/overview", get(config_overview))
        .route("/config/file", get(config_file))
        .route("/config/edit", post(config_edit))
        .route("/config/new-file", post(config_new_file))
        .route("/config/organize", post(config_organize))
        .route("/config/fs/mkdir", post(fs_mkdir))
        .route("/config/fs/rename", post(fs_rename))
        .route("/config/fs/duplicate", post(fs_duplicate))
        .route("/config/fs/delete", post(fs_delete))
        .route("/config/flake-hosts", get(flake_hosts))
        .route("/options/status", get(options_status))
        .route("/options/build", post(options_build))
        .route("/options/search", get(options_search))
        .route("/options/get", get(options_get))
        .route("/packages/search", get(packages_search))
        .route("/config/package-sets", get(package_sets))
        .route("/wizard/recipes", get(wizard_recipes))
        .route("/wizard/render", post(wizard_render))
        .route("/wizard/apply", post(wizard_apply))
        .route("/flake/info", get(flake_info))
        .route("/flake/edit", post(flake_edit))
        .route("/flake/convert", post(flake_convert))
        .route("/channels", get(channels_info))
        .route("/channels/action", post(channels_action))
        .route("/secrets/status", get(secrets_status))
        .route("/secrets/create", post(secrets_create))
        .route("/secrets/unlock", post(secrets_unlock))
        .route("/secrets/lock", post(secrets_lock))
        .route("/secrets/get", post(secrets_get))
        .route("/secrets/set", post(secrets_set))
        .route("/secrets/remove", post(secrets_remove))
        .route("/secrets/change-password", post(secrets_change_password))
        .route("/machine", get(machine))
        .route("/machine/disks", get(machine_disks))
        .route("/generations", get(generations))
        .route("/deploy/preflight", get(deploy_preflight))
        .route("/deploy", post(deploy_start))
        .route("/validate", post(validate_start))
        .route("/remote/targets", get(targets_get).put(targets_put))
        .route("/remote/test", post(remote_test))
        .route("/remote/check", post(remote_check))
        .route("/remote/root-key", post(remote_root_key))
        .route("/remote/setup-key", post(remote_setup_key))
        .route("/jobs", get(jobs_list))
        .route("/jobs/{id}", get(jobs_get))
        .route("/jobs/{id}/stop", post(jobs_stop))
        .route("/jobs/history", get(jobs_history).delete(jobs_history_clear))
        .route("/jobs/history/{name}", get(jobs_history_read).delete(jobs_history_delete))
        .route("/ws", get(ws_handler))
        .with_state(ctx);

    if let Some(origin) = cors_origin {
        let cors = CorsLayer::new()
            .allow_origin(origin.parse::<HeaderValue>().expect("valid CORS origin"))
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([axum::http::header::CONTENT_TYPE]);
        api = api.layer(cors);
    }

    Router::new()
        .nest("/api", api)
        .fallback(get(static_files::serve))
        .layer(axum::middleware::from_fn(log_server_errors))
        .layer(TraceLayer::new_for_http())
}

async fn health() -> &'static str {
    "ok"
}

async fn get_state(State(ctx): State<AppCtx>) -> ApiResult<AppState> {
    Ok(Json(ctx.store.get().await))
}

#[derive(Deserialize)]
struct PathBody {
    path: String,
}

async fn forget_location(State(ctx): State<AppCtx>, Json(body): Json<PathBody>) -> ApiResult<AppState> {
    Ok(Json(ctx.store.forget_location(&body.path).await?))
}

#[derive(Deserialize)]
struct PathQuery {
    path: Option<String>,
}

async fn fs_browse(Query(q): Query<PathQuery>) -> ApiResult<BrowseResult> {
    let path = q.path.filter(|p| !p.trim().is_empty()).unwrap_or_else(|| {
        fsbrowse::home_dir().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|| "/".into())
    });
    Ok(Json(fsbrowse::browse(&path)?))
}

async fn fs_suggestions() -> ApiResult<Vec<LocationSuggestion>> {
    let s = tokio::task::spawn_blocking(fsbrowse::suggestions).await.map_err(|e| CoreError::Other(e.to_string()))?;
    Ok(Json(s))
}

/// Copy this machine's hardware-configuration.nix into the working folder
/// (the system copy is the source of truth). Called by the UI on open.
async fn hardware_sync(State(ctx): State<AppCtx>, Json(body): Json<PathBody>) -> ApiResult<HardwareSync> {
    let path = root_of(&body.path)?;
    let pull = ctx.store.get().await.hardware_pulls.get(&path.to_string_lossy().to_string()).cloned();
    let mut r = nixos_manager_core::hardware::sync_from_system(&path, pull.as_ref());
    r.lock_removed = nixos_manager_core::flake::remove_workspace_lock(&path);
    Ok(Json(r))
}

/// Copy a deploy target's /etc/nixos/hardware-configuration.nix over the
/// working folder's file and remember it, so the sync-on-open keeps it.
async fn hardware_pull(State(ctx): State<AppCtx>, Json(req): Json<HardwarePullRequest>) -> ApiResult<HardwarePullResult> {
    let target = ctx.store.get().await.deploy_targets.into_iter().find(|t| t.name == req.target).ok_or_else(|| CoreError::NotFound(format!("target {}", req.target)))?;
    let path = root_of(&req.path)?;
    let (source, text) = remote::fetch_target_hardware(&target).await.map_err(CoreError::Invalid)?;
    let (rel, pull) = nixos_manager_core::hardware::install_pulled(&path, &target.name, &text).map_err(|e| CoreError::Invalid(format!("cannot write the file: {e}")))?;
    ctx.store.set_hardware_pull(&path.to_string_lossy(), pull).await?;
    Ok(Json(HardwarePullResult { target: target.name, source, rel_path: rel.to_string_lossy().to_string(), bytes: text.len() as u64 }))
}

#[derive(Deserialize)]
struct KeyAccessQuery {
    target: String,
}

#[derive(Deserialize)]
struct FindQuery {
    path: String,
    q: String,
    mode: Option<String>,
}

async fn search_files(Query(q): Query<FindQuery>) -> ApiResult<SearchResult> {
    let root = root_of(&q.path)?;
    let mode = q.mode.unwrap_or_else(|| "both".into());
    let r = tokio::task::spawn_blocking(move || nixos_manager_core::search::search(&root, &q.q, &mode)).await.map_err(|e| CoreError::Other(e.to_string()))?;
    Ok(Json(r))
}

async fn start_from_system(State(ctx): State<AppCtx>, Json(r): Json<StartFromSystemRequest>) -> ApiResult<LocationInfo> {
    let dest = fsbrowse::normalize_input(&r.dest);
    if !dest.is_absolute() {
        return Err(CoreError::Invalid("destination must be an absolute path".into()).into());
    }
    let d2 = dest.clone();
    tokio::task::spawn_blocking(move || fsbrowse::start_from_system(&d2)).await.map_err(|e| CoreError::Other(e.to_string()))??;
    if r.git_init {
        if let Err(e) = nixos_manager_core::git::init_repo(&dest).await {
            tracing::warn!("git init after start-from-system failed: {e}");
        }
    }
    let info = discover::location_info(&dest)?;
    ctx.store.touch_location(&info.path, info.hosts.first().cloned()).await?;
    Ok(Json(info))
}

async fn set_host(State(ctx): State<AppCtx>, Json(r): Json<SetHostRequest>) -> ApiResult<AppState> {
    let root = root_of(&r.path)?;
    Ok(Json(ctx.store.touch_location(&root.to_string_lossy(), r.host).await?))
}

async fn build_status(Query(q): Query<ConfigQuery>) -> ApiResult<BuildStatus> {
    let root = root_of(&q.path)?;
    Ok(Json(tokio::task::spawn_blocking(move || nixos_manager_core::jobs::JobManager::build_status(&root)).await.map_err(|e| CoreError::Other(e.to_string()))?))
}

async fn packages_popular() -> ApiResult<Vec<PopularPackage>> {
    Ok(Json(nixos_manager_core::packages::popular()))
}

async fn system_store() -> ApiResult<StoreUsage> {
    Ok(Json(nixos_manager_core::system::store_usage().await?))
}

async fn system_rollback(State(ctx): State<AppCtx>, Json(r): Json<RollbackRequest>) -> ApiResult<JobInfo> {
    if r.password.is_empty() && !deploy::running_as_root() {
        return Err(CoreError::Invalid("sudo password is required".into()).into());
    }
    let job = ctx.jobs.create("system", &format!("Roll back to generation {}", r.number)).await;
    let id = job.id.clone();
    tokio::spawn(nixos_manager_core::system::rollback(job, r.number, r.password));
    Ok(Json(ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?))
}

async fn system_gc(State(ctx): State<AppCtx>, Json(r): Json<GcRequest>) -> ApiResult<JobInfo> {
    if r.password.is_empty() && !deploy::running_as_root() {
        return Err(CoreError::Invalid("sudo password is required".into()).into());
    }
    let title = if r.keep_generations { "Collect garbage (keep all generations)".to_string() } else if r.older_than_days == 0 { "Collect garbage (delete all old generations)".to_string() } else { format!("Collect garbage (older than {} days)", r.older_than_days) };
    let job = ctx.jobs.create("system", &title).await;
    let id = job.id.clone();
    tokio::spawn(nixos_manager_core::system::collect_garbage(job, r.older_than_days, r.password, r.keep_generations));
    Ok(Json(ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?))
}

async fn git_discard(Json(r): Json<GitDiscardRequest>) -> ApiResult<GitInfo> {
    Ok(Json(nixos_manager_core::git::discard(&root_of(&r.path)?, r.files.as_deref(), r.clean_untracked).await?))
}

async fn system_other() -> ApiResult<Vec<OtherSystem>> {
    Ok(Json(nixos_manager_core::system::other_systems().await?))
}

async fn system_delete_paths(State(ctx): State<AppCtx>, Json(r): Json<DeletePathsRequest>) -> ApiResult<JobInfo> {
    if r.password.is_empty() && !deploy::running_as_root() {
        return Err(CoreError::Invalid("sudo password is required".into()).into());
    }
    if r.paths.is_empty() {
        return Err(CoreError::Invalid("no paths selected".into()).into());
    }
    let job = ctx.jobs.create("system", &format!("Delete {} build(s) for other machines from the store", r.paths.len())).await;
    let id = job.id.clone();
    tokio::spawn(nixos_manager_core::system::delete_paths(job, r.paths, r.password, r.gc_after));
    Ok(Json(ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?))
}

async fn system_remote_other(State(ctx): State<AppCtx>, Query(q): Query<KeyAccessQuery>) -> ApiResult<Vec<OtherSystem>> {
    let t = find_target(&ctx, &q.target).await?;
    Ok(Json(nixos_manager_core::system::remote_other_systems(&t).await?))
}

async fn system_remote_delete_paths(State(ctx): State<AppCtx>, Json(r): Json<RemoteDeletePathsRequest>) -> ApiResult<JobInfo> {
    let t = find_target(&ctx, &r.target).await?;
    if r.paths.is_empty() {
        return Err(CoreError::Invalid("no paths selected".into()).into());
    }
    let job = ctx.jobs.create("system", &format!("Delete {} build(s) for other machines on {}", r.paths.len(), t.name)).await;
    let id = job.id.clone();
    tokio::spawn(nixos_manager_core::system::remote_delete_paths(job, t, r.paths, r.password.unwrap_or_default(), r.gc_after));
    Ok(Json(ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?))
}

async fn find_target(ctx: &AppCtx, name: &str) -> Result<DeployTarget, CoreError> {
    ctx.store.get().await.deploy_targets.into_iter().find(|t| t.name == name).ok_or_else(|| CoreError::NotFound(format!("target {name}")))
}

async fn system_remote(State(ctx): State<AppCtx>, Query(q): Query<KeyAccessQuery>) -> ApiResult<RemoteSystemInfo> {
    let t = find_target(&ctx, &q.target).await?;
    Ok(Json(nixos_manager_core::system::remote_info(&t).await?))
}

async fn system_remote_rollback(State(ctx): State<AppCtx>, Json(r): Json<RemoteSystemRequest>) -> ApiResult<JobInfo> {
    let t = find_target(&ctx, &r.target).await?;
    let job = ctx.jobs.create("system", &format!("Roll back {} to generation {}", t.name, r.value)).await;
    let id = job.id.clone();
    tokio::spawn(nixos_manager_core::system::remote_rollback(job, t, r.value, r.password.unwrap_or_default()));
    Ok(Json(ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?))
}

async fn system_remote_gc(State(ctx): State<AppCtx>, Json(r): Json<RemoteSystemRequest>) -> ApiResult<JobInfo> {
    let t = find_target(&ctx, &r.target).await?;
    let title = if r.keep_generations { format!("Collect garbage on {} (keep all generations)", t.name) } else if r.value == 0 { format!("Collect garbage on {} (delete all old generations)", t.name) } else { format!("Collect garbage on {} (older than {} days)", t.name, r.value) };
    let job = ctx.jobs.create("system", &title).await;
    let id = job.id.clone();
    tokio::spawn(nixos_manager_core::system::remote_gc(job, t, r.value, r.password.unwrap_or_default(), r.keep_generations));
    Ok(Json(ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?))
}

async fn git_init(Json(r): Json<PathBody>) -> ApiResult<GitInfo> {
    Ok(Json(nixos_manager_core::git::init_repo(&root_of(&r.path)?).await?))
}

// ───────────────────────── git ─────────────────────────

async fn git_info(Query(q): Query<ConfigQuery>) -> ApiResult<GitInfo> {
    Ok(Json(nixos_manager_core::git::info(&root_of(&q.path)?).await?))
}
async fn git_branches(Query(q): Query<ConfigQuery>) -> ApiResult<Vec<GitBranch>> {
    Ok(Json(nixos_manager_core::git::branches(&root_of(&q.path)?).await?))
}
async fn git_diff(Query(q): Query<ConfigQuery>) -> ApiResult<String> {
    let file = q.file.ok_or_else(|| CoreError::Invalid("file is required".into()))?;
    Ok(Json(nixos_manager_core::git::diff(&root_of(&q.path)?, &file).await?))
}
async fn git_suggest(Query(q): Query<ConfigQuery>) -> ApiResult<GitSuggestion> {
    Ok(Json(nixos_manager_core::git::suggest_message(&root_of(&q.path)?).await?))
}
async fn git_switch(Json(r): Json<GitBranchRequest>) -> ApiResult<GitInfo> {
    Ok(Json(nixos_manager_core::git::switch(&root_of(&r.path)?, &r.name).await?))
}
async fn git_branch(Json(r): Json<GitBranchRequest>) -> ApiResult<GitInfo> {
    Ok(Json(nixos_manager_core::git::create_branch(&root_of(&r.path)?, &r.name).await?))
}
async fn git_commit(Json(r): Json<GitCommitRequest>) -> ApiResult<GitNetResult> {
    let root = root_of(&r.path)?;
    let info = nixos_manager_core::git::commit(&root, &r.message, r.paths.as_deref()).await?;
    if r.push {
        let output = nixos_manager_core::git::push(&root, r.password.as_deref()).await?;
        let info = nixos_manager_core::git::info(&root).await?;
        return Ok(Json(GitNetResult { output, info }));
    }
    Ok(Json(GitNetResult { output: String::new(), info }))
}
async fn git_push(Json(r): Json<GitNetRequest>) -> ApiResult<GitNetResult> {
    let root = root_of(&r.path)?;
    let output = nixos_manager_core::git::push(&root, r.password.as_deref()).await?;
    Ok(Json(GitNetResult { output, info: nixos_manager_core::git::info(&root).await? }))
}
async fn git_fetch(Json(r): Json<GitNetRequest>) -> ApiResult<GitNetResult> {
    let root = root_of(&r.path)?;
    let output = nixos_manager_core::git::fetch(&root, r.password.as_deref()).await?;
    Ok(Json(GitNetResult { output, info: nixos_manager_core::git::info(&root).await? }))
}
async fn git_pull(Json(r): Json<GitNetRequest>) -> ApiResult<GitNetResult> {
    let root = root_of(&r.path)?;
    let output = nixos_manager_core::git::pull(&root, r.password.as_deref()).await?;
    Ok(Json(GitNetResult { output, info: nixos_manager_core::git::info(&root).await? }))
}
async fn git_remote_add(Json(r): Json<GitRemoteRequest>) -> ApiResult<GitInfo> {
    Ok(Json(nixos_manager_core::git::add_remote(&root_of(&r.path)?, &r.name, &r.url).await?))
}
async fn git_identity(Json(r): Json<GitIdentityRequest>) -> ApiResult<GitInfo> {
    Ok(Json(nixos_manager_core::git::set_identity(&root_of(&r.path)?, &r.name, &r.email).await?))
}

async fn open_location(State(ctx): State<AppCtx>, Json(body): Json<PathBody>) -> ApiResult<LocationInfo> {
    let path = fsbrowse::normalize_input(&body.path);
    let info = discover::location_info(&path)?;
    if info.kind == ConfigKind::Unknown {
        return Err(CoreError::Invalid(format!("{} has neither flake.nix nor configuration.nix", path.display())).into());
    }
    ctx.store.touch_location(&info.path, info.hosts.first().cloned()).await?;
    Ok(Json(info))
}

#[derive(Deserialize)]
struct ConfigQuery {
    path: String,
    file: Option<String>,
}

/// Canonical, existing configuration folder (refuses anything else; never creates it).
fn root_of(q: &str) -> Result<PathBuf, CoreError> {
    fsbrowse::config_root(q)
}

async fn config_overview(Query(q): Query<ConfigQuery>) -> ApiResult<ConfigOverview> {
    let root = root_of(&q.path)?;
    Ok(Json(discover::overview(&root).await?))
}

async fn config_file(Query(q): Query<ConfigQuery>) -> ApiResult<NixFileView> {
    let root = root_of(&q.path)?;
    let file = q.file.ok_or_else(|| CoreError::Invalid("file is required".into()))?;
    Ok(Json(discover::read_file_view(&root, &file)?))
}

async fn config_edit(Json(req): Json<EditRequest>) -> ApiResult<NixFileView> {
    let root = root_of(&req.path)?;
    Ok(Json(edit::apply(&root, &req.file, &req.version, &req.op)?))
}

async fn config_organize(Json(req): Json<OrganizeRequest>) -> ApiResult<OrganizeResult> {
    let root = root_of(&req.path)?;
    let full = discover::resolve_inside(&root, &req.file)?;
    let source = std::fs::read_to_string(&full).map_err(|_| CoreError::NotFound(format!("{} not found", req.file)))?;
    if nixos_manager_core::nixparse::version_of(&source) != req.version {
        return Err(CoreError::Conflict(format!("{} changed on disk since it was loaded; reload and retry", req.file)).into());
    }
    let report = organize::organize(&source)?;
    let diff = similar::TextDiff::from_lines(&source, &report.source).unified_diff().context_radius(2).header("before", "after").to_string();
    let mut view = None;
    if !req.preview && report.changed {
        edit::write_atomic(&full, &report.source)?;
        view = Some(discover::read_file_view(&root, &req.file)?);
    }
    Ok(Json(OrganizeResult { changed: report.changed, groups_created: report.groups_created as u32, source: report.source, unified_diff: diff, view }))
}

async fn config_new_file(Json(req): Json<NewFileRequest>) -> ApiResult<NixFileView> {
    let root = root_of(&req.path)?;
    edit::create_file(&root, &req.file, &req.source, req.import_into.as_deref())?;
    Ok(Json(discover::read_file_view(&root, &req.file)?))
}

async fn fs_mkdir(Json(req): Json<FsRequest>) -> ApiResult<serde_json::Value> {
    fsops::mkdir(&root_of(&req.path)?, &req.target)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn fs_rename(Json(req): Json<FsRequest>) -> ApiResult<RenameResult> {
    let to = req.to.ok_or_else(|| CoreError::Invalid("'to' is required".into()))?;
    Ok(Json(fsops::rename(&root_of(&req.path)?, &req.target, &to, req.update_references.unwrap_or(true))?))
}

async fn fs_duplicate(Json(req): Json<FsRequest>) -> ApiResult<serde_json::Value> {
    let to = req.to.ok_or_else(|| CoreError::Invalid("'to' is required".into()))?;
    fsops::duplicate(&root_of(&req.path)?, &req.target, &to)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn fs_delete(Json(req): Json<FsRequest>) -> ApiResult<serde_json::Value> {
    let updated = fsops::delete(&root_of(&req.path)?, &req.target, req.recursive.unwrap_or(false), req.remove_references.unwrap_or(false))?;
    return Ok(Json(serde_json::json!({ "ok": true, "updated_files": updated })));
    #[allow(unreachable_code)]
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn flake_hosts(Query(q): Query<ConfigQuery>) -> ApiResult<Vec<String>> {
    let root = root_of(&q.path)?;
    Ok(Json(nix::flake_hosts(&root).await?))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    name: Option<String>,
    limit: Option<usize>,
    /// Package set name (pkgs, pkgs-25_11, ...) and the config path to resolve it.
    set: Option<String>,
    path: Option<String>,
}

async fn options_status(State(ctx): State<AppCtx>) -> ApiResult<OptionsStatus> {
    Ok(Json(ctx.options.status().await))
}

async fn options_build(State(ctx): State<AppCtx>) -> ApiResult<OptionsStatus> {
    if ctx.options.try_load_cached().await.unwrap_or(false) {
        return Ok(Json(ctx.options.status().await));
    }
    let st = ctx.options.status().await;
    if !st.building {
        let job = ctx.jobs.create("options-index", "Build NixOS option index").await;
        let opts = ctx.options.clone();
        tokio::spawn(async move { opts.build(job).await });
    }
    Ok(Json(ctx.options.status().await))
}

async fn options_search(State(ctx): State<AppCtx>, Query(q): Query<SearchQuery>) -> ApiResult<Vec<OptionDoc>> {
    Ok(Json(ctx.options.search(q.q.as_deref().unwrap_or(""), q.limit.unwrap_or(40).min(200)).await))
}

async fn options_get(State(ctx): State<AppCtx>, Query(q): Query<SearchQuery>) -> ApiResult<OptionDoc> {
    let name = q.name.ok_or_else(|| CoreError::Invalid("name is required".into()))?;
    ctx.options.get(&name).await.map(Json).ok_or_else(|| CoreError::NotFound(format!("option {name}")).into())
}

async fn packages_search(State(ctx): State<AppCtx>, Query(q): Query<SearchQuery>) -> ApiResult<PackageSearchResult> {
    let set = q.set.clone().unwrap_or_else(|| "pkgs".into());
    let installable = match &q.path {
        Some(p) => flake::package_sets(&root_of(p)?).into_iter().find(|s| s.name == set).and_then(|s| s.flake_ref),
        None => None,
    };
    Ok(Json(ctx.packages.search(q.q.as_deref().unwrap_or(""), installable.as_deref(), &set).await?))
}

async fn package_sets(Query(q): Query<ConfigQuery>) -> ApiResult<Vec<PackageSet>> {
    Ok(Json(flake::package_sets(&root_of(&q.path)?)))
}

async fn wizard_recipes() -> ApiResult<Vec<Recipe>> {
    Ok(Json(catalog::recipes()))
}

async fn wizard_render(Json(req): Json<RenderRequest>) -> ApiResult<RenderResult> {
    Ok(Json(catalog::render(&req.recipe_id, &req.values, req.package_set.as_deref())?))
}

async fn wizard_apply(Json(req): Json<ApplyChangesRequest>) -> ApiResult<NixFileView> {
    let root = root_of(&req.path)?;
    let full = discover::resolve_inside(&root, &req.file)?;
    let source = std::fs::read_to_string(&full).map_err(|_| CoreError::NotFound(format!("{} not found", req.file)))?;
    if nixos_manager_core::nixparse::version_of(&source) != req.version {
        return Err(CoreError::Conflict(format!("{} changed on disk since it was loaded; reload and retry", req.file)).into());
    }
    let new_source = edit::apply_changes(&source, &req.changes)?;
    edit::write_atomic(&full, &new_source)?;
    Ok(Json(discover::read_file_view(&root, &req.file)?))
}

async fn flake_info(Query(q): Query<ConfigQuery>) -> ApiResult<FlakeInfo> {
    Ok(Json(flake::info(&root_of(&q.path)?)?))
}

async fn flake_edit(Json(req): Json<FlakeEditRequest>) -> ApiResult<FlakeInfo> {
    let root = root_of(&req.path)?;
    let path = root.join("flake.nix");
    let source = std::fs::read_to_string(&path).map_err(|_| CoreError::NotFound("flake.nix not found".into()))?;
    if nixos_manager_core::nixparse::version_of(&source) != req.version {
        return Err(CoreError::Conflict("flake.nix changed on disk since it was loaded; reload and retry".into()).into());
    }
    let new_source = flake::apply(&source, &req.op)?;
    edit::write_atomic(&path, &new_source)?;
    Ok(Json(flake::info(&root)?))
}

async fn flake_convert(State(ctx): State<AppCtx>, Json(req): Json<ConvertToFlakeRequest>) -> ApiResult<FlakeInfo> {
    let root = root_of(&req.path)?;
    let username = req.username.clone().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "user".into()));
    let r = if req.nixpkgs_ref.trim().is_empty() { flake::DEFAULT_NIXPKGS_REF.to_string() } else { req.nixpkgs_ref.clone() };
    flake::convert(&root, &req.hostname, &username, &r)?;
    let info = discover::location_info(&root)?;
    ctx.store.touch_location(&info.path, info.hosts.first().cloned()).await?;
    Ok(Json(flake::info(&root)?))
}

async fn channels_info() -> ApiResult<ChannelsInfo> {
    Ok(Json(channels::info().await))
}

async fn channels_action(State(ctx): State<AppCtx>, Json(req): Json<ChannelActionRequest>) -> ApiResult<JobInfo> {
    let title = format!("nix-channel {:?} {}", req.action, req.name.clone().unwrap_or_default());
    let job = ctx.jobs.create("channels", &title).await;
    let id = job.id.clone();
    tokio::spawn(channels::run(job, req));
    let info = ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?;
    Ok(Json(info))
}

async fn secrets_status(State(ctx): State<AppCtx>, Query(q): Query<ConfigQuery>) -> ApiResult<SecretsStatus> {
    Ok(Json(ctx.secrets.status(&root_of(&q.path)?).await))
}
async fn secrets_create(State(ctx): State<AppCtx>, Json(r): Json<SecretsRequest>) -> ApiResult<SecretsStatus> {
    let pw = r.password.ok_or_else(|| CoreError::Invalid("password is required".into()))?;
    Ok(Json(ctx.secrets.create(&root_of(&r.path)?, &pw).await?))
}
async fn secrets_unlock(State(ctx): State<AppCtx>, Json(r): Json<SecretsRequest>) -> ApiResult<SecretsStatus> {
    let pw = r.password.ok_or_else(|| CoreError::Invalid("password is required".into()))?;
    Ok(Json(ctx.secrets.unlock(&root_of(&r.path)?, &pw).await?))
}
async fn secrets_lock(State(ctx): State<AppCtx>, Json(r): Json<SecretsRequest>) -> ApiResult<SecretsStatus> {
    Ok(Json(ctx.secrets.lock(&root_of(&r.path)?).await))
}
async fn secrets_get(State(ctx): State<AppCtx>, Json(r): Json<SecretsRequest>) -> ApiResult<SecretEntry> {
    let name = r.name.ok_or_else(|| CoreError::Invalid("name is required".into()))?;
    Ok(Json(ctx.secrets.get(&root_of(&r.path)?, &name).await?))
}
async fn secrets_set(State(ctx): State<AppCtx>, Json(r): Json<SecretsRequest>) -> ApiResult<SecretsStatus> {
    let entry = r.entry.ok_or_else(|| CoreError::Invalid("entry is required".into()))?;
    Ok(Json(ctx.secrets.set(&root_of(&r.path)?, entry).await?))
}
async fn secrets_remove(State(ctx): State<AppCtx>, Json(r): Json<SecretsRequest>) -> ApiResult<SecretsStatus> {
    let name = r.name.ok_or_else(|| CoreError::Invalid("name is required".into()))?;
    Ok(Json(ctx.secrets.remove(&root_of(&r.path)?, &name).await?))
}
async fn secrets_change_password(State(ctx): State<AppCtx>, Json(r): Json<SecretsRequest>) -> ApiResult<SecretsStatus> {
    let pw = r.password.ok_or_else(|| CoreError::Invalid("password is required".into()))?;
    Ok(Json(ctx.secrets.change_password(&root_of(&r.path)?, &pw).await?))
}

async fn machine_disks() -> ApiResult<Vec<DiskInfo>> {
    Ok(Json(disks::list().await?))
}

async fn machine() -> ApiResult<MachineInfo> {
    Ok(Json(nix::machine_info().await))
}

async fn generations() -> ApiResult<Vec<Generation>> {
    Ok(Json(nix::generations().await?))
}

async fn deploy_preflight(State(ctx): State<AppCtx>, Query(q): Query<ConfigQuery>) -> ApiResult<DeployPreflight> {
    let root = root_of(&q.path)?;
    let state = ctx.store.get().await;
    let pull = state.hardware_pulls.get(&root.to_string_lossy().to_string()).cloned();
    let mut p = deploy::preflight(&root, pull.as_ref()).await?;
    p.targets = state.deploy_targets;
    Ok(Json(p))
}

async fn targets_get(State(ctx): State<AppCtx>) -> ApiResult<Vec<DeployTarget>> {
    Ok(Json(ctx.store.get().await.deploy_targets))
}

async fn targets_put(State(ctx): State<AppCtx>, Json(targets): Json<Vec<DeployTarget>>) -> ApiResult<Vec<DeployTarget>> {
    for t in &targets {
        if t.name.trim().is_empty() || t.ssh_target.trim().is_empty() {
            return Err(CoreError::Invalid("each target needs a name and an SSH address".into()).into());
        }
    }
    Ok(Json(ctx.store.set_targets(targets).await?.deploy_targets))
}

async fn remote_check(State(ctx): State<AppCtx>, Json(req): Json<KeyAccessRequest>) -> ApiResult<RemoteCheck> {
    let target = ctx.store.get().await.deploy_targets.into_iter().find(|t| t.name == req.target).ok_or_else(|| CoreError::NotFound(format!("target {}", req.target)))?;
    Ok(Json(remote::check(&target).await))
}

async fn remote_test(State(ctx): State<AppCtx>, Json(req): Json<RemoteTestRequest>) -> ApiResult<JobInfo> {
    let job = ctx.jobs.create("remote-test", &format!("Test connection to {}", req.ssh_target)).await;
    let id = job.id.clone();
    tokio::spawn(remote::test_connection(job, req.ssh_target, req.port));
    let info = ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?;
    Ok(Json(info))
}

async fn remote_setup_key(State(ctx): State<AppCtx>, Json(req): Json<KeyAccessRequest>) -> ApiResult<JobInfo> {
    if req.password.is_empty() {
        return Err(CoreError::Invalid("the target's SSH password is required".into()).into());
    }
    let target = ctx.store.get().await.deploy_targets.into_iter().find(|t| t.name == req.target).ok_or_else(|| CoreError::NotFound(format!("target {}", req.target)))?;
    let job = ctx.jobs.create("setup-key", &format!("Set up key access to {}", target.name)).await;
    let id = job.id.clone();
    tokio::spawn(remote::setup_key_access(job, target, req.password));
    let info = ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?;
    Ok(Json(info))
}

async fn remote_root_key(State(ctx): State<AppCtx>, Json(req): Json<RootKeyRequest>) -> ApiResult<JobInfo> {
    if req.password.is_empty() && !deploy::running_as_root() {
        return Err(CoreError::Invalid("sudo password is required".into()).into());
    }
    let job = ctx.jobs.create("root-key", "Root SSH key for remote builders").await;
    let id = job.id.clone();
    tokio::spawn(remote::root_builder_key(job, req.password, req.builder_host));
    let info = ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?;
    Ok(Json(info))
}

async fn deploy_start(State(ctx): State<AppCtx>, Json(req): Json<DeployRequest>) -> ApiResult<JobInfo> {
    if req.password.is_empty() && req.target.is_none() && !deploy::running_as_root() {
        return Err(CoreError::Invalid("sudo password is required".into()).into());
    }
    let title = match &req.target {
        Some(t) if req.on_target => format!("nixos-rebuild {:?} on {} ({})", req.action, t.name, req.path),
        Some(t) => format!("nixos-rebuild {:?} → {} ({})", req.action, t.name, req.path),
        None => format!("nixos-rebuild {:?} ({})", req.action, req.path),
    };
    let job = ctx.jobs.create("deploy", &title).await;
    let id = job.id.clone();
    tokio::spawn(deploy::run(job, req));
    let info = ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?;
    Ok(Json(info))
}

async fn validate_start(State(ctx): State<AppCtx>, Json(req): Json<ValidateRequest>) -> ApiResult<JobInfo> {
    let root = root_of(&req.path)?;
    let job = ctx.jobs.create("validate", &format!("Validate {}", req.path)).await;
    let id = job.id.clone();
    tokio::spawn(validate::run(job, root, req.host));
    let info = ctx.jobs.get(&id).await.map(|d| d.info).ok_or_else(|| CoreError::Other("job vanished".into()))?;
    Ok(Json(info))
}

async fn jobs_list(State(ctx): State<AppCtx>) -> ApiResult<Vec<JobInfo>> {
    Ok(Json(ctx.jobs.list().await))
}

async fn jobs_history() -> ApiResult<Vec<JobLogFile>> {
    Ok(Json(nixos_manager_core::jobs::JobManager::history()))
}
async fn jobs_history_clear(State(ctx): State<AppCtx>) -> ApiResult<serde_json::Value> {
    let (deleted, bytes) = ctx.jobs.history_clear().await;
    Ok(Json(serde_json::json!({ "deleted": deleted, "bytes": bytes })))
}
async fn jobs_history_delete(State(ctx): State<AppCtx>, AxPath(name): AxPath<String>) -> ApiResult<serde_json::Value> {
    ctx.jobs.history_delete(&name).await?;
    Ok(Json(serde_json::json!({ "deleted": 1 })))
}
async fn jobs_history_read(AxPath(name): AxPath<String>) -> ApiResult<String> {
    Ok(Json(nixos_manager_core::jobs::JobManager::history_read(&name)?))
}

async fn jobs_stop(State(ctx): State<AppCtx>, AxPath(id): AxPath<String>) -> ApiResult<JobInfo> {
    Ok(Json(ctx.jobs.stop(&id).await?))
}

async fn jobs_get(State(ctx): State<AppCtx>, AxPath(id): AxPath<String>) -> ApiResult<JobDetail> {
    ctx.jobs.get(&id).await.map(Json).ok_or_else(|| CoreError::NotFound(format!("job {id}")).into())
}

async fn ws_handler(ws: WebSocketUpgrade, State(ctx): State<AppCtx>) -> Response {
    ws.on_upgrade(move |socket| ws_loop(socket, ctx))
}

async fn ws_loop(mut socket: WebSocket, ctx: AppCtx) {
    let mut rx = ctx.jobs.subscribe();
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(event) => {
                    let text = match serde_json::to_string(&event) { Ok(t) => t, Err(_) => continue };
                    if socket.send(Message::Text(text.into())).await.is_err() { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(Message::Ping(p))) => { let _ = socket.send(Message::Pong(p)).await; }
                Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}
