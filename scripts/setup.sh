#!/usr/bin/env bash
# One-command setup for a fresh machine. If a new checkout needs a step that is not in
# here, this script is wrong — fix it rather than documenting the step.
#
#   --skip-tests     install everything, but do not run the suite at the end.
#   --with-browser   also install the Playwright browser for visual validation.
#
# CI uses --skip-tests where another workflow has already verified the same commit.
# Browser automation is opt-in because most builds do not need its large download.
set -euo pipefail
cd "$(dirname "$0")/.."

original_path="$PATH"
run_tests=1
install_browser=0
for arg in "$@"; do
  case "$arg" in
    --skip-tests) run_tests=0 ;;
    --with-browser) install_browser=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

# Cargo-installed binaries are separate programs and are not covered by Cargo.lock,
# so the version is pinned here instead.
WASM_PACK_VERSION="0.15.0"

info() { printf '\n==> %s\n' "$1"; }
warn() { printf '    warning: %s\n' "$1" >&2; }
path_hint() {
  printf '    Add it to PATH in the profile your shell reads:\n'
  printf '        export PATH="%s:$PATH"\n' "$1"
}
normalize_path() {
  case "$(uname -s)" in
    CYGWIN*|MINGW*|MSYS*)
      if command -v cygpath >/dev/null 2>&1; then
        cygpath -u "$1"
        return
      fi
      ;;
  esac
  printf '%s\n' "$1"
}
normalize_executable_path() {
  if [ -z "$1" ]; then
    printf '\n'
    return
  fi
  executable_path="$(normalize_path "$1")"
  case "$(uname -s)" in
    CYGWIN*|MINGW*|MSYS*)
      executable_path="${executable_path%.exe}"
      executable_path="${executable_path%.EXE}"
      ;;
  esac
  printf '%s\n' "$executable_path"
}

info "Rust toolchain"
# Reads rust-toolchain.toml and installs the pinned compiler, components, and the
# wasm target if they are missing. Nobody has to remember `rustup target add`.
rustup_bin="$(./scripts/rustup.sh --print-bin)"
./scripts/rustup.sh show
cargo_path="$(./scripts/rustup.sh which cargo)"
export PATH="$(dirname "$cargo_path"):$PATH"

info "Rust binary tools"
# Cargo-installed tools live outside the selected toolchain. Use Cargo's configured
# install root directly so setup does not depend on a particular shell profile.
CARGO_INSTALL_DIR="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-}}"
if [ -z "$CARGO_INSTALL_DIR" ]; then
  if [ -z "${HOME:-}" ]; then
    echo "HOME or CARGO_HOME must be set so Cargo tools have an install directory." >&2
    exit 1
  fi
  CARGO_INSTALL_DIR="$HOME/.cargo"
fi
CARGO_INSTALL_DIR="$(normalize_path "$CARGO_INSTALL_DIR")"
CARGO_BIN="$CARGO_INSTALL_DIR/bin"
wasm_pack_path=""
for candidate in "$CARGO_BIN/wasm-pack" "$CARGO_BIN/wasm-pack.exe"; do
  if [ -x "$candidate" ]; then
    wasm_pack_path="$candidate"
    break
  fi
done
if [ -z "$wasm_pack_path" ]; then
  wasm_pack_path="$(command -v wasm-pack || true)"
fi
installed_wasm_pack=""
if [ -n "$wasm_pack_path" ]; then
  installed_wasm_pack="$("$wasm_pack_path" --version 2>/dev/null | awk '{print $2}' || true)"
fi
if [ "$installed_wasm_pack" = "$WASM_PACK_VERSION" ]; then
  echo "    wasm-pack $WASM_PACK_VERSION already installed"
