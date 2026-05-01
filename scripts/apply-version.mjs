#!/usr/bin/env node
// Selah version helper.
//
// Format: MAJOR.MINOR.PATCH where PATCH = (UTC year - 2020) * 1000 + day_of_year (UTC).
// Example: 2026-05-01 -> patch = 6 * 1000 + 121 = 6121, version = 1.0.6121.
//
// Constraints behind the format:
// - Strict semver (3 parts) so npm / Cargo / Tauri all accept it.
// - Each segment <= 65535 so MSIX (Microsoft Store) accepts it. The patch
//   formula stays under 65535 until 2086.
// - Patch is monotonically increasing across years, so the Tauri updater's
//   semver comparison keeps pointing at newer builds.
//
// Usage:
//   node scripts/apply-version.mjs              # print computed version (no writes)
//   node scripts/apply-version.mjs <version>    # write the given semver to all files
//   node scripts/apply-version.mjs --base 1.1   # compute against an explicit major.minor

import fs from "node:fs";

const PKG_PATH = "package.json";
const LOCK_PATH = "package-lock.json";
const TAURI_PATH = "src-tauri/tauri.conf.json";
const CARGO_TOML_PATH = "src-tauri/Cargo.toml";
const CARGO_LOCK_PATH = "src-tauri/Cargo.lock";

function computePatch() {
  const now = new Date();
  const year = now.getUTCFullYear();
  const start = Date.UTC(year, 0, 1);
  const doy = Math.floor((now.getTime() - start) / 86400000) + 1;
  return (year - 2020) * 1000 + doy;
}

function readBase() {
  const pkg = JSON.parse(fs.readFileSync(PKG_PATH, "utf8"));
  return pkg.version.split(".").slice(0, 2).join(".");
}

function writeJsonVersion(path, mutate) {
  const obj = JSON.parse(fs.readFileSync(path, "utf8"));
  mutate(obj);
  fs.writeFileSync(path, JSON.stringify(obj, null, 2) + "\n");
}

function writePackageVersionInToml(path, version) {
  const lines = fs.readFileSync(path, "utf8").split("\n");
  let inPackage = false;
  let updated = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith("[package]")) inPackage = true;
    else if (line.startsWith("[")) inPackage = false;
    if (inPackage && !updated && /^version\s*=\s*"[^"]*"/.test(line)) {
      lines[i] = `version = "${version}"`;
      updated = true;
    }
  }
  if (!updated) {
    throw new Error(`Could not find [package].version in ${path}`);
  }
  fs.writeFileSync(path, lines.join("\n"));
}

function writeSelahAppVersionInLock(path, version) {
  if (!fs.existsSync(path)) return;
  const text = fs.readFileSync(path, "utf8");
  const re = /(name\s*=\s*"selah-app"\s*\nversion\s*=\s*")[^"]*(")/;
  if (!re.test(text)) {
    throw new Error(`Could not find selah-app entry in ${path}`);
  }
  fs.writeFileSync(path, text.replace(re, `$1${version}$2`));
}

function applyVersion(version) {
  writeJsonVersion(PKG_PATH, (p) => {
    p.version = version;
  });
  writeJsonVersion(LOCK_PATH, (p) => {
    p.version = version;
    if (p.packages && p.packages[""]) p.packages[""].version = version;
  });
  writeJsonVersion(TAURI_PATH, (p) => {
    p.version = version;
  });
  writePackageVersionInToml(CARGO_TOML_PATH, version);
  writeSelahAppVersionInLock(CARGO_LOCK_PATH, version);
}

const args = process.argv.slice(2);
let baseOverride = null;
let explicit = null;
for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === "--base") {
    baseOverride = args[++i];
  } else if (arg.startsWith("--")) {
    console.error(`Unknown flag: ${arg}`);
    process.exit(1);
  } else if (explicit === null) {
    explicit = arg;
  } else {
    console.error(`Unexpected argument: ${arg}`);
    process.exit(1);
  }
}

if (explicit) {
  if (!/^\d+\.\d+\.\d+$/.test(explicit)) {
    console.error(`Invalid semver: ${explicit}`);
    process.exit(1);
  }
  applyVersion(explicit);
  console.log(explicit);
} else {
  const base = baseOverride ?? readBase();
  if (!/^\d+\.\d+$/.test(base)) {
    console.error(`Invalid major.minor: ${base}`);
    process.exit(1);
  }
  console.log(`${base}.${computePatch()}`);
}
