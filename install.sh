#!/usr/bin/env sh
set -eu

REPO="${IKAROS_INSTALL_REPO:-lingbou/Ikaros}"
VERSION="${IKAROS_INSTALL_VERSION:-latest}"
BIN_DIR="${IKAROS_INSTALL_BIN_DIR:-$HOME/.local/bin}"
TMP_DIR="${TMPDIR:-/tmp}/ikaros-install.$$"

detect_target() {
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$os:$arch" in
    linux:x86_64) echo "x86_64-unknown-linux-gnu" ;;
    linux:aarch64|linux:arm64) echo "aarch64-unknown-linux-gnu" ;;
    darwin:x86_64) echo "x86_64-apple-darwin" ;;
    darwin:aarch64|darwin:arm64) echo "aarch64-apple-darwin" ;;
    *) echo "unsupported target: $os $arch" >&2; exit 1 ;;
  esac
}

download_url() {
  target="$1"
  if [ "$VERSION" = "latest" ]; then
    echo "https://github.com/$REPO/releases/latest/download/ikaros-$target.tar.gz"
  else
    echo "https://github.com/$REPO/releases/download/$VERSION/ikaros-$target.tar.gz"
  fi
}

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

target="$(detect_target)"
url="$(download_url "$target")"
mkdir -p "$BIN_DIR" "$TMP_DIR"

echo "install_target: $target"
echo "install_url: $url"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$TMP_DIR/ikaros.tar.gz"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$TMP_DIR/ikaros.tar.gz" "$url"
else
  echo "curl or wget is required" >&2
  exit 1
fi

tar -xzf "$TMP_DIR/ikaros.tar.gz" -C "$TMP_DIR"
binary="$(find "$TMP_DIR" -type f -name ikaros -perm -111 | head -n 1)"
if [ -z "$binary" ]; then
  echo "release archive did not contain executable ikaros" >&2
  exit 1
fi
install -m 0755 "$binary" "$BIN_DIR/ikaros"
echo "installed: $BIN_DIR/ikaros"
echo "next: ikaros setup"