else
  # --locked builds wasm-pack against its own committed lockfile; without it cargo
  # resolves its dependencies fresh and the build occasionally fails.
  "$cargo_path" install --locked --root "$CARGO_INSTALL_DIR" "wasm-pack@$WASM_PACK_VERSION"
  for candidate in "$CARGO_BIN/wasm-pack" "$CARGO_BIN/wasm-pack.exe"; do
    if [ -x "$candidate" ]; then
      wasm_pack_path="$candidate"
      break
    fi
  done
  if [ -z "$wasm_pack_path" ]; then
    echo "cargo install succeeded but wasm-pack was not found under $CARGO_BIN" >&2
    exit 1
  fi
fi

info "Node"
# nvm is a shell function, not a binary, so it has to be sourced before it can be
# called from a script. Managing Node some other way is fine — the version is what
# matters, so this checks that and moves on rather than insisting on nvm.
NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
if [ -s "$NVM_DIR/nvm.sh" ]; then
  # shellcheck disable=SC1091
  . "$NVM_DIR/nvm.sh"
  nvm install
else
  wanted="$(tr -d '[:space:]' < .nvmrc)"
  if ! command -v node >/dev/null 2>&1; then
    echo "Node is not installed. Install Node $wanted — nvm (https://github.com/nvm-sh/nvm)," >&2
    echo "fnm, Volta, or a package manager all work." >&2
    exit 1
  fi
  current="$(node --version | sed 's/^v//; s/\..*//')"
  if [ "$current" != "$wanted" ]; then
    warn "node $(node --version) is installed but .nvmrc asks for $wanted"
  else
    echo "    node $(node --version)"
  fi
fi

info "Node packages"
# `ci` installs exactly what package-lock.json says and errors if the manifest and the
# lockfile disagree. `install` would quietly rewrite the lockfile, which is how two
# machines diverge — use it only when deliberately adding a dependency.
npm ci --prefix web

if [ "$install_browser" -eq 1 ]; then
  info "Browser automation"
  if [ "$(uname -s)" = "Linux" ]; then
    npm run browser:install-with-deps --prefix web
  else
    npm run browser:install --prefix web
  fi
fi

if [ "$run_tests" -eq 1 ]; then
  info "Verifying"
  "$cargo_path" test --workspace
else
  info "Verifying"
  echo "    skipped (--skip-tests)"
fi
printf '\nSetup complete.\n'
printf '  cargo test --workspace       run the test suite\n'
printf '  npm run dev --prefix web     start the client (use the URL Vite prints)\n'

cargo_on_path="$(PATH="$original_path" command -v cargo || true)"
cargo_proxy="$rustup_bin/cargo"
if [ -x "$rustup_bin/cargo.exe" ]; then
  cargo_proxy="$rustup_bin/cargo.exe"
fi
cargo_on_path_normalized="$(normalize_executable_path "$cargo_on_path")"
cargo_proxy_normalized="$(normalize_executable_path "$cargo_proxy")"
if [ -z "$cargo_on_path" ] ||
   [ "$cargo_on_path_normalized" != "$cargo_proxy_normalized" ]; then
  printf '\n'
  warn "PATH does not select the rustup proxy for Cargo."
  path_hint "$rustup_bin"
fi
wasm_pack_on_path="$(PATH="$original_path" command -v wasm-pack || true)"
wasm_pack_on_path_version=""
if [ -n "$wasm_pack_on_path" ]; then
  wasm_pack_on_path_version="$("$wasm_pack_on_path" --version 2>/dev/null | awk '{print $2}' || true)"
fi
wasm_pack_on_path_normalized="$(normalize_executable_path "$wasm_pack_on_path")"
wasm_pack_path_normalized="$(normalize_executable_path "$wasm_pack_path")"
if [ -z "$wasm_pack_on_path" ] ||
   [ "$wasm_pack_on_path_version" != "$WASM_PACK_VERSION" ] ||
   [ "$wasm_pack_on_path_normalized" != "$wasm_pack_path_normalized" ]; then
  printf '\n'
  warn "PATH does not select the pinned wasm-pack $WASM_PACK_VERSION."
  path_hint "$(dirname "$wasm_pack_path")"
fi
