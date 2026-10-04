// Gzip the single-file Vite build output and place it where the firmware
// build embeds it (EMBED_FILES in firmware/main/CMakeLists.txt).
// gzip rather than br/zstd: browsers only accept those over HTTPS, and the
// device serves plain HTTP on the LAN.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync, constants } from "node:zlib";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const source = join(root, "dist", "index.html");
const target = join(root, "..", "firmware", "main", "web", "index.html.gz");

const html = readFileSync(source);

// The device serves this one file as the whole site, so check before embedding
// that it really is self-contained, and that it talks to the device it is
// served from. A development-mode build points at the mock on localhost, and a
// flashed page that does so shows "unreachable" forever with nothing amiss.
const text = html.toString("utf8");
const problems = [];
if (/<script\b[^>]*\bsrc\s*=/i.test(text)) {
  problems.push("a <script src> was left as a separate file instead of inlined");
}
if (/<link\b[^>]*\bstylesheet\b[^>]*\bhref\s*=\s*["']?(?!data:)/i.test(text)) {
  problems.push("a stylesheet was left as a separate file instead of inlined");
}
if (/localhost|127\.0\.0\.1/.test(text)) {
  problems.push("it refers to localhost, so it was not built for production (vite build --mode development?)");
}
if (problems.length > 0) {
  console.error(`refusing to embed ${source}:`);
  for (const problem of problems) console.error(`  - ${problem}`);
  process.exit(1);
}

const gz = gzipSync(html, { level: constants.Z_BEST_COMPRESSION });
mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, gz);

console.log(
  `embedded: ${source} (${html.length} B) -> ${target} (${gz.length} B gzipped)`
);
