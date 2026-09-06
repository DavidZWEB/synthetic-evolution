#!/usr/bin/env bash
# Runs the repository's pinned wasm-pack without relying on shell-specific PATH setup.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo_path="$(./scripts/rustup.sh which cargo)"
export PATH="$(dirname "$cargo_path"):$PATH"

cargo_install_dir="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-}}"
if [ -z "$cargo_install_dir" ]; then
  if [ -z "${HOME:-}" ]; then
    echo "HOME or CARGO_HOME must be set so Cargo tools can be located." >&2
    exit 1
  fi
  cargo_install_dir="$HOME/.cargo"
fi
case "$(uname -s)" in
  CYGWIN*|MINGW*|MSYS*)
    if command -v cygpath >/dev/null 2>&1; then
      cargo_install_dir="$(cygpath -u "$cargo_install_dir")"
    fi
    ;;
esac

wasm_pack=""
for candidate in \
  "$cargo_install_dir/bin/wasm-pack" \
  "$cargo_install_dir/bin/wasm-pack.exe"
do
  if [ -x "$candidate" ]; then
    wasm_pack="$candidate"
    break
  fi
done
if [ -z "$wasm_pack" ]; then
  wasm_pack="$(command -v wasm-pack || true)"
fi
if [ -z "$wasm_pack" ] || [ ! -x "$wasm_pack" ]; then
  echo "wasm-pack is not installed; run ./scripts/setup.sh first" >&2
  exit 1
fi

exec "$wasm_pack" "$@"
