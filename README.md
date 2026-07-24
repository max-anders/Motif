# Motif

Open-source music sketchpad: piano roll, playlist, soft-synth piano, and headless CLAP/VST3 instruments over a real audio engine.

## Status

**Early but usable sketchpad** (2026-07-24): playlist + piano roll editing, transport, cpal playback (built-in piano and/or CLAP/VST3), plugin manager, `project.json` save/load, and Settings for shortcuts, themes, and plugin scan. Not a full DAW.

| Area | State |
|---|---|
| Playlist (tracks / MIDI clips) | Working |
| Per-track instruments (piano / CLAP / VST3) | Working (headless; no plugin GUI yet) |
| Plugin manager (scan / cache) | Working (`plugin_cache.json` in CWD) |
| Piano roll (notes + key audition) | Working |
| Transport + loop + BPM | Working |
| Soft piano + plugin mix (`cpal`) | Working (UI continues if device open fails) |
| Project save/load | Working (`project.json` in CWD) |
| Settings (shortcuts + themes + plugins) | Working (`settings.json` in CWD) |
| Plugin editor GUI / VST2 / mixer / export | Not started |
| Tests / undo / samples | Not started |

It is **free and open source from minute zero**: public from the first commit, not opened up later after something polished existed behind closed doors.

## Open-source philosophy

This is a personal experiment being built in the open from day one — not a product that went public after a private beta.

The repo is public from the start. No closed preview, no "open sourcing later" roadmap. If you are reading this early, you are seeing it almost as it is being born.

The early versions may be rough, experimental, or "vibe coded." That is intentional. The goal is to explore ideas, test workflows, and see what kind of creative tool can emerge.

This is not a promise that every feature request will be implemented, and it is not a community-driven product roadmap.

The project has a vision and a direction. I develop it **at my own pace** — no release schedule, no feature backlog driven by outside pressure.

If you want to help without writing code: **consider donating**. Money does not buy roadmap votes or priority features, but it does help keep this sustainable as a free, open project built on my time.

If you think something can be improved:

- Improve it.
- Experiment with it.
- Submit changes.
- Share your ideas.

Contributions are welcome, but acceptance is not guaranteed. A contribution should fit the goals and philosophy of the project.

If you disagree with the direction or want to take the idea somewhere else:

**Fork it. Build your own version. Experiment.**

That is one of the main reasons this project is open source. The code is here to be used, studied, modified, and expanded.

If you ship a derivative, keep it open source under **GPL-3.0** (same as this repo) and link back to [Motif](https://github.com/max-anders/Motif) in your README.

Credit for contributions is appreciated and will be given where appropriate, but the priority is creating something useful and interesting.

## License

Motif is licensed under the [GNU General Public License v3.0 or later](LICENSE).

## Setup

Install Rust if needed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Linux may also need egui native deps (Wayland/X11 + OpenGL or wgpu stack) and audio (ALSA/PipeWire). On Arch:

```bash
sudo pacman -S --needed base-devel pkgconf openssl libxkbcommon wayland alsa-lib
```

## Run

```bash
cd motif
cargo run
```

Release build (snappier UI):

```bash
cargo run --release
```

## Controls

### Playlist (default view)

- **Left-click empty lane** - create a MIDI clip (4 beats)
- **Left-click clip** - select clip
- **Double-click clip** - open piano roll for that clip
- **Ctrl/Cmd + left-click clip** - toggle multi-select
- **Drag clip body** - move selected clip(s) on timeline
- **Shift + drag clip body** - duplicate selection, then move the copies
- **Drag clip edges** - resize clip length
- **Delete / Backspace / Ctrl+X** - remove selected clip(s)
- **Ctrl/Cmd+D** - duplicate selected clip(s) to the right by selection length (remappable)
- **Add track** - menu: Built-in Piano or a scanned CLAP/VST3 instrument
- **Right-click track header** - change instrument (searchable list)
- **Left-click / drag ruler** - move playhead (snapped to 1/16)
- **Shift + left-click** or **right-click empty timeline** - move playhead
- **Wheel** - scroll; **Shift+Wheel** - horizontal scroll; **Ctrl/Cmd+Wheel** - zoom time

### Piano roll (clip editor)

- **Back to playlist** or **Escape** - return to arrangement
- **Left-click empty grid** - add a note (1 beat, snapped to 1/16)
- **Left-click note** - select note
- **Drag empty grid** - marquee multi-select
- **Drag note body** - move selected note(s) (pitch/start)
- **Shift + drag note body** - duplicate selection, then move the copies
- **Drag left/right edge** - resize note
- **Press / drag piano keys** - audition pitches (active track instrument)
- **Left-click / drag ruler** - move playhead (mapped to arrangement time)
- **Shift + left-click** or **right-click empty grid** - move playhead
- **Delete / Backspace / Ctrl+X** - remove selected note(s)
- **Ctrl/Cmd+D** - duplicate selected note(s) to the right by selection length (remappable)
- **Right-click note** - delete note
- **Wheel** - scroll vertical; **Shift+Wheel** - horizontal; **Ctrl/Cmd+Wheel** - zoom time; **Alt+Wheel** - zoom keys

### Transport

- **Play / Pause** - play arrangement notes through each track's instrument (playhead loops)
- **Stop** - stop, silence, and return to start
- **Space** - play/pause (factory default; remappable)
- **Ctrl/Cmd+S** - save `project.json` (factory default; remappable)
- **Ctrl/Cmd+O** - load `project.json` (factory default; remappable)
- **Save / Load** buttons - same as the shortcuts above
- **Settings** - themes, shortcut remapping, Plugin Manager (Rescan / extra paths); saved to `settings.json` + `plugin_cache.json`
- **Escape** - leave piano roll or Settings (factory default; remappable)

## Plugins (CLAP / VST3)

Motif hosts **CLAP** and **VST3** instruments in-process via [truce-rack](https://crates.io/crates/truce-rack). There is **no plugin editor window** yet (headless audio only). **VST2 is not supported.**

- Open **Settings → Plugin Manager → Rescan** to refresh the instrument list from standard OS plugin directories (plus any extra paths you add).
- **Add track** or right-click a track header to pick Built-in Piano or a scanned instrument.
- Good Linux smoke targets: native **Vital** or **Surge XT** (CLAP/VST3).
- **Serum on Linux** usually needs a Windows VST3 bridge such as yabridge; Motif does not bundle or configure that — if the bridged VST3 appears in a scan path, Motif may load it, but support is best-effort and crashes can take the app down (in-process host).

## Architecture

```text
UI (egui)
  -> Project model (tracks + instruments, clips, notes, tempo, loop)
  -> PlaylistUi / PianoRollUi (shared timeline navigation)
  -> DawEngine trait
       -> AudioEngine (UI clock + cpal mix of piano / hosted plugins)
       -> MockEngine (silent fallback / tests)
  -> PluginCatalog (scan cache; load off the audio thread)
```

If the audio device fails to open, Motif keeps the transport UI and reports it in the status line.

## Pitch range

Full MIDI range (0-127). Snap grid: quarter-beat (1/16 in 4/4).
