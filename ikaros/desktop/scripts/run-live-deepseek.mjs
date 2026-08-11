import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";

const keyFile = process.env.IKAROS_LIVE_DEEPSEEK_KEY_FILE;
if (!keyFile) {
  process.stderr.write("IKAROS_LIVE_DEEPSEEK_KEY_FILE is required.\n");
  process.exit(2);
}

let source;
try {
  source = readFileSync(keyFile, "utf8").replace(/^\uFEFF/u, "");
} catch {
  process.stderr.write("The DeepSeek credential file could not be read.\n");
  process.exit(2);
}

const credential = source.endsWith("\r\n")
  ? source.slice(0, -2)
  : source.endsWith("\n")
    ? source.slice(0, -1)
    : source;
if (
  credential.length === 0 ||
  credential !== credential.trim() ||
  /[\u0000-\u001f\u007f]/u.test(credential)
) {
  process.stderr.write("The DeepSeek credential file has an invalid format.\n");
  process.exit(2);
}

const vitestCli = join(process.cwd(), "node_modules", "vitest", "vitest.mjs");
const completed = spawnSync(
  process.execPath,
  [vitestCli, "run", "src/renderer/runtimeStore.live.test.ts"],
  {
    cwd: process.cwd(),
    env: { ...process.env, IKAROS_LIVE_DEEPSEEK_SMOKE: "1" },
    encoding: "buffer",
    maxBuffer: 16 * 1024 * 1024,
    timeout: 360_000,
    windowsHide: true,
  },
);
const stdout = completed.stdout ?? Buffer.alloc(0);
const stderr = completed.stderr ?? Buffer.alloc(0);
const secret = Buffer.from(credential, "utf8");
if (stdout.indexOf(secret) >= 0 || stderr.indexOf(secret) >= 0) {
  process.stderr.write("LIVE_SMOKE_OUTPUT_SECRET_SCAN=FAIL\n");
  process.exit(97);
}

process.stdout.write("LIVE_SMOKE_OUTPUT_SECRET_SCAN=PASS\n");
process.stdout.write(stdout);
process.stderr.write(stderr);
if (completed.error) {
  process.stderr.write("The live smoke runner failed to execute or timed out.\n");
  process.exit(completed.error.code === "ETIMEDOUT" ? 124 : 1);
}
process.exit(completed.status ?? 1);
