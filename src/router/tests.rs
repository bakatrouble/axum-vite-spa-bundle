use super::*;
use crate::config::{SpaConfig, dev_base};
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
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
