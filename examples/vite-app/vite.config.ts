import { defineConfig } from 'vite'

// Dev server on 5173 is the default WEB_DEV_URL the Rust example proxies to.
export default defineConfig({
  server: {
    port: 5173,
    strictPort: true,
  },
})
