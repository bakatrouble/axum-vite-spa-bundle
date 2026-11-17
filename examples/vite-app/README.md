# vite-app

A small Vite + TypeScript app used by `examples/serve.rs`.

```bash
cd examples/vite-app
bun install
bun run build                      # writes dist/, which the Rust example embeds
cd ../..
cargo run --example serve          # http://127.0.0.1:3000
```

For frontend work with hot reload, run the dev server and the Rust example
in debug mode (the debug build proxies to Vite):

```bash
cd examples/vite-app && bun run dev    # http://127.0.0.1:5173
cd ../.. && cargo run --example serve  # http://127.0.0.1:3000 -> Vite
```

`dist/` and `node_modules/` are build output and are not committed.
