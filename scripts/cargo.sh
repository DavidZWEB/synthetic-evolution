#!/usr/bin/env bash
# Runs Cargo from the repository's rustup-selected toolchain without assuming PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo_path="$(./scripts/rustup.sh which cargo)"
export PATH="$(dirname "$cargo_path"):$PATH"
exec "$cargo_path" "$@"
