# Motif

Experimental music sketchpad: piano roll, playlist, soft-synth piano, and CLAP/VST3 instruments (optional native editor windows) over a real audio engine.

## Status

**Early but usable sketchpad** (2026-07-24): playlist + piano roll editing with undo/redo, transport, cpal playback (built-in piano, CLAP/VST3, and imported audio sample clips), plugin manager + editor windows (Linux X11/XWayland), multi-project `.motif` save/open with recent list + recovery autosave, and Settings for shortcuts, themes, plugins, editing, and project prefs. Not a full DAW.

| Area | State |
|---|---|
| Playlist (tracks / MIDI + audio clips) | Working |
| Per-track instruments (piano / CLAP / VST3) | Working (load off UI thread) |
| Plugin manager (scan / cache) | Working (`plugin_cache.json` in CWD) |
| Plugin editor GUI | Working on Linux (X11 / XWayland); not Wayland-native |
| Piano roll (notes + key audition + copy/paste) | Working |
| Undo / redo (clips + notes) | Working (depth in Settings → Editing) |
| Transport + loop + BPM + metronome | Working (includes live CPU / buffer / xrun strip) |
| Performance view | Working (CPU graph + per-track DSP; Ctrl/Cmd+Shift+P) |
| Soft piano + plugin mix (`cpal`) | Working (UI continues if device open fails) |
| Project save/load | Working (`.motif` files; recent projects; recovery autosave) |
| Settings (shortcuts + themes + plugins + editing + project) | Working (`settings.json` in CWD) |
| VST2 / export | Not started |
| Platform support | **Linux** (developed & tested); Windows/macOS untested |

The repository is public from the first commit so progress is visible early — not a closed preview opened up later.

## Source and licensing

This is an experimental sketchpad I am building because existing DAWs do not let me customize the workflow I want — especially the piano roll and the sketch loop. The comparison point is not "the next Bitwig"; it is whether Motif becomes the fastest place for me to sketch melodies and arrange ideas.

The repository is **public while the project is in active development**. You can watch it evolve, report bugs, and share ideas. **Long-term licensing, distribution, and commercialization have not been decided yet.**

Early versions are rough. I iterate quickly in public; not every path is reviewed to production standards. This is not a promise that every feature request will land, and there is no community-driven product roadmap. I develop at my own pace.

**Using the code today:** No open-source license is attached yet. You may browse the source on GitHub, but copying, modifying, or redistributing it requires permission until a license is announced. If you want to experiment or contribute substantial work, open an issue or discussion first.

**Feedback and contributions:** Bug reports and ideas are welcome. Code contributions may be accepted case by case once licensing is clearer; ask before investing large patches.

## License

No license selected yet. Copyright (c) 2026. All rights reserved until stated otherwise here.

When a license is chosen, this section and a `LICENSE` file will be updated.

## Platform support

**Linux only for now** — developed and daily-tested on Arch (Wayland/X11). There is no Windows or macOS build in CI and no maintainer testing on those platforms yet.

The stack (egui, cpal, truce-rack) is cross-platform in principle, so ports may work with extra setup, but **non-Linux use is best-effort**. Bug reports and patches for other OSes are welcome; they are not the current focus.

## Setup

