# Demo recording

The launcher can run a script (keys, mouse, palette switches) by itself and record every frame it draws,
so the demo video needs no one at the keyboard. The same script runs on the Mac (mock) and on the device.

| File | What |
|---|---|
| `main.script` | the whole demo; includes `chapters/01..12_*.script` (Paint and the Japanese input method are not in it) |
| `record_mock.sh` | Mac: run `main.script` with `HARBOUR_MOCK=1`, record, write `out/demo.mp4` |
| `../tools/rec_to_mp4.sh` | raw frames -> integer-scaled mp4 (nearest neighbour, default 4x = 1280x680) |

## Script syntax
Tokens separated by commas or newlines, `#` starts a comment. Everything in docs/DESIGN.md "Test hooks" plus:
`theme:N[:invert]` (instant; N = 0 Amber, 1 Sky, 2 Mobile, 3 Terminal; config.json is rewritten),
`theme-ui:N` (walks Settings > Display > Theme with Left/Right; the Theme row must be selected),
`goto:Name` (Home selection jumps to that tile, so chapters do not depend on tile order),
`include:path` (relative to the including file). Scripts disable the screen-off timer.
Parts that only mean something on real hardware are marked `DEVICE ONLY` in the chapters.

## Recording
`harbour-shell --demo FILE --rec DIR [--rec-fps 15]` (or `HARBOUR_SCRIPT_FILE`, `HARBOUR_REC_DIR`, `HARBOUR_REC_FPS`).
`DIR/frames.rgb565` is the raw RGB565-LE 320x170 stream at a constant fps (the last frame is repeated while the
screen is still), `DIR/rec.txt` has the parameters. Recording does not change what is drawn or how keys behave.

Load options for the device (nothing is compressed there):

| Way | Device cost | Notes |
|---|---|---|
| A. raw file on tmpfs (`--rec /dev/shm/rec`), `scp` afterwards | memory 1.6 MB/s at 15 fps (108 KB x 15): 4 min = ~380 MB, so record per chapter | simplest, no network during the run (Wi-Fi stays out of the picture) |
| B. pipe over ssh: `HARBOUR_REC_FILE=/dev/stdout`, `ssh ... \| ffmpeg` | no disk; sshd + network load, a slow link can stall the launcher | stdout carries only frames (log lines go to stderr) |
| C. SD card file | avoid: card writes stall the UI | |

Recommended: A, one chapter at a time (`demo/chapters/NN_*.script`), then join the mp4 files.

## On the Mac (no device)
```
demo/record_mock.sh                       # -> demo/out/demo.mp4
demo/record_mock.sh demo/chapters/02_theme.script demo/out/theme.mp4 6
```
The mock shows placeholders for Wi-Fi/Bluetooth/battery, no microphone/camera, no network for the Browser.

## Auto-filming on the device (needs the owner's go-ahead at each point marked ASK)
1. ASK: deploy the build (`tools_build_arm64.sh`, `tools_make_bundle.sh`, `install_user.sh`). This session never deploys.
2. ASK: stop the service that owns the LCD (the display is single-owner): `systemctl --user stop harbour-shell`
   (unit name per `install_user.sh`; restart it afterwards). `sudo` is only needed if your install uses a system unit.
3. Run in the foreground on the device (over ssh, or on its own console):
   ```
   cd <install dir>    # assets/ must be next to it, or set HARBOUR_ROOT
   HARBOUR_FB=/dev/fb0 ./harbour-shell --demo demo/chapters/02_theme.script --rec /dev/shm/rec --rec-fps 15
   ```
   (use the DRM path of your build if the service normally uses it; `HARBOUR_FB` selects the LCD backend.)
   The script ends the process when it is done. A real keypress is still read (evdev), so keep hands off the keyboard.
4. Collect: `scp pi@<device>:/dev/shm/rec/frames.rgb565 rec/02.rgb565` (or pipe, way B), `ssh ... rm -r /dev/shm/rec`.
5. On the Mac: `tools/rec_to_mp4.sh rec/02.rgb565 out/02.mp4 4 15`, join chapters with `ffmpeg -f concat`.
6. ASK/restart: `systemctl --user start harbour-shell`.
Check privacy before publishing (SSID, IP, QR, chat history): memos/demo-video-plan.md section 6.
