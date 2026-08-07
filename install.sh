#!/bin/sh
#
# zyris-code installer for Linux and macOS.
#
# Builds zyris-code from source with cargo and installs the binary into
# <prefix>/bin. Re-running the script rebuilds from the latest main, so it
# doubles as the updater.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/attacca-cc/zyris-code/main/install.sh | sh
#   ./install.sh [--prefix <dir>] [--uninstall] [--help]

set -eu

REPO_URL="https://github.com/attacca-cc/zyris-code"

usage() {
  cat <<'EOF'
Usage: install.sh [--prefix <dir>] [--uninstall] [--help]

Installs the zyris-code terminal client by building it from source with cargo.

  --prefix <dir>  Install root; the binary lands in <dir>/bin.
                  Default: the directory that contains the cargo bin dir
                  (the cargo home, usually ~/.cargo).
  --uninstall     Remove the zyris-code binary.
  --help          Show this help.

The binary is installed to <prefix>/bin/zyris-code. Re-running this script
rebuilds and reinstalls the latest version from main.
EOF
}

PREFIX=""
PREFIX_AUTO=0
UNINSTALL=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --prefix)
      [ "$#" -ge 2 ] || { echo "error: --prefix requires a value" >&2; exit 2; }
      PREFIX="$2"
      shift 2
      ;;
    --prefix=*)
      PREFIX="${1#--prefix=}"
      shift
      ;;
    --uninstall)
      UNINSTALL=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ -z "$PREFIX" ]; then
  PREFIX_AUTO=1
  if command -v cargo >/dev/null 2>&1; then
    # Parent of the directory cargo lives in, so the binary lands right back
    # in the bin dir that is already on PATH (rustup: ~/.cargo/bin).
    PREFIX="$(CDPATH= cd "$(dirname "$(command -v cargo)")/.." && pwd)"
  else
    PREFIX="$HOME/.cargo"
  fi
fi
BIN_DIR="$PREFIX/bin"

# An auto-detected prefix that is not writable (e.g. a distro cargo in /usr)
# falls back to the user's cargo home instead of failing on a permission error.
if [ "$PREFIX_AUTO" -eq 1 ] && [ ! -w "$BIN_DIR" ] && [ ! -w "$PREFIX" ]; then
  echo "note: $PREFIX is not writable by you; installing to $HOME/.cargo instead" >&2
  PREFIX="$HOME/.cargo"
  BIN_DIR="$PREFIX/bin"
fi

if [ "$UNINSTALL" -eq 1 ]; then
  if [ -f "$BIN_DIR/zyris-code" ]; then
    rm -f "$BIN_DIR/zyris-code"
    echo "Uninstalled zyris-code from $BIN_DIR"
  else
    echo "error: zyris-code is not installed in $BIN_DIR" >&2
    exit 1
  fi
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  cat >&2 <<'EOF'
error: cargo (Rust) is required to build zyris-code from source.

Install it with rustup:
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

then open a new terminal (or run: source "$HOME/.cargo/env")
and re-run this script.
EOF
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "error: git is required to fetch the zyris-code sources" >&2
  exit 1
fi

echo "Installing zyris-code ..."
echo "  platform: $(uname -s) ($(uname -m))"
echo "  source:   $REPO_URL"
echo "  prefix:   $PREFIX  (binaries go to $BIN_DIR)"
echo "  cargo:    $(command -v cargo)"

mkdir -p "$BIN_DIR"
cargo install --git "$REPO_URL" --locked --force --root "$PREFIX"

echo
echo "Installed zyris-code to $BIN_DIR/zyris-code"
echo
echo "Run it from the directory you want to work in:"
echo "    cd ~/your-project"
echo "    zyris-code"
