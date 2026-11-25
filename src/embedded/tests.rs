use super::*;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::Response;

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

fn request_with_encoding(
    method: Method,
    path: &str,
    accept_encoding: Option<&str>,
    if_none_match: Option<&str>,
) -> Request {
    let mut builder = axum::http::Request::builder().method(method).uri(path);
    if let Some(value) = accept_encoding {
        builder = builder.header(header::ACCEPT_ENCODING, value);
    }
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

#[tokio::test]
async fn gzip_accepted_serves_the_sibling_with_content_encoding() {
    let name = first_asset(".js");
    let res = serve_embedded::<Dist>(&request_with_encoding(
        Method::GET,
        &format!("/{name}"),
        Some("gzip, deflate, br"),
        None,
    ));
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_of(&res, header::CONTENT_ENCODING), Some("gzip"));
    assert!(
        header_of(&res, header::CONTENT_TYPE).is_some_and(|value| value.contains("javascript")),
        "content type {:?}",
        header_of(&res, header::CONTENT_TYPE)
    );
    let expected = Dist::get(&format!("{name}.gz"))
        .expect("the fixture has a .gz sibling")
        .data
        .into_owned();
    assert_eq!(body_of(res).await, expected);
}

#[tokio::test]
async fn gzip_not_accepted_serves_the_plain_file() {
    let name = first_asset(".js");
    let res = serve_embedded::<Dist>(&request(Method::GET, &format!("/{name}"), None));
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_of(&res, header::CONTENT_ENCODING), None);
    let expected = Dist::get(&name).expect("asset exists").data.into_owned();
    assert_eq!(body_of(res).await, expected);
}

#[tokio::test]
async fn gzip_and_plain_variants_have_different_etags() {
    let name = first_asset(".css");
    let plain = serve_embedded::<Dist>(&request(Method::GET, &format!("/{name}"), None));
    let gzip = serve_embedded::<Dist>(&request_with_encoding(
        Method::GET,
        &format!("/{name}"),
        Some("gzip"),
        None,
    ));
    let plain_etag = header_of(&plain, header::ETAG).expect("plain etag");
    let gzip_etag = header_of(&gzip, header::ETAG).expect("gzip etag");
    assert_ne!(plain_etag, gzip_etag);
}

#[tokio::test]
async fn matching_if_none_match_gets_304_for_the_gzip_variant() {
    let name = first_asset(".css");
    let first = serve_embedded::<Dist>(&request_with_encoding(
        Method::GET,
        &format!("/{name}"),
        Some("gzip"),
        None,
    ));
    let etag = header_of(&first, header::ETAG)
        .expect("etag set")
        .to_owned();
    let res = serve_embedded::<Dist>(&request_with_encoding(
        Method::GET,
        &format!("/{name}"),
        Some("gzip"),
        Some(&format!("\"{etag}\"")),
    ));
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn the_fallback_document_is_also_gzip_negotiated() {
    let res = serve_embedded::<Dist>(&request_with_encoding(
        Method::GET,
        "/some/client/route",
        Some("gzip"),
        None,
    ));
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_of(&res, header::CONTENT_ENCODING), Some("gzip"));
    let expected = Dist::get(&format!("{INDEX}.gz"))
        .expect("the fixture has an index.html.gz sibling")
        .data
        .into_owned();
    assert_eq!(body_of(res).await, expected);
}
