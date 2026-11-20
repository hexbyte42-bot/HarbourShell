# Cellular as a standalone app

The cellular feature ships in two forms over one backend:

- **harbour-celld** (separate crate, `~/workspace/harbour-celld`): the
  daemon that owns the QDC507 modem — AT channel, SMS PDU codec, SQLite
  store — and serves NDJSON JSON-RPC on `/run/cell.sock`.
- **Frontends**: Harbour Shell's built-in Cellular page, and the
  standalone `harbour-cellular` binary (`src/bin/harbour-cellular.rs`), a
  minimal single-app host for the same `LauncherApp` module, meant to be
  launched from the official APPLaunch via
  `/usr/share/APPLaunch/applications/harbour-cellular.desktop` (APPLaunch
  scans that dir, not the freedesktop one; its icon goes to
  `/usr/share/APPLaunch/share/images/harbour-cellular.png`).

`build-deb.sh` assembles one `harbour-cellular` deb containing the
standalone UI, the daemon, its systemd unit and the udev rule, so a
device gets the whole feature from a single package; the shell's page
then works too, since it only needs the daemon.

Notes / sharp edges:

- The standalone binary loads its fonts through the same block as the
  shell's `main`. That block differs between the topic branches
  (`load_jis_font` + Shinonome on `feat/cellular`, `load_cjk_font` +
  Fusion Pixel once `feat/fusion-font` is merged). When building from the
  integration branch, adapt the marked block in
  `src/bin/harbour-cellular.rs` the same way as the shell's, and build
  from that tree so the Fusion font assets are the ones packaged.
- The packaged unit runs `/usr/bin/harbour-celld`; the postinst removes a
  hand-installed `/etc/systemd/system/harbour-celld.service` (which used
  `/usr/local/bin`) because it would shadow the packaged unit.
- `99-harbour-cell.rules` encodes the AT port as USB interface 02 of the
  modem's rewritten identity `2c7c:0125`. Verify it against the device's
  existing rule on first install. Never reflash stock EC25/EG25-G
  firmware and never touch the module's usbid/usbcfg (smaller NAND =
  brick).
- The daemon's state (`/var/lib/harbour-cell/cell.db`) is shared by both
  frontends and survives reinstalls; the deb does not touch it.
