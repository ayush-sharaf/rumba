#!/usr/bin/env bash
# rumba installer — downloads the latest prebuilt binary for your platform.
#
#   curl -fsSL https://raw.githubusercontent.com/ayush-sharaf/rumba/master/install.sh | bash
#
# Honors:
#   RUMBA_INSTALL_DIR   where to install (default: ~/.local/bin)
#   RUMBA_VERSION       a specific tag, e.g. v0.1.0 (default: latest)
set -euo pipefail

REPO="ayush-sharaf/rumba"
INSTALL_DIR="${RUMBA_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${RUMBA_VERSION:-latest}"

red()  { printf '\033[1;31m%s\033[0m\n' "$*"; }
dim()  { printf '\033[2m%s\033[0m\n' "$*"; }
bold() { printf '\033[1m%s\033[0m\n' "$*"; }

# --- detect platform -> Rust target triple ---------------------------------
os="$(uname -s)"; arch="$(uname -m)"
case "$os" in
  Darwin) case "$arch" in
            arm64|aarch64) target="aarch64-apple-darwin" ;;
            x86_64)        target="x86_64-apple-darwin" ;;
            *) red "unsupported macOS arch: $arch"; exit 1 ;;
          esac ;;
  Linux)  case "$arch" in
            x86_64)        target="x86_64-unknown-linux-gnu" ;;
            aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
            *) red "unsupported Linux arch: $arch"; exit 1 ;;
          esac ;;
  *) red "unsupported OS: $os (use 'cargo install rumba' instead)"; exit 1 ;;
esac

asset="rumba-${target}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

bold "Installing rumba ($target)"
dim  "  from $url"

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
if ! curl -fSL --progress-bar "$url" -o "$tmp/$asset"; then
  red "download failed — is there a release yet for $target?"
  exit 1
fi

# verify checksum if the .sha256 is published alongside
if curl -fsSL "${url}.sha256" -o "$tmp/$asset.sha256" 2>/dev/null; then
  ( cd "$tmp" && (sha256sum -c "$asset.sha256" >/dev/null 2>&1 \
      || shasum -a 256 -c "$asset.sha256" >/dev/null 2>&1) ) \
    && dim "  checksum OK" || { red "checksum mismatch"; exit 1; }
fi

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/rumba" "$INSTALL_DIR/rumba"
bold "✓ installed rumba -> $INSTALL_DIR/rumba"

# --- PATH hint --------------------------------------------------------------
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) dim "  note: $INSTALL_DIR is not on your PATH — add:"
     echo "        export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

# --- runtime dependency check ----------------------------------------------
missing=""
for dep in mpv yt-dlp ffmpeg; do
  command -v "$dep" >/dev/null 2>&1 || missing="$missing $dep"
done
if [ -n "$missing" ]; then
  echo
  red "rumba needs these runtime tools, not found on PATH:$missing"
  if [ "$os" = "Darwin" ]; then
    dim  "  install with:  brew install$missing"
  else
    dim  "  install via your package manager (e.g. apt/dnf/pacman):$missing"
    dim  "  (yt-dlp is often best via:  pipx install yt-dlp)"
  fi
fi

echo
bold "Run it:  rumba"
dim  "First launch reads your logged-in YouTube Music session from your browser."
