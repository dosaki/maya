import { defineConfig, searchForWorkspaceRoot } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// The phone's page: the same src/ modules behind mobile/web/index.html,
// served on 1430 for `tauri android dev` and built to dist-mobile/.
export default defineConfig(() => ({
  root: "mobile/web",
  publicDir: false,
  clearScreen: false,
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
