# Motif

Open-source music sketchpad: piano roll, playlist, and a soft-synth piano over a real audio engine.

## Status

**Early but usable sketchpad** (2026-07-24): playlist + piano roll editing, transport, cpal soft-piano playback, `project.json` save/load, and Settings for shortcut remapping plus editable/saveable themes. Not a full DAW.

| Area | State |
|---|---|
| Playlist (tracks / MIDI clips) | Working |
| Piano roll (notes + key audition) | Working |
| Transport + loop + BPM | Working |
| Soft piano audio (`cpal`) | Working (UI continues if device open fails) |
| Project save/load | Working (`project.json` in CWD) |
| Settings (shortcuts + themes) | Working (`settings.json` in CWD) |
| Tests / undo / samples / mixer / export | Not started |

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

That is one of the main reasons this project is open source.

The code is available to be:

- used,
- studied,
- modified,
- expanded,
- and taken in new directions.

Credit for contributions is appreciated and will be given where appropriate, but the priority is creating something useful and interesting.

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
- **Left-click clip** - open piano roll for that clip
- **Ctrl/Cmd + left-click clip** - toggle multi-select (does not open)
- **Drag clip body** - move selected clip(s) on timeline
- **Shift + drag clip body** - duplicate selection, then move the copies
- **Drag clip edges** - resize clip length
- **Delete / Backspace / Ctrl+X** - remove selected clip(s)
- **Ctrl/Cmd+D** - duplicate selected clip(s) to the right by selection length (remappable)
- **Add track** - new track lane
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
- **Press / drag piano keys** - audition pitches (soft piano)
- **Left-click / drag ruler** - move playhead (mapped to arrangement time)
- **Shift + left-click** or **right-click empty grid** - move playhead
- **Delete / Backspace / Ctrl+X** - remove selected note(s)
- **Ctrl/Cmd+D** - duplicate selected note(s) to the right by selection length (remappable)
- **Right-click note** - delete note
- **Wheel** - scroll vertical; **Shift+Wheel** - horizontal; **Ctrl/Cmd+Wheel** - zoom time; **Alt+Wheel** - zoom keys

### Transport

- **Play / Pause** - play arrangement notes through the soft piano (playhead loops)
- **Stop** - stop, silence, and return to start
- **Space** - play/pause (factory default; remappable)
- **Ctrl/Cmd+S** - save `project.json` (factory default; remappable)
- **Ctrl/Cmd+O** - load `project.json` (factory default; remappable)
- **Save / Load** buttons - same as the shortcuts above
- **Settings** - open Settings; remap shortcuts and edit/save color themes (saved to `settings.json`)
- **Escape** - leave piano roll or Settings (factory default; remappable)

## Architecture

```text
UI (egui)
  -> Project model (tracks, clips, notes, tempo, loop)
  -> PlaylistUi / PianoRollUi (shared timeline navigation)
  -> DawEngine trait
       -> AudioEngine (UI clock + cpal output + soft piano)
       -> MockEngine (silent fallback / tests)
```

If the audio device fails to open, Motif keeps the transport UI and reports it in the status line.

## Pitch range

Full MIDI range (0-127). Snap grid: quarter-beat (1/16 in 4/4).
