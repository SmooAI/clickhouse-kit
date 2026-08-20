#!/usr/bin/env node
// Lockstep versioning: package.json is the single source of truth, and every
// other version-bearing file in the repo is written from it.
//
// Run as part of the changesets `version` lifecycle (see package.json) so the
// synced manifests are committed WITH the version bump — the failure mode this
// exists to prevent is syncing after `changeset publish`, which leaves the
// release commit/tag carrying stale version constants.
//
// `--check` asserts instead of writing, and exits non-zero on any mismatch.
// That is the CI guard; it must fail, never warn.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const check = process.argv.includes("--check");

const version = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")).version;

/**
 * Every version-bearing file, as {file, regex}. The regex captures the version
 * in group 1 and must be anchored tightly enough to hit only the intended line
 * — a new manifest that needs syncing gets a row here, and the `--check` guard
 * covers it for free.
 */
const targets = [
  {
    file: "crates/clickhouse-kit/Cargo.toml",
    // The [package] version — the first `version = "x"` in the file.
    re: /^version = "([^"]+)"$/m,
  },
  {
    file: "crates/clickhouse-kit/Cargo.lock",
    // The crate's own [[package]] entry in its lockfile.
    re: /(?<=name = "smooai-clickhouse-kit"\nversion = ")([^"]+)(?=")/,
  },
];

let failed = false;

for (const { file, re } of targets) {
  const abs = path.join(root, file);
  const text = readFileSync(abs, "utf8");
  const match = text.match(re);
  if (!match) {
    console.error(`✗ ${file}: no version found (pattern ${re})`);
    failed = true;
    continue;
  }
  const found = match[1] ?? match[0];
  if (found === version) continue;

  if (check) {
    console.error(`✗ ${file}: version ${found} !== package.json ${version}`);
    failed = true;
  } else {
    writeFileSync(
      abs,
      text.replace(re, (whole, captured) => whole.replace(captured ?? whole, version)),
    );
    console.log(`→ ${file}: ${found} → ${version}`);
  }
}

if (failed) {
  console.error(
    check
      ? `\nVersions are out of lockstep. Run \`node scripts/sync-versions.mjs\` and commit the result.`
      : `\nSync failed.`,
  );
  process.exit(1);
}

console.log(check ? `✓ all manifests at ${version}` : `✓ synced to ${version}`);
