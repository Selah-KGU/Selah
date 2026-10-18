# sherpa-onnx-sys 1.13.8 security patch

Source: the crates.io `sherpa-onnx-sys` 1.13.8 archive, with its Apache-2.0 license.
The FFI bindings and build script are unchanged.

The only source change is `zip = "2"` to `zip = "9"` in Cargo.toml, matching
upstream commit [1a9d234ab95567eba8c2e89873a87d70b79633e2](https://github.com/k2-fsa/sherpa-onnx/commit/1a9d234ab95567eba8c2e89873a87d70b79633e2)
([upstream issue](https://github.com/k2-fsa/sherpa-onnx/issues/4058)).
This removes RUSTSEC-2026-0367, RUSTSEC-2026-0368, and RUSTSEC-2026-0369 without
changing the pinned 1.13.8 native runtime or its ABI.

Remove this Cargo patch when a compatible crates.io release includes the fix.
