// Mock of the controller's HTTP API for frontend development, normally
// started/stopped together with `vite dev` (see vite.config.js). Can also
// run standalone:  node mock/server.mjs [port]
//
// The response shape is specified in PROTOCOL.md ("HTTP API"); the firmware
// produces it in http_server.c, and the mock in simulation.mjs. Keep all three
// in step.
import { realpathSync } from "node:fs";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";

import { STATUS_PATH } from "../src/lib/constants.js";
import {
  DEFAULT_MOCK_PORT,
  HTTP_METHOD_NOT_ALLOWED,
  HTTP_NOT_FOUND,
  HTTP_OK,
  MOCK_HOST,
} from "./constants.mjs";
import { createSimulation } from "./simulation.mjs";

export function createMockServer(port) {
  const simulation = createSimulation();

  const server = createServer((req, res) => {
    // The dev page runs on the vite origin, so the mock must allow CORS.
    const headers = {
      "Content-Type": "application/json",
      "Access-Control-Allow-Origin": "*",
    };
    if (req.url === STATUS_PATH && req.method !== "GET") {
      // As the firmware answers: the path exists, but only for GET.
      res.writeHead(HTTP_METHOD_NOT_ALLOWED, { ...headers, Allow: "GET" });
      res.end(JSON.stringify({ error: "method not allowed" }));
    } else if (req.url === STATUS_PATH) {
      res.writeHead(HTTP_OK, headers);
      res.end(JSON.stringify(simulation.status()));
    } else {
      res.writeHead(HTTP_NOT_FOUND, headers);
      res.end(JSON.stringify({ error: "not found" }));
    }
  });

  const stop = simulation.stop;
  server.on("close", stop);
  // Without a listener an EADDRINUSE is an unhandled 'error' event, which takes
  // down the whole process: the Vite dev server, when vite.config.js started us.
  server.on("error", (err) => {
    stop();
    console.error(`  ✗  mock controller API: cannot listen on ${MOCK_HOST}:${port}: ${err.message}`);
  });
  server.listen(port, MOCK_HOST, () => {
    // The real port, so port 0 (any free port, as the tests use) reads right.
    const { port: actual } = server.address();
    console.log(`  ➜  mock controller API: http://${MOCK_HOST}:${actual}${STATUS_PATH}`);
  });
  return server;
}

// Run directly rather than imported. import.meta.main needs Node 24.2; the
// fallback compares real paths, which a symlinked checkout or a path needing
// URL escaping cannot fool, unlike comparing against a hand-built file:// URL.
const runDirectly =
  import.meta.main ??
  (process.argv[1] !== undefined &&
    realpathSync(process.argv[1]) === fileURLToPath(import.meta.url));

if (runDirectly) {
  createMockServer(Number(process.argv[2] ?? DEFAULT_MOCK_PORT)).on("error", () => {
    process.exitCode = 1;
  });
}
