#!/usr/bin/env bash
# Runs the repository's pinned wasm-pack without relying on shell-specific PATH setup.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is not available; run ./scripts/setup.sh first" >&2
  exit 1
fi

cargo_path="$(rustup which cargo)"
export PATH="$(dirname "$cargo_path"):$PATH"

cargo_install_dir="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}"
wasm_pack="$cargo_install_dir/bin/wasm-pack"
if [ ! -x "$wasm_pack" ]; then
  wasm_pack="$(command -v wasm-pack || true)"
fi
if [ -z "$wasm_pack" ] || [ ! -x "$wasm_pack" ]; then
  echo "wasm-pack is not installed; run ./scripts/setup.sh first" >&2
  exit 1
fi

exec "$wasm_pack" "$@"
