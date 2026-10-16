//! Nixos-manager daemon: serves the Angular UI and the JSON/WebSocket API.

mod api;
mod static_files;

use clap::Parser;
use nixos_manager_core::jobs::JobManager;
use nixos_manager_core::options::OptionsIndex;
use nixos_manager_core::packages::PackageSearch;
use nixos_manager_core::secrets::SecretsVault;
use nixos_manager_core::state::StateStore;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "nixos-manager-daemon", about = "Nixos-manager daemon (API + UI)")]
struct Args {
    /// Address to listen on. Keep it on localhost unless you add TLS in front:
    /// the sudo password travels over this connection.
    #[arg(long, env = "NIXOS_MANAGER_LISTEN", default_value = "127.0.0.1:7788")]
    listen: SocketAddr,
    /// Path of the JSON state file.
    #[arg(long, env = "NIXOS_MANAGER_STATE")]
    state: Option<std::path::PathBuf>,
    /// Allow cross-origin requests from this origin (for `ng serve` on :4200).
    #[arg(long, env = "NIXOS_MANAGER_CORS_ORIGIN")]
    cors_origin: Option<String>,
}

#[derive(Clone)]
pub struct AppCtx {
    pub store: Arc<StateStore>,
    pub jobs: JobManager,
    pub options: OptionsIndex,
    pub packages: PackageSearch,
    pub secrets: SecretsVault,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into()))
        .init();
    let args = Args::parse();
    let state_path = args.state.unwrap_or_else(StateStore::default_path);
    let store = Arc::new(StateStore::load(state_path.clone())?);
    tracing::info!("state file: {}", state_path.display());
    let ctx = AppCtx { store, jobs: JobManager::new(), options: OptionsIndex::default(), packages: PackageSearch::default(), secrets: SecretsVault::default() };
    // Load the option docs in the background if nix already has them.
    {
        let opts = ctx.options.clone();
        tokio::spawn(async move {
            match opts.try_load_cached().await {
                Ok(true) => tracing::info!("option index loaded ({} options)", opts.status().await.count),
                Ok(false) => tracing::info!("option index not cached yet; it can be built from the UI"),
                Err(e) => tracing::warn!("option index: {e}"),
            }
        });
    }

    let app = api::router(ctx, args.cors_origin.as_deref());
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!("listening on http://{}", args.listen);
    axum::serve(listener, app).await?;
    Ok(())
}
