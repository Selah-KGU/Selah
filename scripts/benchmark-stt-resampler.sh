#!/usr/bin/env bash
# Compile the real DSP module without rebuilding Tauri or loading STT models.
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
benchmark_dir="$(mktemp -d "${TMPDIR:-/tmp}/selah-resampler.XXXXXX")"
trap 'rm -rf -- "$benchmark_dir"' EXIT

mkdir -p "$benchmark_dir/stt/audio/resampler"
cp "$repo_root/src-tauri/src/stt/audio/resampler.rs" "$benchmark_dir/stt/audio/resampler.rs"
cp "$repo_root/src-tauri/src/stt/audio/resampler/tests.rs" "$benchmark_dir/stt/audio/resampler/tests.rs"
cp "$repo_root/src-tauri/src/stt/audio/resampler/reference.rs" "$benchmark_dir/stt/audio/resampler/reference.rs"
cat > "$benchmark_dir/main.rs" <<'RUST'
mod stt {
    const TARGET_SAMPLE_RATE: i32 = 16000;
    mod audio {
        #[path = "resampler.rs"]
        mod resampler;
    }
}
RUST

rustc --edition 2021 --test -O "$benchmark_dir/main.rs" -o "$benchmark_dir/check"
"$benchmark_dir/check"
"$benchmark_dir/check" --ignored --nocapture
