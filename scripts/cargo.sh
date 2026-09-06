#!/usr/bin/env bash
# Runs Cargo from the repository's rustup-selected toolchain without assuming PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is not available; run ./scripts/setup.sh first" >&2
  exit 1
fi

cargo_path="$(rustup which cargo)"
export PATH="$(dirname "$cargo_path"):$PATH"
exec "$cargo_path" "$@"
