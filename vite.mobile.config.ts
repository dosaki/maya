import { defineConfig, searchForWorkspaceRoot, type Plugin } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// The phone's page at /: mobile/web/index.html, whose links climb to src/.
const phonePage: Plugin = {
  name: "phone-page",
  configureServer(server) {
    server.middlewares.use((req, _res, next) => {
      if (req.url === "/" || req.url === "/index.html") req.url = "/mobile/web/index.html";
      next();
    });
  },
};

// The phone's page: the same src/ modules behind mobile/web/index.html,
// served on 1430 for `tauri android dev` and built to dist-mobile/.
// The dev server's root is the repository: served from mobile/web, the
// page's ../../src/ links would climb no higher than / and ask for /src/,
// which is not under that root, and every one would get the page back.
export default defineConfig(({ command }) => ({
  root: command === "serve" ? "." : "mobile/web",
  publicDir: false,
  clearScreen: false,
  plugins: command === "serve" ? [phonePage] : [],
  build: { outDir: "../../dist-mobile", emptyOutDir: true },
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
    fs: { allow: [searchForWorkspaceRoot(process.cwd())] },
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/core/**", "**/cli/**", "**/hook/**", "**/vendor/**", "**/mobile/gen/**"] },
  },
}));
