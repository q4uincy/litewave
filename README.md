# litewave

A tiny, fast, native music player. Rust + [egui](https://github.com/emilk/egui) + [rodio](https://github.com/RustAudio/rodio). No browser engine, no web view.

## Features

- Plays local files (mp3, flac, ogg, wav, m4a, aac); background folder scan with tag reading
- Virtualized track list and instant search
- **iPod mode**: LCD menus (Music > Artists / Albums / Songs, Now Playing, Settings) and a click wheel you can drag, scroll, or click
- Seek, volume, shuffle, repeat, previous-track history
- Drag and drop a folder or files onto the window
- **Optional Spotify** (build feature): browse your playlists, liked songs, saved albums and followed artists in the iPod menus, with built-in playback

## Build

Requires a stable [Rust toolchain](https://rustup.rs).

```sh
# Linux: audio headers (add libssl-dev for the spotify feature)
sudo apt install pkg-config libasound2-dev

cargo run --release                      # local player + iPod mode
cargo run --release --features spotify   # plus Spotify
```

## iPod mode

Click **iPod mode** in the classic view (or leave it enabled in Settings > Classic view to switch back).

| Input | Action |
|---|---|
| Drag around the wheel / mouse scroll / Up, Down | Move selection (adjusts volume on Now Playing) |
| Wheel center / Enter | Select |
| MENU zone / Esc / Backspace | Back |
| Wheel right / left zones, Right / Left | Next / previous |
| Wheel bottom zone / Space | Play / pause |

## Spotify (experimental)

1. Build with `--features spotify`.
2. Create an app at <https://developer.spotify.com/dashboard> and add the redirect URI `http://127.0.0.1:8898/callback`.
3. In litewave open **Spotify** (classic view) or Settings > Spotify account, paste the app's Client ID, and click **Connect**.

Things to know:

- Each person uses their own Client ID. Spotify's Development Mode limits an app to 5 users, and the app owner needs Premium.
- Playback needs a Premium account. It uses [librespot](https://github.com/librespot-org/librespot), an unofficial client library, so Spotify can change or block it at any time.
- In Development Mode the Web API only returns tracks for playlists you own or collaborate on.
- The refresh token is stored in plain text in your config directory (`litewave/config.json`). Use Settings > Disconnect Spotify to remove it.
- Spotify volume follows your system volume; the litewave volume control affects local files only.

## Layout

```
src/
  main.rs        window setup
  app.rs         classic UI, queue logic, Spotify connect flow
  app/ipod.rs    iPod menus and click wheel
  player.rs      rodio player + router between local and Spotify playback
  library.rs     background folder scan + tag reading
  config.rs      JSON settings in the OS config directory
  spotify/       Web API client, PKCE login, librespot engine (feature-gated)
```

## Roadmap

- [ ] OS media keys (`souvlaki`)
- [ ] Gapless playback
- [ ] Album art
- [ ] Spotify volume control
- [ ] Packaged releases via GitHub Actions

## License

MIT