Install Rust if needed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Linux needs egui native deps (Wayland/X11 + OpenGL or wgpu stack) and audio (ALSA/PipeWire). On Arch:

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
- **Drag empty lane** - marquee multi-select
- **Left-click clip** - select clip
- **Double-click clip** - open piano roll for that clip (zoomed out to fit the clip)
- **Audio clips** - do not open piano roll (arrangement-only clips)
- **Ctrl/Cmd + left-click clip** - toggle multi-select
- **Drag clip body** - move selected clip(s) on timeline (same-track clips cannot overlap)
- **Shift + drag clip body** - duplicate selection, then move the copies
- **Drag clip edges** - resize clip length (stops at neighboring clips)
- **Delete / Backspace** - remove selected clip(s)
- **Ctrl/Cmd+C** - copy selected clip(s)
- **Ctrl/Cmd+X** - cut selected clip(s)
- **Ctrl/Cmd+V** - paste clips at the playhead (same tracks)
- **Ctrl/Cmd+D** - duplicate selected clip(s) to the right by selection length (remappable)
- **Ctrl/Cmd+Z** - undo last clip/note edit (remappable)
- **Ctrl/Cmd+Shift+Z** - redo (remappable)
- **Add track** - menu: Built-in Piano or a scanned CLAP/VST3 instrument
- **Import sample...** - import an audio clip (`wav/mp3/flac/ogg/m4a/aac`) onto the selected track at the playhead
- **M / S** on track header (or right-click **Mute** / **Solo**) - mute or solo a track; when any track is soloed, only soloed tracks play; otherwise muted tracks are silent
- **Right-click track header → Delete track** - remove the track and its clips (last track cannot be deleted; Ctrl/Cmd+Z restores a deleted track)
- **X** (while pointer is over a track header) - delete that track (remappable; same rules as context-menu delete)
- **Right-click track header** - open/close plugin editor (plugin tracks) or change instrument (searchable list)
- **Ctrl/Cmd+Shift+E** - open/close plugin editor for the selected track (remappable)
- **Shift+Q** - close a focused plugin editor window (remappable; default)
- **Left-click / drag ruler** - move playhead (snapped to 1/16)
- **Shift + left-click**, **right-click empty timeline**, or **right-click drag** on the timeline - move playhead
- **Wheel** - scroll; **Shift+Wheel** - horizontal scroll; **Ctrl/Cmd+Wheel** - zoom time
- **Scrollbars** - always-visible solid bars (drag to scroll)
- Track headers stay pinned on the left while the timeline scrolls horizontally (vertical scroll stays synced); the beat ruler stays pinned to the top

### Piano roll (clip editor)

- **Back to playlist** or **Escape** - return to arrangement
- **Open from playlist** - horizontal zoom starts at the zoom-out floor (whole clip in view); wheel zoom afterward as usual
- **Left-click empty grid** - add a note (1 beat, snapped to 1/16; rejected if it would overlap another note on the same pitch)
- **Left-click note** - select note
- **Ctrl/Cmd+A** - select all notes in the clip (remappable)
- **Shift+Up / Shift+Down** - move selected note(s) up/down one semitone (remappable; blocked/clamped if same-pitch overlap)
- **Ctrl/Cmd+Up / Down** - move selected note(s) up/down one octave (remappable)
- **Drag empty grid** - marquee multi-select
- **Drag note body** - move selected note(s) (pitch/start; same-pitch notes cannot overlap; adjacent OK; cannot leave the clip)
- **Alt + drag note body or resize** - free horizontal placement (no 1/16 snap)
- **Shift + drag note body** - duplicate selection, then move the copies
- **Drag left/right edge** - resize selected note(s) (same delta; stops at same-pitch neighbors and clip end)
- **Press / drag piano keys** - audition pitches (active track instrument)
- **Left-click / drag ruler** - move playhead (mapped to arrangement time)
- **Shift + left-click**, **right-click empty grid**, or **right-click drag** on the grid - move playhead (right-click on a note still deletes)
- **Delete / Backspace** - remove selected note(s)
- **Ctrl/Cmd+C** - copy selected note(s)
- **Ctrl/Cmd+X** - cut selected note(s)
- **Ctrl/Cmd+V** - paste notes into the open clip at the playhead (works across clips)
- **Ctrl/Cmd+D** - duplicate selected note(s) to the right by selection length (remappable)
- **Ctrl/Cmd+Z** - undo last clip/note edit (remappable)
- **Ctrl/Cmd+Shift+Z** - redo (remappable)
- **Right-click note** - delete note
- **Wheel** - scroll vertical; **Shift+Wheel** - horizontal; **Ctrl/Cmd+Wheel** - zoom time; **Alt+Wheel** - zoom keys
- **Scrollbars** - always-visible solid bars (drag to scroll)
- Piano keys stay pinned on the left while the grid scrolls horizontally (vertical scroll stays synced); the beat ruler stays pinned to the top

### Transport

