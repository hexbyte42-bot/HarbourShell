# Harbour Radio

Internet radio and RSS podcasts for the Cardputer Zero (320 x 170 screen).
Built into Harbour Shell; uses its existing SDL app contract, theme colours,
hardware key aliases and mouse handling. The interface is a two-colour tuner
and cassette deck, rather than a generic list of web links.

## Controls

| Key | Radio | Podcasts |
| --- | --- | --- |
| Tab | Switch to Podcasts | Switch to Radio |
| Up/Down | Change band (tuner); pick station (list) | Pick show / episode |
| Left/Right | Tune previous / next station | — |
| Enter | Start current station | Open show / play episode |
| Space | Pause / resume current audio | Pause / resume |
| Ctrl+F | Search stations by name | Search Apple Podcasts catalogue |
| L | Toggle tuner / station list | — |
| B | Saved radio stations | Toggle subscriptions |
| P | Pin / unpin station favourite | Subscribe / unsubscribe to podcast |
| R | Refresh results | Refresh shows / selected feed |
| Backspace / Esc | Return to tuner | Return to shows |
| Alt | App menu (including Stop Audio) | App menu |

Mouse: click the dial to tune, click the knob to play, use the wheel to select,
double-click a list item to open it. The top-right panel switches modes.

## How it works

- Radio Browser provides a community-run station directory. Bands are
  Ambient, Electronic, Jazz, Classical, World, Talk, and local Favorites.
  The app searches up to 60 stations per request, filters broken or invalid
  streams and displays codec/bitrate where available.
- Apple iTunes Search API provides podcast **discovery only**. Choosing a
  podcast reads its public RSS 2.0 feed and presents up to 60 episodes with
  HTTP(S) enclosures. Select an episode to stream.
- Network requests run on worker threads via curl; playback is an ffplay child
  process owned by a process-wide audio deck. It continues when you close
  Radio or switch to another launcher app. Use Radio > Stop audio to stop it.
- Favorite stations and podcast subscriptions are local to the device:
  ~/.config/harbour/radio.json. Deleting the app does not remove them.
  Searching, changing bands or reopening Radio does not interrupt playback.
- HARBOUR_MOCK=1 supplies fake catalogue entries for offline UI preview, and
  deliberately disables playback. It does not use real stations.
- No extra Rust dependencies are introduced.

## Requirements

Device needs internet access, curl and ffplay (the existing Browser and Music
apps already rely on curl and ffplay respectively). Some stations use codecs or
HLS formats that the installed ffplay may not support. A directory entry is
not a guarantee the stream is reachable at the time of playback.

## Current limits / follow-up

- No Apple account login, iTunes library sync or Apple Podcasts listening
  history sync. Apple's public Search API is a catalogue, not an account API.
- RSS 2.0 audio enclosures only; Atom / non-UTF-8 feeds are not supported yet.
- Episodes stream; there is no offline download, resume position, scrubbing or
  cross-device subscription sync in this first version.
- Playback is process-wide only for this app. Existing Music owns a separate
  ffplay process; a future shared media service can arbitrate between apps.
- No per-station stream health diagnostics beyond failure / end detection.
- Real Cardputer audio, startup latency and stream compatibility still need
  device testing. Unit tests cover directory and RSS parsing, query encoding,
  favourite-band state and stale async results.

References:
- Radio Browser API: https://docs.radio-browser.info/
- Apple Search API: https://performance-partners.apple.com/search-api
