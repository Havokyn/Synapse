import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// In production the bundle is embedded under `/dashboard/` and served by the
// Synapse daemon (see crates/synapse-mcp/src/http/transport.rs). For local
// development we serve the app shell from the dev root (`/`) and proxy the
// daemon's data + command endpoints to the running daemon on :7700, so the
// dashboard can be iterated live against real state without rebuilding and
// redeploying the daemon binary. Override the daemon origin with
// SYNAPSE_DAEMON_ORIGIN if it is bound elsewhere.
const DAEMON_ORIGIN = process.env.SYNAPSE_DAEMON_ORIGIN ?? "http://127.0.0.1:7700";
// Daemon-owned paths the SPA fetches at runtime (everything else — the app
// shell, /assets, /src, HMR — is served by vite itself).
const DAEMON_PROXY_PREFIXES = ["/dashboard", "/events", "/agent-events", "/health"];

export default defineConfig(({ command }) => ({
  // Dev serves from root so daemon `/dashboard/*` API paths don't collide with
  // the SPA's own `/dashboard/` base; the production build keeps `/dashboard/`.
  base: command === "serve" ? "/" : "/dashboard/",
  plugins: [react(), tailwindcss()],
  server: {
    host: "127.0.0.1",
    proxy: Object.fromEntries(
      DAEMON_PROXY_PREFIXES.map((prefix) => [
        prefix,
        { target: DAEMON_ORIGIN, changeOrigin: true }
      ])
    )
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src")
    }
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    cssCodeSplit: false,
    assetsInlineLimit: 1048576,
    rollupOptions: {
      output: {
        entryFileNames: "assets/dashboard-[hash].js",
        chunkFileNames: "assets/dashboard-[name]-[hash].js",
        assetFileNames: (assetInfo) => {
          if (assetInfo.names?.some((name) => name.endsWith(".css"))) {
            return "assets/dashboard-[hash].css";
          }
          return "assets/[name]-[hash][extname]";
        }
      }
    }
  }
}));