- **Play / Pause** - play arrangement notes through each track's instrument (playhead loops). Pause returns the playhead to where playback started (triangle mark on the ruler). **Shift+Space** / Shift+click Pause leaves the playhead in place
- **Stop** - stop, silence, and return to start
- **Metronome** - checkbox in the transport bar; quarter-note clicks at project BPM while playing (downbeat accent on beat 1). Default **on**; persisted in `settings.json`. No clicks when stopped or during piano-key audition alone.
- **Space** - play/pause (factory default; remappable)
- **Shift+Space** - pause in place / play (factory default; remappable)
- **File** menu - New / Open... / Open Recent / Projects... / Save / Save As...
- **Ctrl/Cmd+N** - new project (remappable)
- **Ctrl/Cmd+O** - open `.motif` via native file dialog (remappable)
- **Ctrl/Cmd+S** - save (or Save As when untitled; includes per-track CLAP/VST3 state; remappable)
- **Ctrl/Cmd+Shift+S** - Save As... (remappable)
- **Projects...** - in-app Recent Projects loader
- Window title and toolbar show the project name with `*` when unsaved
- **Perf** toolbar / **Ctrl/Cmd+Shift+P** - Performance view (CPU graph, per-track DSP ms, xruns / lock skips; remappable)
- **Mixer** toolbar / **M** - mixer view (remappable)
- **Devices** toolbar / **D** - toggle the bottom device strip (instruments, FX, macros, modulators; remappable)
- Transport strip also shows live **CPU % / buffer / latency / xruns / locks**
- **Settings** - themes, shortcut remapping (multiple keys per action, conflict Override), Plugin Manager (Rescan / extra paths), Editing (undo depth), Project (autosave recovery interval, recent list); saved to `settings.json` + `plugin_cache.json`
- **Escape** - leave piano roll, mixer, devices, Performance, or Settings (factory default; remappable)

### Projects (`.motif`)

- Projects are JSON with a small versioned envelope, saved as `*.motif` (default folder: `~/.local/share/motif/projects` on Linux).
- Legacy bare `project.json` in the working directory still opens on startup if present.
- **Auto-save** writes a crash-recovery backup on an interval (default 3 minutes; Settings → Project). It never overwrites your saved file. On next launch Motif offers Restore or Discard.
- `settings.json` and `plugin_cache.json` stay in the working directory (app prefs / plugin scan cache, not project data).

## Plugins (CLAP / VST3)

Motif hosts **CLAP** and **VST3** instruments in-process via [truce-rack](https://crates.io/crates/truce-rack). **VST2 is not supported.**

- Open **Settings → Plugin Manager → Rescan** to refresh the instrument list from standard OS plugin directories (plus any extra paths you add).
- **Add track** or right-click a track header to pick Built-in Piano or a scanned instrument.
- **Right-click a plugin track header → Open plugin editor** to show the instrument GUI (e.g. Vital). Editors are X11-only; Motif forces the **X11** winit backend on Linux (XWayland under Hyprland) so host + editor share one stack. Native Wayland Motif + XWayland Vital often floats but ignores clicks. Override with `MOTIF_UNIX_BACKEND=wayland` if you need a Wayland Motif window. The editor is a dialog (`WM_CLASS` = `MotifPluginEditor`) so Hyprland should float it. Close via the window chrome or the same menu.
- Good Linux smoke targets: native **Vital** or **Surge XT** (CLAP/VST3).
- **Serum / yabridge on Linux is not supported** in Motif. yabridge VST3 stubs need a Wine host process; scanning or loading them in-process aborts Motif (`Assertion bridge failed`). Do not add `~/.vst3/yabridge` as an extra scan path. Use a **native Linux** CLAP/VST3 (e.g. Vital, Surge XT).

## Architecture

```text
UI (egui)
  -> Project model (tracks + instruments, clips, notes, tempo, loop)
  -> PlaylistUi / PianoRollUi (shared timeline navigation)
  -> DawEngine trait
       -> AudioEngine (audio-clock transport; cpal mix + sample-accurate sequencing;
                       shared plugin slots + editor host)
       -> MockEngine (silent fallback / tests)
  -> PluginCatalog (scan cache; load off the audio thread)
```

If the audio device fails to open, Motif keeps the transport UI and reports it in the status line.

## Pitch range

Full MIDI range (0-127). Snap grid: quarter-beat (1/16 in 4/4).
