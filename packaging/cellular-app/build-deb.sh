#!/bin/sh
# Assemble harbour-cellular_<ver>_arm64.deb: the standalone Cellular app.
#
# One package, three parts:
#   * /usr/bin/harbour-cellular        the standalone UI (src/bin/harbour-cellular.rs)
#   * /usr/bin/harbour-celld           the backend daemon (its own crate,
#                                      ~/workspace/harbour-celld) + systemd unit
#   * /usr/share/harbour-cellular/     the assets the UI loads (fonts, themes,
#                                      images, the cellular app package)
# plus the APPLaunch entry in /usr/share/APPLaunch/applications (that is
# where APPLaunch scans — not the freedesktop dir) and the udev rule
# that names the modem's AT port /dev/cell-at.
#
# The same daemon also serves Harbour Shell's built-in Cellular page over
# /run/cell.sock, so installing this package is enough for both frontends.
#
# Inputs (env, with defaults for the usual workspace layout):
#   CELL_BIN    harbour-cellular binary (default: this repo's release target)
#   CELLD_BIN   harbour-celld binary    (default: ~/workspace/harbour-celld target)
#   VERSION     package version         (default: 1.0.0)
#   OUTDIR      where the .deb lands    (default: ~/workspace/your_files)
# Run from the repo root of a tree that contains the cellular feature
# (feat/cellular, or the zero integration branch for the Fusion font assets).
set -eu
cd "$(dirname "$0")/../.."
REPO=$PWD
CELL_BIN=${CELL_BIN:-$REPO/target/aarch64-unknown-linux-gnu/release/harbour-cellular}
CELLD_BIN=${CELLD_BIN:-$HOME/workspace/harbour-celld/target/aarch64-unknown-linux-gnu/release/harbour-celld}
VERSION=${VERSION:-1.0.0}
OUTDIR=${OUTDIR:-$HOME/workspace/your_files}
STAGE=$HOME/workspace/harbour-cellular-pkg/stage

for f in "$CELL_BIN" "$CELLD_BIN" assets/fonts apps/cellular/icon.png; do
    [ -e "$f" ] || { echo "missing input: $f" >&2; exit 1; }
done

rm -rf "$STAGE"
mkdir -p "$STAGE/usr/bin" "$STAGE/usr/share/harbour-cellular/apps" \
         "$STAGE/usr/share/APPLaunch/applications" "$STAGE/usr/share/APPLaunch/share/images" \
         "$STAGE/usr/lib/systemd/system" \
         "$STAGE/usr/lib/udev/rules.d" "$STAGE/var/lib/harbour-cell" "$STAGE/DEBIAN"

cp "$CELL_BIN" "$STAGE/usr/bin/harbour-cellular"
cp "$CELLD_BIN" "$STAGE/usr/bin/harbour-celld"
cp -r assets "$STAGE/usr/share/harbour-cellular/"
cp -r apps/cellular "$STAGE/usr/share/harbour-cellular/apps/"
# APPLaunch discovers apps in its own applications dir, with icons as
# paths relative to /usr/share/APPLaunch (share/images/<name>.png).
cp packaging/cellular-app/harbour-cellular.desktop "$STAGE/usr/share/APPLaunch/applications/"
cp apps/cellular/icon.png "$STAGE/usr/share/APPLaunch/share/images/harbour-cellular.png"
cp packaging/cellular-app/harbour-celld.service "$STAGE/usr/lib/systemd/system/"
cp packaging/cellular-app/99-harbour-cell.rules "$STAGE/usr/lib/udev/rules.d/"

cat > "$STAGE/DEBIAN/control" <<EOF
Package: harbour-cellular
Version: $VERSION
Section: utils
Priority: optional
Architecture: arm64
Depends: libsdl2-2.0-0, libsdl2-image-2.0-0
Maintainer: hexbyte42-bot
Description: Cellular app for Cardputer Zero + QDC507 (4G data + SMS)
 Standalone app (for the official APPLaunch) plus the harbour-celld
 backend daemon. The daemon is shared: Harbour Shell's built-in Cellular
 page talks to the same /run/cell.sock service. Voice is not included yet.
EOF

cat > "$STAGE/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
mkdir -p /var/lib/harbour-cell
# A hand-installed unit in /etc would shadow the packaged one; drop it so
# the packaged ExecStart (/usr/bin/harbour-celld) is the one that runs.
if [ -f /etc/systemd/system/harbour-celld.service ]; then
    rm -f /etc/systemd/system/harbour-celld.service
fi
systemctl daemon-reload || true
systemctl enable harbour-celld.service || true
systemctl restart harbour-celld.service || true
udevadm control --reload-rules || true
udevadm trigger --subsystem-match=tty || true
EOF
chmod 755 "$STAGE/DEBIAN/postinst"

# The workspace umask leaves files 660; a deb must be world-readable.
chmod -R a+rX "$STAGE"
find "$STAGE" -type d -exec chmod 755 {} +
chmod 755 "$STAGE/usr/bin/harbour-cellular" "$STAGE/usr/bin/harbour-celld"

mkdir -p "$OUTDIR"
dpkg-deb --build "$STAGE" "$OUTDIR/harbour-cellular_${VERSION}_arm64.deb"
echo "built: $OUTDIR/harbour-cellular_${VERSION}_arm64.deb"
dpkg-deb -c "$OUTDIR/harbour-cellular_${VERSION}_arm64.deb" | head -20
