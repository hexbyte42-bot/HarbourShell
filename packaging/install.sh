#!/bin/sh
# Harbour Shell installer for the M5Stack Cardputer Zero.
#
#   curl -fsSL https://raw.githubusercontent.com/kurogedelic/HarbourShell/main/install.sh | sh
#
# Downloads the latest release (.deb) from GitHub, checks its SHA-256, installs it with apt and
# starts Harbour Shell once. Nothing is made permanent: the next boot still uses the official
# launcher until you run `harbourctl enable`. `harbourctl official` goes back at any time.
#   HARBOUR_VERSION=0.1.0 sh install.sh    install that version instead of the latest
set -eu

REPO=kurogedelic/HarbourShell
say() { printf '%s\n' "$*"; }
die() { say "harbour-shell: $*" >&2; exit 1; }

[ "$(uname -m)" = aarch64 ] || die "this is for the Cardputer Zero (aarch64), not $(uname -m)"
command -v apt >/dev/null 2>&1 || die "apt not found (Debian expected)"
command -v curl >/dev/null 2>&1 || die "curl not found"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum not found"
[ "$(id -u)" -ne 0 ] || die "run this as your normal user (it asks for sudo when needed)"

# 1. Which release.
if [ -n "${HARBOUR_VERSION:-}" ]; then
  TAG="v$HARBOUR_VERSION"
else
  TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
  [ -n "$TAG" ] || die "could not find the latest release"
fi
VER=${TAG#v}
DEB="harbour-shell_${VER}_arm64.deb"
BASE="https://github.com/$REPO/releases/download/$TAG"

# 2. Download and check.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM
say "Harbour Shell $VER"
say "  downloading $DEB ..."
curl -fsSL --retry 2 -o "$TMP/$DEB" "$BASE/$DEB" || die "download failed: $BASE/$DEB"
curl -fsSL --retry 2 -o "$TMP/$DEB.sha256" "$BASE/$DEB.sha256" || die "download failed: $BASE/$DEB.sha256"
WANT=$(cut -d' ' -f1 "$TMP/$DEB.sha256")
HAVE=$(sha256sum "$TMP/$DEB" | cut -d' ' -f1)
[ "$WANT" = "$HAVE" ] || die "checksum mismatch: the download is damaged or not the release"
say "  checksum ok"

# 3. Install (apt pulls in the SDL libraries).
say "  installing (sudo asks for your password) ..."
sudo apt-get install -y "$TMP/$DEB" >/dev/null || die "apt could not install $DEB"

# 4. Optional Japanese input dictionary (downloaded only on yes; default No).
if [ -f "$HOME/.local/share/harbour/ime/harbour-ime.dic" ]; then
  say "  Japanese dictionary: already installed"
elif [ -r /dev/tty ]; then
  sh /usr/share/harbour-shell/ime_install.sh install </dev/tty || true
fi

# 5. Start it once.
say ""
harbourctl try || die "installed, but could not start it: run 'harbourctl try'"
say ""
say "Harbour Shell is running. The next boot still uses the official launcher."
say "  Keep it at boot:       harbourctl enable"
say "                         then once: sudo sh /usr/share/harbour-shell/install_system.sh"
say "  Back to the official:  harbourctl official"
say "  Remove it:             sudo apt remove harbour-shell"
