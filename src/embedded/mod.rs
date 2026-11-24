//! Serving the embedded `dist` bundle (release builds).

use std::time::{Duration, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

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
pub(crate) fn serve_embedded<E: rust_embed::RustEmbed>(req: &Request) -> Response {
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

#[cfg(test)]
mod tests;
