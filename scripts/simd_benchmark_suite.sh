#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(CDPATH= cd -- "$script_dir/.." && pwd)"
cd "$repo_dir"

# Keep these commands in one place so a change to layout data, target
# dispatch, or a maintained numeric dependency reruns the same evidence that
# justified the retained kernel. All profiles are explicitly ignored/manual
# tests and print their p50/p95 measurements.
cargo test -p aimer_flex --release -- --ignored --nocapture
cargo test -p aimer_scroll --release -- --ignored --nocapture
cargo test -p aimer_animation --release -- --ignored --nocapture
# The color-layer and scan-conversion profiles live with the font engine.
cargo test -p aimer_cupid -p aimer_cupid_font --release -- --ignored --nocapture

# The low-level profiles are necessary but not sufficient: keep the paired
# native frame control beside them so a kernel is not accepted on microbench
# numbers alone.
cargo run -p aimer_laboratory --example framework_baseline --release
cargo run -p aimer_laboratory --example framework_baseline --release --features force-scalar
