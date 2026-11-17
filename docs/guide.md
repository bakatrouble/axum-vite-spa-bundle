# Guide

## Layout

Your project has two parts that the crate joins:

- the Vite app, built to a folder (usually `web/dist/`) for release builds;
- the axum app, which mounts the SPA router after its API routes.

```text
web/              Vite app (vite build -> web/dist)
src/main.rs       axum app: API routes, then spa_router
```

## Step 1: embed the bundle

```rust
#[derive(Clone, Copy, rust_embed::Embed)]
#[folder = "web/dist/"]
struct WebDist;
```

`rust-embed` reads the folder when the crate compiles in release mode, so
`vite build` must run first. Debug builds read nothing from it.

## Step 2: mount the router last

```rust
let app = api_routes()
    .merge(axum_vite_spa_bundle::spa_router(SpaConfig::new(
        WebDist,
        "WEB_DEV_URL",
        "http://127.0.0.1:5173",
        ["/api/"],
    )));
```

- Merge after every API router. The SPA router only answers paths the API
  does not match.
- `api_prefixes` (any number) are a local 404 in both modes. List every prefix your API uses, for example `["/api/", "/auth/"]`.

## Step 3: develop

```bash
cd web && bun run dev -- --port 5173    # Vite dev server
cargo run                              # debug: SPA proxied to WEB_DEV_URL
```

Set `WEB_DEV_URL` when Vite runs on another port. The value may include a
trailing `/`; it is trimmed.

## Step 4: release

```bash
cd web && bun run build                # writes web/dist
cargo build --release                  # embeds web/dist
```

## Troubleshooting

| Symptom                                        | Cause and fix                                                                 |
|------------------------------------------------|-------------------------------------------------------------------------------|
| `502` on non-API paths in debug                | Vite is not running, or `WEB_DEV_URL` points at the wrong port.              |
| `404` on a deep link in release                | Build did not run before `cargo build --release`; the bundle is empty.        |
| Blank page, Vite HMR errors in debug           | The proxy does not reach the HMR socket; check the Vite port and the env var. |
| Panic at first request in debug about crypto   | Two rustls providers are enabled; the crate installs `ring` on first use.     |
| API route returns the SPA HTML                 | The SPA router was merged before the API router. Merge it last.              |
