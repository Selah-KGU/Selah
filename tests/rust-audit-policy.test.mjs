import assert from "node:assert/strict";
import test from "node:test";
import { checkAuditScope, releaseTargets } from "../scripts/audit-rust-dependencies.mjs";

function fixture() {
  return {
    lockfile: [
      ["glib", "0.18.5"], ["proc-macro-error", "1.0.4"],
      ["zvariant", "5.15.0"], ["zbus_names", "4.3.4"], ["zvariant_derive", "5.15.0"],
    ].map(([name, version]) => `[[package]]\nname = "${name}"\nversion = "${version}"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`).join("\n"),
    config: '[advisories]\nignore = ["RUSTSEC-2024-0370", "RUSTSEC-2024-0429", "RUSTSEC-2026-0370", "RUSTSEC-2026-0371", "RUSTSEC-2026-0372"]\n',
    workflowSources: ["targets: aarch64-apple-darwin,x86_64-apple-darwin\nargs: --target x86_64-pc-windows-msvc"],
    trees: new Map(releaseTargets.map(target => [target, "selah-app v1.1.0\ntauri v2.12.4\n"])),
  };
}

test("reviewed Linux-only packages can remain in the cross-platform lockfile", () => {
  assert.doesNotThrow(() => checkAuditScope(fixture()));
});

for (const name of ["glib", "proc-macro-error", "zvariant", "zbus_names", "zvariant_derive"]) {
  test(`${name} cannot enter any macOS or Windows release graph`, () => {
    for (const target of releaseTargets) {
      const input = fixture();
      input.trees.set(target, `${input.trees.get(target)}${name} v0.18.5\n`);
      assert.throws(() => checkAuditScope(input), /is reachable/);
    }
  });
}

test("changed versions, sources, and duplicate package versions need a new review", () => {
  for (const lockfile of [
    fixture().lockfile.replace('version = "0.18.5"', 'version = "0.19.0"'),
    fixture().lockfile.replace('source = "registry+https://github.com/rust-lang/crates.io-index"', 'source = "git+https://example.com/fork"'),
    fixture().lockfile + '\n[[package]]\nname = "glib"\nversion = "0.17.0"\n',
  ]) {
    assert.throws(() => checkAuditScope({ ...fixture(), lockfile }), /Reassess or remove/);
  }
});

test("obsolete exceptions must be removed when upstream drops a package", () => {
  const input = fixture();
  input.lockfile = input.lockfile.replace('name = "glib"', 'name = "replacement"');
  assert.throws(() => checkAuditScope(input), /Reassess or remove/);
});

test("adding a Linux release requires reconsidering the exceptions", () => {
  for (const directive of ["targets: x86_64-unknown-linux-gnu", "args: --target=aarch64-unknown-linux-gnu"]) {
    const input = fixture();
    input.workflowSources.push(directive);
    assert.throws(() => checkAuditScope(input), /before adding release target/);
  }
});

test("an incomplete or missing dependency graph cannot pass the guard", () => {
  for (const tree of ["", "tauri v2.12.4\n", undefined]) {
    const input = fixture();
    input.trees.set(releaseTargets[0], tree);
    assert.throws(() => checkAuditScope(input), /Missing release dependency graph/);
  }
});

test("the policy rejects added, duplicated, or missing advisory ignores", () => {
  for (const config of [
    'ignore = ["RUSTSEC-2024-0370"]',
    'ignore = ["RUSTSEC-2024-0370", "RUSTSEC-2024-0429", "RUSTSEC-2023-0071"]',
    'ignore = ["RUSTSEC-2024-0370", "RUSTSEC-2024-0429", "RUSTSEC-2024-0429"]',
    fixture().config.replace('"RUSTSEC-2026-0370", ', ''),
    fixture().config.replace('"RUSTSEC-2026-0371", ', ''),
    fixture().config.replace('"RUSTSEC-2026-0372"', '"RUSTSEC-2026-0369"'),
  ]) {
    assert.throws(() => checkAuditScope({ ...fixture(), config }), /must match/);
  }
});
