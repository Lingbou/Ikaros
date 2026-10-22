#!/usr/bin/env node
/**
 * Builds the Runtime wheel that ships inside the desktop installer and writes
 * build/runtime/runtime-bundle.json for scripts/install-runtime.{ps1,sh}.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(desktopRoot, "../..");
const runtimeRoot = join(repoRoot, "runtime");
const outputDir = join(desktopRoot, "build/runtime");

rmSync(outputDir, { recursive: true, force: true });
mkdirSync(outputDir, { recursive: true });

execFileSync("uv", ["build", "--project", runtimeRoot, "--wheel", "--out-dir", outputDir], {
  stdio: "inherit"
});

const wheel = readdirSync(outputDir).find((name) => /^ikaros_runtime-.*\.whl$/.test(name));
if (!wheel) {
  throw new Error(`uv build produced no ikaros_runtime wheel in ${outputDir}.`);
}

const protocol = JSON.parse(
  readFileSync(join(runtimeRoot, "protocol/runtime-protocol.json"), "utf8")
);
const version = /^ikaros_runtime-(.+?)-py3-none-any\.whl$/.exec(wheel)?.[1];
if (!version) {
  throw new Error(`Cannot read the Runtime version from ${wheel}.`);
}

writeFileSync(
  join(outputDir, "runtime-bundle.json"),
  `${JSON.stringify(
    { schemaVersion: 1, version, protocolVersion: protocol.protocolVersion, wheel },
    null,
    2
  )}\n`,
  "utf8"
);
console.log(`Prepared Runtime bundle ${version} (protocol ${protocol.protocolVersion}) -> ${outputDir}`);
