# Meta Muse integration

Status: researched 2026-11-18 from the public repo. Integrated into the Agent as provider kind `muse` (see §4). **Nothing here has been run against a real Muse account** (no token yet); the tests use fake sockets.

## 1. What the Muse Linux Device SDK is

Sources: [muse-gadget-sdk](https://github.com/facebookincubator/muse-gadget-sdk) (Apache-2.0), [`linux/README.md`](https://github.com/facebookincubator/muse-gadget-sdk/blob/main/linux/README.md), [`linux/AGENTS.md`](https://github.com/facebookincubator/muse-gadget-sdk/blob/main/linux/AGENTS.md), [`linux/src/musegadget/`](https://github.com/facebookincubator/muse-gadget-sdk/tree/main/linux/src/musegadget), [gadgets.muse.ai](https://gadgets.muse.ai), [SDK terms](https://gadgets.muse.ai/sdk-terms). Community: [Discord](https://discord.gg/3bhjCkZdd6).

* **Python >= 3.9 package `musegadget`**, deps `cryptography>=3.3.2`, `websockets>=15`; optional `sounddevice` (+ PortAudio) for live voice, `libopus0` for video, a native wake-word lib ("Hey Muse", ~0.4 % of a core, off by default).
* Installed by `install.sh --sdk-token mgst_…` into `/opt/musegadget` as the **`musegadget` systemd service**. Needs sudo, Bluetooth LE, and the Muse phone app for pairing (10-minute window).
* **The gadget is a peripheral of the user's Muse account, not a standalone LLM client.** Pairing is over BLE via the phone app; afterwards the service holds an encrypted Noise session over HTTPS/WebSocket (TCP 443) to Muse's cloud VM and re-registers after reboots. The SDK token is stored root-only in `/var/lib/musegadget/sdk_token`.
* **Muse → device**: Muse (running in the cloud, driven by what the user says in the phone app) can invoke *commands* on the gadget: built-ins `system.run`, `file.read`, `file.write`, `device.health`, plus **custom commands** = one JSON file per command in `/etc/musegadget/commands.d/` (name, description, typed `required`/`optional` params, `exec` argv, `timeout_ms`). Params arrive as JSON on stdin; stdout JSON becomes the result. Output cap 96 KiB, max 4 concurrent, no retry after a dropped session. Reserved prefixes: `system. file. device. link.`. Read at service start only.
* **Device → Muse (text)**: the service exposes a local Unix socket `/run/musegadget/musegadget.sock` (root + run-as account's group). One JSON line in: `{"message", "session_id"?, "image"?, "wait"?}`; one line out: `{"ok", "reply", "partial", "detail", "error", "sent"}`. With `wait` (1..600 s) the service returns Muse's **text answer**. `session_id` is meant to select/create a **side chat**, but **on the real device a side chat id (e.g. `harbour`) fails with "could not follow Muse's replies (HTTP 400); the message was not sent"** (reproduced with `musegadget send-user-msg --session-id harbour`); only the main chat (`session_id` omitted or `""`) works, so Harbour talks to the main chat (it shares the phone chat). Images (JPEG/PNG/GIF/WebP ≤ 2 MiB) can be attached. Only one waiting request per chat at a time. (`musegadget send-user-msg --wait 60 "…"` and `musegadget chat` are thin clients of this socket.)
* **Device → Muse (voice)**: `musegadget voice-call --vm-id … --live` (PortAudio mic/speaker, half-duplex by default, PCM16 24 kHz mono) or WAV in/out; optional video (~1 Mbit/s, SRTP). Needs `--vm-id`; replaces any other active call.
* Replies are **text only**; TTS audio (MP3) exists only for replies Muse gave in voice mode (`stream_tts`).
* Protocol doc for the gadget side (`protocols/DEVICE.md`) only specifies `link.register` identity (`device_family: "gadget"`, `model_id`) and the optional `device.guide` command (markdown ≤ 4096 B, a device self-description Muse can read).

## 2. Terms that matter ([SDK terms](https://gadgets.muse.ai/sdk-terms))

* Token: **personal, non-commercial**, your own Muse account. Do not publish/share the token except embedded in a device; ≤ 50 devices per token. No embedding in devices you **sell, advertise, list in a marketplace/app store or tie to a promotion**.
* Don't imply the device is made/endorsed by Meta (trademark). The SDK's own rule: say "Muse", never "Hatch", in user-visible text.
* Privacy: no access to anyone else's Muse account; if you give a built device to someone, disclose what it does with their data; don't retain/transmit their prompts/responses beyond need.
* "As is", unsupported, may change or be withdrawn; Meta may revoke tokens. No rate limits published.
* The SDK is Apache-2.0 (redistributable with notices), but that licence grants no rights to tokens.

**Consequence for Harbour (MPL-2.0, public distribution):** Harbour must **not bundle the SDK, a token, or ship a "Muse built in" product**; it only offers an optional, off-by-default client for a locally installed `musegadget` service the *user* sets up with their *own* token. Keep wording neutral ("works with a Muse gadget you set up yourself"), no Meta logos.

## 3. What we can and cannot do

| Idea | Possible? | How / caveat |
|---|---|---|
| Type in the Agent, get Muse's answer | **Yes** | local socket, `wait`, **main chat** (verified end to end on the device); side chats do not work so far |
| Muse controls Harbour (list/launch apps, set settings) | **Yes** | custom commands → `harbour-muse-cmd` → launcher socket → tools adapter |
| Muse as a tool-calling LLM backend (our tools via function calling) | **No** | not an LLM API; Muse decides itself which command to call |
| Streaming tokens / system prompt / model choice | **No** | one final text per wait |
| Voice on device (mic → Muse → speaker) | **Partly** | `voice-call --live` exists, Python + PortAudio, needs `--vm-id`; ALSA/ES8389 and RAM untested; competes with our Voice Memo for the mic |
| Wake word "Hey Muse" | Maybe | needs a compiler build; CPU/RAM impact unknown |
| Muse sees the 320x170 screen / camera | Partly | `--image` attach (JPEG/PNG ≤ 2 MiB) could send a screenshot or photo |
| Offline use | **No** | cloud session required |
| Bundle the SDK/token in the Harbour release | **No** | terms (non-commercial, token not shareable) |

## 4. Design (implemented in `src/launcher/agent/muse.rs`)

```
Agent chat ─► MuseProvider ──JSON line──► /run/musegadget/musegadget.sock ─► musegadget (Python svc) ═► Muse cloud ◄─ phone app
                                                                                     │ custom command harbour.<tool>
Agent screen (tools::execute + confirmation card) ◄─ RemoteCall ◄─ muse-cmd.sock (0600) ◄─ harbour-muse-cmd ◄─ exec ┘
```

* **No Python bridge process of ours.** The SDK already is the supervised service (systemd); its socket protocol is one JSON line, so the Rust client is native (std + serde_json, no new crates).
* **Agent -> Muse:** `kind: "muse"` in `agent.json` builds `MuseProvider` (a normal `Provider`). Muse holds the conversation (the **main** Muse chat: side chats fail on the device, see §1), so only the last user line is sent; the answer is one `TextDelta`; no tool call is ever emitted. `wait_secs` (default 60, max 600) bounds the wait.
* **Muse -> Harbour:** every tool of `tools::specs()` becomes the Muse command `harbour.<tool>` (generated by `command_specs()`; the files in `tools/muse_bridge/commands.d/` must match, a test checks it; regenerate with `HARBOUR_WRITE_MUSE_COMMANDS=1 cargo test --lib muse::tests::dropin`). Muse runs `tools/muse_bridge/harbour-muse-cmd <tool>` (python3 stdlib shim, params as JSON on stdin), which forwards to the launcher's socket `$XDG_RUNTIME_DIR/harbour/muse-cmd.sock` (dir 0700, socket 0600). The Agent screen takes each call as a `RemoteCall`, runs it through `tools::execute` and **the same confirmation card** as a model's tool call ("Muse wants to: ..."; skipped only with Auto-run on). Read-only tools run directly. The call is answered with an error if you decline, or after 55 s, and a call whose socket side has given up is never run late.
* Commands work **only while the Agent screen is open** (the card needs a visible screen); otherwise Muse gets "is the Agent screen open on the device?". The command socket is started when the config has a `muse` provider.
* **Token handling:** the launcher never sees the token (root-only `/var/lib/musegadget/sdk_token`). `agent.json` has no token key (`muse_sdk_token_env` is deliberately not implemented).
* `agent.json` entry (every key under `muse` optional, defaults shown):

```json
"muse": {"kind": "muse", "muse": {"socket": "/run/musegadget/musegadget.sock", "wait_secs": 60, "commands_socket": "/run/user/1000/harbour/muse-cmd.sock"}}
```
  `session_id` is omitted by default = the main Muse chat (`""` means the same). An explicit id (`"session_id": "side-1"`) is still passed through, but side chats do not work on the device so far.
* Error states (`MuseError::user_message()`, shown on the status row): **SdkMissing** (socket absent/refused: SDK not installed or stopped); **NoPermission** (user not in the service's group); **Offline** ("not connected to the Muse": unpaired, no/invalid token, or no network; the socket cannot tell which, so it points at `musegadget doctor`); NoAnswer; Uncertain; Timeout; Remote; Protocol.
* **Safety:** Muse's LLM can now act on the device, but only through the Harbour tools above (no shell) and with your confirmation. The SDK's own built-ins `system.run`, `file.read`, `file.write` stay available to Muse and run as the install account: answer **no** to broad access at install, or use `--run-as` a restricted account.
* Resources: the Python service on this device (352 MB RAM, ~120 MB available) is unmeasured. Voice, video and wake word are not wired in and not recommended until measured.

## 5. What the user has to do (nothing was run on the device)

Read-only check of the device (2026-11-18): Debian 13, Python 3.13.5, `python3-cryptography` 43.0.0, `websockets` **missing**, `pip`/`bluetoothctl`/`hciconfig` present, bluetooth service active, `libopus0` installed, `libportaudio2` missing, `/opt/musegadget` and `/run/musegadget` absent, 352 MB RAM (120 MB available, swap 111 MB used), 21 GB free disk. User `pi` is in group `pi`, which is the run-as account's group for the socket.

1. Get a token: sign in at muse.ai → Settings → SDK tokens (`/settings/sdk-tokens`); read the [SDK terms](https://gadgets.muse.ai/sdk-terms). Never paste the token into this repo, `agent.json`, shell history or chat. (Use `read -s TOK` and pass it once.)
2. On the device (needs sudo; yours to run, I did not):
   ```bash
   curl -fsSL https://raw.githubusercontent.com/facebookincubator/muse-gadget-sdk/main/linux/install.sh -o install.sh
   less install.sh                       # read it first
   bash install.sh          # asks for the token securely (hidden); do not pass it on the command line
   sudo musegadget doctor
   ```
   The installer pulls `websockets` itself (not verified); it asks before giving Muse access to the account.
3. Pair: Muse app → Settings → Devices → Developer mode → Add device (`MuseGadgetXXXXXX`). Re-pair: `sudo musegadget pair`.
4. Install our command shim (optional, for Muse → Harbour; copy the files from the repo to the device first):
   ```bash
   sudo install -m 755 harbour-muse-cmd /usr/local/bin/
   sudo install -d -m 755 /etc/musegadget/commands.d
   sudo install -m 644 -o root -g root commands.d/*.json /etc/musegadget/commands.d/
   sudo systemctl restart musegadget
   sudo journalctl -u musegadget | grep commands     # the harbour.* commands should be listed
   ```
5. Smoke test the SDK socket: `musegadget send-user-msg --wait 60 "hello"` (as `pi`; if it says no access, `pi` is not in the service's group).
6. `cp docs/agent.example.json ~/.config/harbour/agent.json` (keep/add the `"muse": {"kind": "muse"}` entry, `chmod 600`), restart the launcher, double-tap Tab, Alt menu → Provider → `muse`, ask something. Ask "open Files" to see the confirmation card for a Muse command.

## 6. Untested / open questions
* Everything against live Muse (pairing, replies, side-chat creation with id `harbour`, `wait` latency).
* Reply quality/format for the 320x170 screen (Muse may answer long or in markdown).
* `commands.d` must be root-owned when the service runs as root; `exec` runs as the install account, same user as the launcher, so `$XDG_RUNTIME_DIR` may not be set in the service's environment: the shim falls back to `/run/user/<uid>`.
* Memory/CPU of the Python service on the CM0; Bluetooth coexistence with the Wi-Fi/Ethernet setup.
* Voice mode through our ALSA path (we did not try it; the SDK opens the mic via PortAudio itself).
* Tests: `cargo test --lib muse` (10 tests in `agent/muse.rs`, 1 in `agent/app.rs`): fake link, real Unix sockets, command server, confirmation flow, drop-in files vs tool list. All pass; they never touch Muse.
* Muse may call a command that needs a confirmation card while you are away: it times out after 55 s with an error (nothing runs).
* `tools/muse_bridge/device-guide.md` (not written by this change) names tools `id`/`key` that differ from the real tool arguments (`name`); check it before using it as a `device.guide`.
