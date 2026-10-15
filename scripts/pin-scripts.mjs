// Pins install scripts: downloads every `type: script` URL in manifest/tools.yaml (with curl, exactly like
// DevSetup does at install time) and records its SHA-256 in the manifest.
//   npm run pin          write new or changed hashes
//   npm run pin:check    only report scripts that differ from their pin (exit 1), for CI
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";

const run = promisify(execFile);
const MANIFEST = fileURLToPath(new URL("../manifest/tools.yaml", import.meta.url));
const RE = /(\{ type: script, url: "([^"]+)")(, sha256: "[0-9a-f]{64}")?/;

export const scriptUrl = line => RE.exec(line)?.[2] ?? null;
export const pinnedHash = line => /sha256: "([0-9a-f]{64})"/.exec(line)?.[1] ?? null;
export const pinLine = (line, hash) => line.replace(RE, (_m, head) => `${head}, sha256: "${hash}"`);

async function sha256Of(url) {
  const { stdout } = await run("curl", ["--proto", "=https", "--tlsv1.2", "-fsSL", url], { encoding: "buffer", maxBuffer: 32 * 1024 * 1024 });
  return createHash("sha256").update(stdout).digest("hex");
}

async function main() {
  const check = process.argv.includes("--check");
  const lines = readFileSync(MANIFEST, "utf8").split("\n");
  let changed = 0, failed = 0;
  for (let i = 0; i < lines.length; i++) {
    const url = scriptUrl(lines[i]);
    if (!url) continue;
    try {
      const hash = await sha256Of(url), old = pinnedHash(lines[i]);
      if (old === hash) { console.log(`ok       ${url}`); continue; }
      console.log(`${old ? "CHANGED " : "new     "} ${url}  ${hash}`);
      changed++;
      if (!check) lines[i] = pinLine(lines[i], hash);
    } catch (e) { console.error(`FAILED   ${url}: ${e.message.split("\n")[0]}`); failed++; }
  }
  if (!check && changed) writeFileSync(MANIFEST, lines.join("\n"));
  console.log(check ? `${changed} script(s) differ from their pin` : `${changed} pin(s) written`);
  process.exit(failed || (check && changed) ? 1 : 0);
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
