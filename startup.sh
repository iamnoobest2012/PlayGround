#!/usr/bin/env bash
# Once-per-worker prerequisites for the Playground 3D (Rust/Bevy) project.
# Executed via the saved hook on worker start. Must stay idempotent.
set -euo pipefail

TRUNK_VERSION="v0.21.14"

# 1) WebAssembly target for the browser (WASM) build served by start.sh.
if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
  /usr/bin/time -p rustup target add wasm32-unknown-unknown
fi

# 2) Trunk: static WASM site builder (release binary, no compilation needed).
if ! command -v trunk >/dev/null 2>&1; then
  /usr/bin/time -p bash -c '
    set -euo pipefail
    tmp="$(mktemp -d)"
    trap "rm -rf $tmp" EXIT
    curl --fail --silent --show-error --location --max-time 120 \
      "https://github.com/trunk-rs/trunk/releases/download/'"$TRUNK_VERSION"'/trunk-x86_64-unknown-linux-gnu.tar.gz" \
      -o "$tmp/trunk.tar.gz"
    tar -xzf "$tmp/trunk.tar.gz" -C "$tmp"
    mkdir -p "$HOME/.cargo/bin"
    install -m 0755 "$tmp/trunk" "$HOME/.cargo/bin/trunk"
  '
fi
/usr/bin/time -p trunk --version
