import { defineConfig, loadEnv } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { viteSingleFile } from "vite-plugin-singlefile";
import { createMockServer } from "./mock/server.mjs";

// Runs the mock controller API (mock/server.mjs) for the lifetime of the
// vite dev server. Its port comes from VITE_API_BASE in .env.development,
// so the page and the mock can never disagree. Never active in builds.
function mockControllerApi(apiBase) {
  return {
    name: "mock-controller-api",
    apply: "serve",
    configureServer(server) {
      if (!apiBase?.startsWith("http")) {
        return; // relative API base in dev means "no mock" (e.g. a proxy setup)
      }
      const mock = createMockServer(Number(new URL(apiBase).port));
      server.httpServer?.once("close", () => mock.close());
    },
  };
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  return {
    plugins: [svelte(), viteSingleFile(), mockControllerApi(env.VITE_API_BASE)],
  };
});
