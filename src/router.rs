//! The SPA router: release serves the embedded bundle, debug proxies the Vite
//! dev server. API prefixes are guarded in both.

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::config::SpaConfig;
#[cfg(debug_assertions)]
use crate::config::dev_base;
#[cfg(not(debug_assertions))]
use crate::embedded::serve_embedded;

/// True when `path` belongs to any API namespace: exactly a prefix (sans
/// trailing `/`) or anything under it. `/apix` is not under `/api/`.
fn is_api_path(path: &str, api_prefixes: &[String]) -> bool {
    api_prefixes.iter().any(|prefix| {
        let stem = prefix.trim_end_matches('/');
        !stem.is_empty() && (path == stem || path.starts_with(&format!("{stem}/")))
    })
}

/// SPA router: embedded bundle in release, dev-server proxy in debug.
///
/// `E` is the embedded `dist` type taken from [`SpaConfig::bundle`] (used in
/// release builds only); `S` is the caller state, inferred from the router it
/// is merged into.
/// Merge the result LAST so the caller's API routes keep priority; the
/// [`SpaConfig::api_prefixes`] guard lives in the fallback, so unmatched API
/// paths stay 404. Do not mount with `nest_service` at `/` (axum 0.8 rejects
/// root nesting) — the returned router is already fallback-style.
pub fn spa_router<E, S>(config: SpaConfig<E>) -> Router<S>
where
    E: rust_embed::RustEmbed + Clone + Send + Sync + 'static,
    S: Clone + Send + Sync + 'static,
{
    #[cfg(not(debug_assertions))]
    {
        release_router::<E, S>(config)
    }
    #[cfg(debug_assertions)]
    {
        debug_router::<E, S>(&config)
    }
}

/// Embedded bundle with `index.html` fallback served as 200.
#[cfg(not(debug_assertions))]
fn release_router<E, S>(config: SpaConfig<E>) -> Router<S>
where
    E: rust_embed::RustEmbed + Clone + Send + Sync + 'static,
    S: Clone + Send + Sync + 'static,
{
    let api_prefixes: Arc<[String]> = config.api_prefixes.into();
    Router::new().fallback(move |req: Request| {
        let api_prefixes = Arc::clone(&api_prefixes);
        async move {
            if is_api_path(req.uri().path(), &api_prefixes) {
                return StatusCode::NOT_FOUND.into_response();
            }
            serve_embedded::<E>(&req)
        }
    })
}

/// Reverse proxy to the locally running Vite dev server (HTTP + HMR).
#[cfg(debug_assertions)]
fn debug_router<E, S>(config: &SpaConfig<E>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // The dependency tree enables both rustls providers (ring and aws-lc-rs),
    // so hyper-rustls cannot pick one itself and `ReverseProxy::new` would
    // panic. Install ring as the process default first, as the rustls docs
    // prescribe; a no-op error when something installed one already.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let base = dev_base(&config.dev_url_env, &config.dev_url_default);
    // Root-level fallback: an empty match path forwards the full request
    // path (the proxy joins the remainder onto the target base).
    let proxy = axum_reverse_proxy::ReverseProxy::new("", base.as_str());
    let api_prefixes: Arc<[String]> = config.api_prefixes.clone().into();
    Router::new().fallback(move |req: Request| {
        let proxy = proxy.clone();
        let api_prefixes = Arc::clone(&api_prefixes);
        async move {
            if is_api_path(req.uri().path(), &api_prefixes) {
                return StatusCode::NOT_FOUND.into_response();
            }
            proxy.proxy_request(req).await.into_response()
        }
    })
}

#[cfg(test)]
mod tests;
