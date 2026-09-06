#!/usr/bin/env bash
# Runs rustup with useful PATH diagnostics and Git Bash path normalization.
set -euo pipefail

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

find_brew() {
  brew_path="$(command -v brew || true)"
  if [ -n "$brew_path" ]; then
    printf '%s\n' "$brew_path"
    return
  fi
  for candidate in \
    "${HOMEBREW_PREFIX:-}/bin/brew" \
    /opt/homebrew/bin/brew \
    /usr/local/bin/brew \
    /home/linuxbrew/.linuxbrew/bin/brew
  do
    if [ -x "$candidate" ]; then
      printf '%s\n' "$candidate"
      return
    fi
  done
  return 1
}

rustup_path="$(command -v rustup || command -v rustup.exe || true)"

if [ -z "$rustup_path" ]; then
  cargo_home="${CARGO_HOME:-}"
  if [ -z "$cargo_home" ] && [ -n "${HOME:-}" ]; then
    cargo_home="$HOME/.cargo"
  fi
  for candidate in \
    "${cargo_home:+$cargo_home/bin/rustup}" \
    "${cargo_home:+$cargo_home/bin/rustup.exe}" \
    "${HOMEBREW_PREFIX:-}/opt/rustup/bin/rustup" \
    /opt/homebrew/opt/rustup/bin/rustup \
    /usr/local/opt/rustup/bin/rustup \
    /home/linuxbrew/.linuxbrew/opt/rustup/bin/rustup
  do
    if [ -n "$candidate" ] && [ -x "$candidate" ]; then
      rustup_path="$candidate"
      break
    fi
  done
fi

if [ -z "$rustup_path" ]; then
  brew_path="$(find_brew || true)"
  if [ -n "$brew_path" ]; then
    brew_rustup="$("$brew_path" --prefix rustup 2>/dev/null || true)"
    if [ -x "$brew_rustup/bin/rustup" ]; then
      rustup_path="$brew_rustup/bin/rustup"
    fi
  fi
fi

if [ -z "$rustup_path" ]; then
  printf '%s\n' \
    'rustup is not on PATH or in a supported default location.' \
    'If rustup is already installed elsewhere, add its bin directory to PATH.' \
    '  rustup.rs default: $HOME/.cargo/bin' \
    '  Homebrew defaults: /opt/homebrew/opt/rustup/bin or /usr/local/opt/rustup/bin' \
    '  Linuxbrew default: /home/linuxbrew/.linuxbrew/opt/rustup/bin' \
    'Otherwise install rustup from https://rustup.rs.' >&2
  exit 1
fi

rustup_path="$(normalize_path "$rustup_path")"
rustup_bin="$(dirname "$rustup_path")"

# Homebrew links rustup into its general bin but leaves Cargo's proxies in the
# formula's keg-only bin directory.
if [ ! -x "$rustup_bin/cargo" ] && [ ! -x "$rustup_bin/cargo.exe" ]; then
  brew_path="$(find_brew || true)"
  if [ -n "$brew_path" ]; then
    brew_rustup="$("$brew_path" --prefix rustup 2>/dev/null || true)"
  else
    brew_rustup=""
  fi
  if [ -x "$brew_rustup/bin/rustup" ] &&
     { [ -x "$brew_rustup/bin/cargo" ] || [ -x "$brew_rustup/bin/cargo.exe" ]; }; then
    rustup_path="$(normalize_path "$brew_rustup/bin/rustup")"
    rustup_bin="$brew_rustup/bin"
  fi
fi

case "${1:-}" in
  --print-path)
    printf '%s\n' "$rustup_path"
    exit 0
    ;;
  --print-bin)
    normalize_path "$rustup_bin"
    exit 0
    ;;
  which)
    selected_path="$("$rustup_path" "$@")"
    selected_path="${selected_path%$'\r'}"
    normalize_path "$selected_path"
    exit 0
    ;;
esac

exec "$rustup_path" "$@"
