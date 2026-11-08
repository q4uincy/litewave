# litewave

A tiny, fast, native music player. Rust + [egui](https://github.com/emilk/egui) + [rodio](https://github.com/RustAudio/rodio). No browser engine, no web view.

## Features

- Plays local files (mp3, flac, ogg, wav, m4a, aac); background folder scan with tag reading
- **Themes**: presets, your own colors, and a wallpaper image behind the whole app; share themes as a small `.json` file
- Virtualized track list and instant search
- **iPod mode**: LCD menus (Music > Artists / Albums / Songs, Now Playing, Settings) and a click wheel you can drag, scroll, or click
- Seek, volume, shuffle, repeat, previous-track history
- Drag and drop a folder or files onto the window
- Cover art from your files' embedded pictures and from Spotify, cached in small thumbnails
- **Optional Spotify** (build feature): browse your playlists, liked songs, saved albums and followed artists in the main window or the iPod menus, with built-in playback

## Build

Requires a stable [Rust toolchain](https://rustup.rs).

```sh
# Linux: audio headers (add libssl-dev for the spotify feature)
sudo apt install pkg-config libasound2-dev

cargo run --release                      # local player + iPod mode
cargo run --release --features spotify   # plus Spotify
```

## Themes

Open **Themes** in the sidebar. Pick a preset, tweak the accent/background/panel/text colors, set panel opacity, wallpaper dim and roundness, and choose a wallpaper image (png, jpg, webp). You can also drop an image onto the window to use it as the wallpaper.

**Save theme** writes `<name>.json` (and copies the wallpaper next to it) to the themes folder (**Open themes folder** in the window). To share a theme, send those two files; to install one, drop the `.json` onto the window or copy both files into the themes folder and press **Reload**.

```json
{
  "name": "Sakura Night",
  "author": "you",
  "bg": "#150f13",
  "panel": "#201519",
  "text": "#fbe9f0",
  "subtext": "#b99aa8",
  "accent": "#ff7eb6",
  "panel_alpha": 190,
  "dim": 130,
  "rounding": 10,
  "background": "sakura-night.jpg"
}
```

Every field is optional. Wallpapers are downscaled to at most 1280x720 when loaded (about 4 MB of RAM) and only still images are supported, which keeps memory use low.

## iPod mode

Click **iPod mode** in the sidebar. The window shrinks to just the iPod (borderless, draggable by the strip on top) with **Minimize**, **Full view** (back to the normal window) and **Close** buttons. Settings > Skin switches between Silver (Y2K), Black, Classic and Pink bodies. The current song's cover shows behind the menu and on Now Playing.

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
- If a track will not play, the Spotify window shows the last librespot message and has an **Open log folder** button (`librespot.log`). Free accounts cannot stream through librespot.

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
