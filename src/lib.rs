//! Generic single-page-app bundle serving.
//!
//! Release builds serve an embedded `dist` bundle through [`axum_embed`]:
//! exact matches, `dir/` → `dir/index.html`, and unknown non-API paths fall
//! back to `index.html` with 200 (standard SPA behavior). Debug builds
//! forward to the locally running Vite dev server (HTTP + HMR websockets)
//! through [`axum_reverse_proxy`].
//!
//! Mount with [`Router::fallback`] semantics: merge the returned router LAST,
//! after the API routers, so existing API routes keep priority. Any request
//! whose path is under any of the [`SpaConfig::api_prefixes`] gets a local
//! 404 in both modes — never `index.html`, never the proxy.

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::IntoResponse;

/// Configuration for [`spa_router`].
#[derive(Debug, Clone)]
pub struct SpaConfig<E> {
    /// Embedded `dist` bundle type: a unit struct, used in release builds.
    pub bundle: E,
    /// Env var holding the dev-server base URL (debug builds only).
    pub dev_url_env: String,
    /// Default dev-server base URL when the env var is unset (debug only).
    pub dev_url_default: String,
    /// Path prefixes reserved for the caller's APIs; never served or proxied.
    pub api_prefixes: Vec<String>,
}

impl<E: Default> Default for SpaConfig<E> {
    fn default() -> Self {
        Self {
            bundle: E::default(),
            dev_url_env: "WEB_DEV_URL".to_string(),
            dev_url_default: "http://127.0.0.1:5173".to_string(),
            api_prefixes: vec!["/api/".to_string()],
        }
    }
}

impl<E> SpaConfig<E> {
    /// Build a config explicitly. `api_prefixes` may hold any number of
    /// prefixes, for example `["/api/", "/auth/"]`.
    pub fn new<I, P>(
        bundle: E,
        dev_url_env: impl Into<String>,
        dev_url_default: impl Into<String>,
        api_prefixes: I,
    ) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<String>,
    {
        Self {
            bundle,
            dev_url_env: dev_url_env.into(),
            dev_url_default: dev_url_default.into(),
            api_prefixes: api_prefixes.into_iter().map(Into::into).collect(),
        }
    }
}

/// Resolve the dev-server base URL: `$env_var`, or `default` when unset.
/// A trailing `/` is trimmed so path joins never duplicate a slash.
pub fn dev_base(env_var: &str, default: &str) -> String {
    let base = std::env::var(env_var).unwrap_or_else(|_| default.to_string());
    base.trim_end_matches('/').to_string()
}

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
    use axum_embed::{FallbackBehavior, ServeEmbed};

    let embed = ServeEmbed::<E>::with_parameters(
        Some("index.html".to_string()),
        FallbackBehavior::Ok,
        Some("index.html".to_string()),
    );
    let api_prefixes: Arc<[String]> = config.api_prefixes.into();
    Router::new().fallback(move |req: Request| {
        let embed = embed.clone();
        let api_prefixes = Arc::clone(&api_prefixes);
        async move {
            if is_api_path(req.uri().path(), &api_prefixes) {
                return StatusCode::NOT_FOUND.into_response();
            }
            match tower::ServiceExt::oneshot(embed, req).await {
                Ok(res) => res.into_response(),
                Err(never) => match never {},
            }
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
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt as _;

    #[derive(Clone, rust_embed::Embed)]
    #[folder = "src/"]
    struct TestDist;

    fn test_config() -> SpaConfig<TestDist> {
        // Port 9 (discard) refuses connections: the proxy fails with 502,
        // which distinguishes a proxied miss from the guard's local 404.
        SpaConfig::new(
            TestDist,
            "WEB_BUNDLE_TEST_DEV_URL",
            "http://127.0.0.1:9",
            ["/api/"],
        )
    }

    async fn get_status(app: Router, path: &str) -> StatusCode {
        let req = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("test request builds");
        app.oneshot(req).await.expect("oneshot").status()
    }

    #[tokio::test]
    async fn api_prefix_guard_returns_local_404() {
        let app = spa_router::<_, ()>(test_config());
        assert_eq!(get_status(app, "/api/nope").await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_prefix_guard_covers_bare_prefix() {
        let app = spa_router::<_, ()>(test_config());
        assert_eq!(get_status(app, "/api").await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn every_configured_prefix_is_guarded() {
        let config = SpaConfig::new(
            TestDist,
            "WEB_BUNDLE_TEST_DEV_URL",
            "http://127.0.0.1:9",
            ["/api/", "/auth/"],
        );
        let app = spa_router::<_, ()>(config.clone());
        assert_eq!(get_status(app, "/auth/login").await, StatusCode::NOT_FOUND);
        let app = spa_router::<_, ()>(config.clone());
        assert_eq!(get_status(app, "/api/nope").await, StatusCode::NOT_FOUND);
        let app = spa_router::<_, ()>(config);
        assert_eq!(get_status(app, "/authx").await, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn non_api_paths_reach_the_proxy() {
        // Nothing listens on port 9, so proxied requests fail with 502 —
        // proving the guard only intercepts the API prefix.
        let app = spa_router::<_, ()>(test_config());
        assert_eq!(get_status(app, "/").await, StatusCode::BAD_GATEWAY);
        let app = spa_router::<_, ()>(test_config());
        assert_eq!(get_status(app, "/apix").await, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn dev_base_env_override_and_default() {
        let env = "WEB_BUNDLE_TEST_DEV_BASE";
        unsafe { std::env::remove_var(env) };
        assert_eq!(
            dev_base(env, "http://127.0.0.1:5173"),
            "http://127.0.0.1:5173"
        );
        unsafe { std::env::set_var(env, "http://127.0.0.1:9999") };
        assert_eq!(
            dev_base(env, "http://127.0.0.1:5173"),
            "http://127.0.0.1:9999"
        );
        unsafe { std::env::remove_var(env) };
    }

    #[test]
    fn dev_base_trims_trailing_slash() {
        let env = "WEB_BUNDLE_TEST_DEV_TRIM";
        unsafe { std::env::remove_var(env) };
        assert_eq!(
            dev_base(env, "http://127.0.0.1:5173///"),
            "http://127.0.0.1:5173"
        );
        unsafe { std::env::set_var(env, "http://127.0.0.1:9999/") };
        assert_eq!(dev_base(env, "http://x"), "http://127.0.0.1:9999");
        unsafe { std::env::remove_var(env) };
    }
}
