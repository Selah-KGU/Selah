import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const releaseTargets = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
];

const exceptions = [
  { name: "proc-macro-error", version: "1.0.4", id: "RUSTSEC-2024-0370" },
  { name: "glib", version: "0.18.5", id: "RUSTSEC-2024-0429" },
  { name: "zvariant", version: "5.15.0", id: "RUSTSEC-2026-0370" },
  { name: "zbus_names", version: "4.3.4", id: "RUSTSEC-2026-0371" },
  { name: "zvariant_derive", version: "5.15.0", id: "RUSTSEC-2026-0372" },
];

export function checkAuditScope({ lockfile, config, workflowSources, trees }) {
  const configured = [...(config.match(/ignore\s*=\s*\[([^\]]*)\]/)?.[1] ?? "")
    .matchAll(/"(RUSTSEC-[^"]+)"/g)].map(match => match[1]).sort();
  if (JSON.stringify(configured) !== JSON.stringify(exceptions.map(item => item.id).sort())) {
    throw new Error("Audit exceptions must match the reviewed Linux-only advisories.");
  }

  const packages = lockfile.split(/^\[\[package\]\]\s*$/m).map(block => ({
    name: block.match(/^name = "([^"]+)"/m)?.[1],
    version: block.match(/^version = "([^"]+)"/m)?.[1],
    source: block.match(/^source = "([^"]+)"/m)?.[1],
  }));
  for (const exception of exceptions) {
    const found = packages.filter(item => item.name === exception.name);
    if (found.length !== 1 || found[0].version !== exception.version ||
        found[0].source !== "registry+https://github.com/rust-lang/crates.io-index") {
      throw new Error(`Reassess or remove ${exception.id}: ${exception.name} changed in Cargo.lock.`);
    }
  }

  for (const source of workflowSources) {
    for (const match of source.matchAll(/(?:\btargets:\s*|--target(?:=|\s+))([\w.,-]+)/g)) {
      for (const target of match[1].split(",")) {
        if (!releaseTargets.includes(target) && target !== "universal-apple-darwin") {
          throw new Error(`Review the Linux audit exceptions before adding release target ${target}.`);
        }
      }
    }
  }

  for (const target of releaseTargets) {
    const tree = trees.get(target);
    if (!tree || !/^selah-app v/m.test(tree)) {
      throw new Error(`Missing release dependency graph for ${target}.`);
    }
    for (const exception of exceptions) {
      if (tree.split(/\r?\n/).some(line => line.startsWith(`${exception.name} v`))) {
        throw new Error(`${exception.name} is reachable on ${target}; ${exception.id} cannot be excepted.`);
      }
    }
  }
}

function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const backend = join(root, "src-tauri");
  const trees = new Map();
  for (const target of releaseTargets) {
    const result = spawnSync("cargo", [
      "tree", "--locked", "--all-features", "--target", target,
      "--edges", "normal,build,dev", "--prefix", "none", "--format", "{p}",
    ], { cwd: backend, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(result.stderr || `cargo tree failed for ${target}.`);
    trees.set(target, result.stdout);
  }
  const workflows = join(root, ".github", "workflows");
  checkAuditScope({
    lockfile: readFileSync(join(backend, "Cargo.lock"), "utf8"),
    config: readFileSync(join(backend, ".cargo", "audit.toml"), "utf8"),
    workflowSources: readdirSync(workflows).filter(name => /\.ya?ml$/.test(name))
      .map(name => readFileSync(join(workflows, name), "utf8")),
    trees,
  });
  console.log("Verified Linux-only audit exceptions are absent from all four release dependency graphs.");
  const audit = spawnSync("cargo", ["audit", "--deny", "warnings"], { cwd: backend, stdio: "inherit" });
  if (audit.error) throw audit.error;
  process.exitCode = audit.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
