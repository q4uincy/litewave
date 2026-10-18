# litewave

A tiny, fast, native music player for local files. Rust + [egui](https://github.com/emilk/egui) + [rodio](https://github.com/RustAudio/rodio). No browser engine, no web view.

## Features

- Scans a folder in the background (mp3, flac, ogg, wav, m4a, aac) and reads tags with `lofty`
- Virtualized track list: stays smooth with tens of thousands of tracks
- Instant search across title, artist, and album
- Seek bar, volume, shuffle, repeat (off / all / one), previous-track history
- Drag and drop a folder or files onto the window
- Remembers last folder, volume, shuffle, and repeat

## Build

Requires a stable [Rust toolchain](https://rustup.rs).

```sh
# Linux only: audio headers
sudo apt install pkg-config libasound2-dev

cargo run --release
```

The release profile uses LTO and `strip` for a small binary.

## Shortcuts

| Key | Action |
|---|---|
| Space | Play / pause |
| Left / Right | Seek -5s / +5s |
| Up / Down | Volume +/- 5% |
| N / P | Next / previous |
| S | Toggle shuffle |
| R | Cycle repeat |

Shortcuts are disabled while the search box is focused.

## Layout

```
src/
  main.rs      window setup
  app.rs       UI, input handling, playback queue logic
  player.rs    rodio wrapper with its own position clock
  library.rs   background folder scan + tag reading
  config.rs    JSON settings in the OS config directory
```

## Roadmap

- [ ] OS media keys and now-playing integration (`souvlaki`)
- [ ] Gapless playback
- [ ] Album art
- [ ] Playlists and a persistent queue
- [ ] Optional Spotify Web API controller panel
- [ ] Packaged releases via GitHub Actions

## License

MIT
