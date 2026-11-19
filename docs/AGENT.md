# Agent

Double-tap **Tab** (two taps within 300 ms, no other key between) opens the chat where Tab means nothing to the
screen in front. Where Tab is the screen's own key (Terminal, Text Editor, Browser, Settings > Wi-Fi password and
the tabbed apps: Monitor, Infoscope, Interceptor, Timer, Voice Memo, Radio) Tab Tab is two Tabs; **Alt+Tab** opens
the chat there and everywhere else, and so does Harbour menu > Agent. Tab Tab, Alt+Tab or Esc closes it; the app behind keeps running and
the tools act on it. Hold Esc leaves everything, as in other apps. On Home a single Tab (grid/list) waits
300 ms to be sure it is not the first half of a double tap (no wait while the Agent is switched off).
Hold Tab 1 s still locks.

## Settings > Agent

* **Enabled**: switch the double tap on or off.
* **Provider** (Left/Right or Enter): the active provider name from your config; **Model** and **Status** (the result
  of the last call: `OK`, `Error: HTTP 401`, ...) are shown read-only. No keys or URLs are ever shown there.
* **Connect**: the device's Wi-Fi / Ethernet address and a QR code of this guide
  (<https://github.com/kurogedelic/HarbourShell/blob/main/docs/AGENT.md>).

## Configure a provider

Copy `docs/agent.example.yaml` to `~/.config/harbour/agent.yaml`, `chmod 600` it, fill in a model, and start the
launcher with the key in its environment (e.g. `GROQ_API_KEY`), or put `api_key` in the file.
`agent.yaml` is read first; `~/.config/harbour/agent.json` (`docs/agent.example.json`, `//` comments allowed)
is the fallback when there is no YAML file. Provider and Auto-run are also in the chat's menu (Alt).
Chats and the Agent's small state file live in `~/.local/share/harbour/agent/`.

Source: `src/launcher/agent/` (config, provider trait + openai/anthropic wire formats, curl transport, tools, history,
input trait for a future microphone, the chat screen); `src/launcher/qr.rs` (the QR encoder, byte mode, level L,
versions 1 to 5).

## Muse: installing the SDK on the device

The Muse provider needs the `musegadget` service from the Muse Linux Device SDK. Short version (full steps in
[MUSE.md](MUSE.md) section 5 and the README):

- `bash install.sh` asks for the SDK token itself (hidden); never put it on the command line or in a config file.
- **It takes several minutes** on the Pi (Python packages, Bluetooth, pairing). Wait for it to finish.
- The installer asks whether Muse may run commands as the installing account. With `pi` that includes sudo, so
  for anything but a throwaway device run `bash install.sh --run-as <small account>` instead.
- Afterwards `sudo musegadget doctor`, then `musegadget send-user-msg --wait 60 "hello"` as `pi`.
- Remove with `bash install.sh --uninstall`.
