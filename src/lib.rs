#![doc = include_str!("../README.md")]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

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

/// Name of the bundle's entry document, served for `/` and for unknown paths.
const INDEX: &str = "index.html";

/// Serve a GET or HEAD request from the embedded bundle `E`.
///
/// An exact file is served with its content type, an ETag (the file's
/// SHA-256 in hex), and `Last-Modified` when the bundle records one; a
/// matching `If-None-Match` gets `304`. The root and a trailing-slash path
/// map to `index.html`. Any other unknown path gets `index.html` with `200`
/// (SPA fallback). Other methods get `405`. The bundle's precompressed
/// variants (`.br`, `.gz`, `.zz`) are not negotiated.
///
/// Compiled in debug builds too, so it is tested with the same code the
/// release router uses.
#[cfg_attr(debug_assertions, allow(dead_code))]
fn serve_embedded<E: rust_embed::RustEmbed>(req: &Request) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        let mut res = Response::new(Body::from("Method not allowed"));
        *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        res.headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        return res;
    }

    let requested = req.uri().path().trim_start_matches('/');
    let wanted = if requested.is_empty() {
        INDEX.to_owned()
    } else if requested.ends_with('/') {
        format!("{requested}{INDEX}")
    } else {
        requested.to_owned()
    };
    let (name, file, is_fallback) = match E::get(&wanted) {
        Some(file) => (wanted, file, false),
        None => match E::get(INDEX) {
            Some(file) => (INDEX.to_owned(), file, true),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };

    let digest = file.metadata.sha256_hash();
    let etag = hex(&digest);
    if !is_fallback && if_none_match(req) == Some(etag.as_str()) {
        let mut res = Response::new(Body::empty());
        *res.status_mut() = StatusCode::NOT_MODIFIED;
        return res;
    }

    let mut res = Response::new(Body::from(file.data.into_owned()));
    *res.status_mut() = StatusCode::OK;
    let headers = res.headers_mut();
    let content_type = mime_guess::from_path(&name)
        .first_or_octet_stream()
        .to_string();
    insert_header(headers, header::CONTENT_TYPE, &content_type);
    insert_header(headers, header::ETAG, &etag);
    if let Some(modified) = file.metadata.last_modified() {
        let when = UNIX_EPOCH + Duration::from_secs(modified);
        insert_header(
            headers,
            header::LAST_MODIFIED,
            &httpdate::fmt_http_date(when),
        );
    }
    res
}

/// The `If-None-Match` value without quotes, when the header is present.
fn if_none_match(req: &Request) -> Option<&str> {
    req.headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_matches('"'))
}

/// Set a header from a value this module built; a value that cannot be a
/// header is skipped rather than failing the response.
fn insert_header(headers: &mut axum::http::HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

/// Lowercase hex of a 32-byte digest.
fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    digest
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
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

    #[cfg(debug_assertions)]
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

    #[cfg(debug_assertions)]
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

#[cfg(test)]
mod serve_tests {
    use super::*;
    use axum::body::to_bytes;

    #[derive(rust_embed::Embed)]
    #[folder = "examples/vite-app/dist/"]
    struct Dist;

    fn request(method: Method, path: &str, if_none_match: Option<&str>) -> Request {
        let mut builder = axum::http::Request::builder().method(method).uri(path);
        if let Some(value) = if_none_match {
            builder = builder.header(header::IF_NONE_MATCH, value);
        }
        builder.body(Body::empty()).expect("test request builds")
    }

    async fn body_of(res: Response) -> Vec<u8> {
        to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("body reads")
            .to_vec()
    }

    fn header_of(res: &Response, name: header::HeaderName) -> Option<&str> {
        res.headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    fn first_asset(suffix: &str) -> String {
        Dist::iter()
            .map(|name| name.to_string())
            .find(|name| name.starts_with("assets/") && name.ends_with(suffix))
            .expect("bundle has an asset with this suffix")
    }

    #[tokio::test]
    async fn root_serves_index_html() {
        let res = serve_embedded::<Dist>(&request(Method::GET, "/", None));
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(header_of(&res, header::CONTENT_TYPE), Some("text/html"));
        let expected = Dist::get(INDEX)
            .expect("bundle has index.html")
            .data
            .into_owned();
        assert_eq!(body_of(res).await, expected);
    }

    #[tokio::test]
    async fn asset_gets_its_content_type_and_a_sha256_etag() {
        let name = first_asset(".js");
        let res = serve_embedded::<Dist>(&request(Method::GET, &format!("/{name}"), None));
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            header_of(&res, header::CONTENT_TYPE).is_some_and(|value| value.contains("javascript")),
            "content type {:?}",
            header_of(&res, header::CONTENT_TYPE)
        );
        let etag = header_of(&res, header::ETAG).expect("etag set").to_owned();
        assert_eq!(etag.len(), 64);
        assert!(etag.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let expected = Dist::get(&name).expect("asset exists").data.into_owned();
        assert_eq!(body_of(res).await, expected);
    }

    #[tokio::test]
    async fn matching_if_none_match_gets_304() {
        let name = first_asset(".css");
        let first = serve_embedded::<Dist>(&request(Method::GET, &format!("/{name}"), None));
        let etag = header_of(&first, header::ETAG)
            .expect("etag set")
            .to_owned();
        let res = serve_embedded::<Dist>(&request(
            Method::GET,
            &format!("/{name}"),
            Some(&format!("\"{etag}\"")),
        ));
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
        assert!(body_of(res).await.is_empty());
    }

    #[tokio::test]
    async fn other_if_none_match_gets_the_file() {
        let name = first_asset(".css");
        let res = serve_embedded::<Dist>(&request(
            Method::GET,
            &format!("/{name}"),
            Some("\"not-the-etag\""),
        ));
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_paths_fall_back_to_index_with_200() {
        for path in ["/some/client/route", "/assets/missing.js", "/assets/"] {
            let res = serve_embedded::<Dist>(&request(Method::GET, path, None));
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert!(
                header_of(&res, header::ETAG).is_some(),
                "{path} has an etag"
            );
            let expected = Dist::get(INDEX).expect("index").data.into_owned();
            assert_eq!(body_of(res).await, expected, "{path}");
        }
    }

    #[tokio::test]
    async fn fallback_ignores_if_none_match() {
        // A fallback response is the index document, not a validated file.
        let etag = header_of(
            &serve_embedded::<Dist>(&request(Method::GET, "/", None)),
            header::ETAG,
        )
        .expect("etag set")
        .to_owned();
        let res = serve_embedded::<Dist>(&request(Method::GET, "/client/route", Some(&etag)));
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn head_is_served_like_get() {
        let res = serve_embedded::<Dist>(&request(Method::HEAD, "/", None));
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn other_methods_get_405() {
        let res = serve_embedded::<Dist>(&request(Method::POST, "/", None));
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(header_of(&res, header::CONTENT_TYPE), Some("text/plain"));
    }
}
