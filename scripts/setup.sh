#!/usr/bin/env bash
# One-command setup for a fresh machine. If a new checkout needs a step that is not in
# here, this script is wrong — fix it rather than documenting the step.
set -euo pipefail
cd "$(dirname "$0")/.."

# Cargo-installed binaries are separate programs and are not covered by Cargo.lock,
# so the version is pinned here instead.
WASM_PACK_VERSION="0.15.0"

info() { printf '\n==> %s\n' "$1"; }
warn() { printf '    warning: %s\n' "$1" >&2; }

# Prints how to put a directory on PATH without assuming a shell or an OS.
path_hint() {
  printf '    Add it to your PATH, in whichever profile your shell reads\n'
  printf '    (~/.zshrc, ~/.bashrc, ~/.config/fish/config.fish, ...):\n'
  printf '        export PATH="%s:$PATH"\n' "$1"
}

info "Rust toolchain"
if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is not on your PATH. Install it from https://rustup.rs" >&2
  if [ "$(uname -s)" = "Darwin" ] && [ -x /opt/homebrew/opt/rustup/bin/rustup ]; then
    echo "  Homebrew has it installed, but the formula is keg-only:" >&2
    path_hint "/opt/homebrew/opt/rustup/bin" >&2
  fi
  exit 1
fi
# Reads rust-toolchain.toml and installs the pinned compiler, components, and the
# wasm target if they are missing. Nobody has to remember `rustup target add`.
rustup show

info "Rust binary tools"
# `cargo install` writes here. The rustup.rs installer puts it on your PATH; a
# Homebrew rustup does not, which is why this is checked explicitly below rather than
# assumed — `npm run wasm` fails with "command not found" otherwise.
CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
installed_wasm_pack="$("$CARGO_BIN/wasm-pack" --version 2>/dev/null | awk '{print $2}' || true)"
if [ "$installed_wasm_pack" = "$WASM_PACK_VERSION" ]; then
  echo "    wasm-pack $WASM_PACK_VERSION already installed"
else
  # --locked builds wasm-pack against its own committed lockfile; without it cargo
  # resolves its dependencies fresh and the build occasionally fails.
  cargo install --locked "wasm-pack@$WASM_PACK_VERSION"
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

info "Verifying"
cargo test --workspace

printf '\nSetup complete.\n'
printf '  cargo test --workspace       run the test suite\n'
printf '  npm run dev --prefix web     start the client at http://localhost:5173\n'

case ":$PATH:" in
  *":$CARGO_BIN:"*) ;;
  *)
    printf '\n'
    warn "$CARGO_BIN is not on your PATH, so wasm-pack will not be found."
    path_hint "$CARGO_BIN"
    ;;
esac
