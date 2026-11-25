# axum-vite-spa-bundle

[![crates.io](https://img.shields.io/crates/v/axum-vite-spa-bundle.svg)](https://crates.io/crates/axum-vite-spa-bundle)
[![docs.rs](https://img.shields.io/docsrs/axum-vite-spa-bundle)](https://docs.rs/axum-vite-spa-bundle)

Serve a Vite single-page app from an [axum](https://crates.io/crates/axum)
server:

- **Release builds** serve a bundle embedded in the binary (`rust-embed`),
  with `index.html` as the SPA fallback.
- **Debug builds** reverse-proxy every non-API request to the locally
  running Vite dev server, including HMR websockets. Frontend edits need no
  Rust rebuild and no `dist` build.

## Usage

```rust,ignore
use axum::Router;
use axum_vite_spa_bundle::{SpaConfig, spa_router};

#[derive(Clone, Copy, rust_embed::Embed)]
#[folder = "web/dist/"]
struct WebDist;

fn app() -> Router {
    let api: Router = Router::new(); // your /api routes
    api.merge(spa_router(SpaConfig::new(
        WebDist,
        "WEB_DEV_URL",        // env var holding the Vite base URL (debug)
        "http://127.0.0.1:5173", // default when the env var is unset
        ["/api/", "/auth/"],  // prefixes that always answer 404 locally
    )))
}
```

Merge the SPA router **last**, after the API routers, so API routes keep
priority.

## Behaviour

| Request                         | Release                          | Debug                          |
|---------------------------------|----------------------------------|--------------------------------|
| path under any `api_prefixes`   | 404 (never `index.html`)         | 404 (never proxied)            |
| exact file in the bundle        | served                           | proxied to the dev server      |
| `dir/`                          | `dir/index.html`                 | proxied                        |
| unknown non-API path            | `index.html` with 200            | proxied (Vite handles it)      |

`dev_base` trims a trailing `/` from the env value, so path joins never
double a slash.

## Precompressed assets

In release builds, a `<path>.gz` sibling next to any served file (`index.html`
included) is served instead when the request's `Accept-Encoding` lists
`gzip`, with its own `Content-Encoding: gzip` and ETag. This crate does not
compress anything itself: write the `.gz` files into the embedded folder at
build time (a `vite build` plugin, a `build.rs` step, or any other script),
before `rust-embed` reads the folder. `.br`/`.zz` siblings are not
negotiated.

## Requirements

- `rust-embed` must point at a folder that exists at compile time in
  release builds (for example `web/dist/` after `vite build`).
- The debug proxy installs a `rustls` crypto provider on first use (`ring`).
  If your application installs its own default provider, the call is a
  harmless no-op.

## Status

Early release. The API may change before 1.0.

## License

MIT. See the LICENSE file in the repository.

## Example

`examples/vite-app` is a small Vite + TypeScript app.
`examples/serve.rs` embeds its `dist/` in release builds:

```bash
cd examples/vite-app && bun install && bun run build && cd ../..
cargo run --release --example serve     # http://127.0.0.1:3000
```
