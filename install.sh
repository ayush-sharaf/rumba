#!/usr/bin/env bash
# rumba one-step installer — installs the binary AND its runtime tools.
#
#   curl -fsSL https://raw.githubusercontent.com/ayush-sharaf/rumba/master/install.sh | bash
#
# Env knobs:
#   RUMBA_INSTALL_DIR   where the binary goes (default: ~/.local/bin)
#   RUMBA_VERSION       a specific tag, e.g. v0.1.0 (default: latest)
#   RUMBA_NO_DEPS=1     skip installing mpv/yt-dlp/ffmpeg
set -euo pipefail

REPO="ayush-sharaf/rumba"
INSTALL_DIR="${RUMBA_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${RUMBA_VERSION:-latest}"
DEPS="mpv yt-dlp ffmpeg"

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
  *) red "unsupported OS: $os"; exit 1 ;;
esac

# --- 1. download + install the binary --------------------------------------
asset="rumba-${target}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  base="https://github.com/${REPO}/releases/latest/download"
else
  base="https://github.com/${REPO}/releases/download/${VERSION}"
fi
url="${base}/${asset}"

bold "▶ Installing rumba ($target)"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
if ! curl -fSL --progress-bar "$url" -o "$tmp/$asset"; then
  red "  download failed — no release asset for $target?"; exit 1
fi
if curl -fsSL "${url}.sha256" -o "$tmp/$asset.sha256" 2>/dev/null; then
  ( cd "$tmp" && (sha256sum -c "$asset.sha256" >/dev/null 2>&1 \
      || shasum -a 256 -c "$asset.sha256" >/dev/null 2>&1) ) \
    && dim "  checksum OK" || { red "  checksum mismatch"; exit 1; }
fi
tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/rumba" "$INSTALL_DIR/rumba"
bold "  ✓ rumba → $INSTALL_DIR/rumba"

# --- 2. install runtime tools (mpv / yt-dlp / ffmpeg) ----------------------
missing=""
for d in $DEPS; do command -v "$d" >/dev/null 2>&1 || missing="$missing $d"; done
missing="${missing# }"

if [ -n "$missing" ] && [ "${RUMBA_NO_DEPS:-0}" != "1" ]; then
  bold "▶ Installing runtime tools:$missing"
  sudo_cmd=""; [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1 && sudo_cmd="sudo -n"

  run_pm() { dim "  + $*"; eval "$* >/dev/null 2>&1"; }

  if [ "$os" = "Darwin" ]; then
    if command -v brew >/dev/null 2>&1; then
      run_pm "brew install $missing" && bold "  ✓ tools installed" \
        || red "  couldn't auto-install — run: brew install $missing"
    else
      red "  Homebrew not found — install it (https://brew.sh) then: brew install $missing"
    fi
  else
    pkgs="$missing"
    if   command -v apt-get >/dev/null 2>&1; then pm="$sudo_cmd apt-get install -y $pkgs"; pre="$sudo_cmd apt-get update"
    elif command -v dnf     >/dev/null 2>&1; then pm="$sudo_cmd dnf install -y $pkgs"
    elif command -v pacman  >/dev/null 2>&1; then pm="$sudo_cmd pacman -S --noconfirm $pkgs"
    elif command -v zypper  >/dev/null 2>&1; then pm="$sudo_cmd zypper install -y $pkgs"
    else pm=""; fi

    if [ -n "$pm" ]; then
      [ -n "${pre:-}" ] && run_pm "$pre"
      if run_pm "$pm"; then bold "  ✓ tools installed"
      else
        red "  couldn't auto-install (need a password? a pipe can't prompt). Run:"
        dim "    ${pm#sudo -n }"
        dim "    (yt-dlp may need: pipx install yt-dlp)"
      fi
    else
      red "  unknown package manager — install manually:$missing"
    fi
  fi
fi

# --- 3. PATH hint + done ----------------------------------------------------
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo; dim "Add rumba to your PATH:"; echo "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
echo
bold "Done. Run:  rumba"
dim  "First launch reads your logged-in YouTube Music session from your browser."
