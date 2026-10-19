import { defineConfig } from "vitest/config";
// styles.css is kept (not blanked) so a test can read the stylesheet's source.
export default defineConfig({ test: { environment: "jsdom", include: ["src/**/*.test.ts"], css: { include: [/styles\.css/] } } });
