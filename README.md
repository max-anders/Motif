# Motif

Open-source music sketchpad: piano roll, playlist, soft-synth piano, and CLAP/VST3 instruments (optional native editor windows) over a real audio engine.

## Status

**Early but usable sketchpad** (2026-07-24): playlist + piano roll editing with undo/redo, transport, cpal playback (built-in piano and/or CLAP/VST3), plugin manager + editor windows (Linux X11/XWayland), multi-project `.motif` save/open with recent list + recovery autosave, and Settings for shortcuts, themes, plugins, editing, and project prefs. Not a full DAW.

| Area | State |
|---|---|
| Playlist (tracks / MIDI clips) | Working |
| Per-track instruments (piano / CLAP / VST3) | Working (load off UI thread) |
| Plugin manager (scan / cache) | Working (`plugin_cache.json` in CWD) |
| Plugin editor GUI | Working on Linux (X11 / XWayland); not Wayland-native |
| Piano roll (notes + key audition + copy/paste) | Working |
| Undo / redo (clips + notes) | Working (depth in Settings → Editing) |
| Transport + loop + BPM + metronome | Working |
| Soft piano + plugin mix (`cpal`) | Working (UI continues if device open fails) |
| Project save/load | Working (`.motif` files; recent projects; recovery autosave) |
| Settings (shortcuts + themes + plugins + editing + project) | Working (`settings.json` in CWD) |
| VST2 / mixer / export / samples | Not started |
| Platform support | **Linux** (developed & tested); Windows/macOS untested |

It is **free and open source from minute zero**: public from the first commit, not opened up later after something polished existed behind closed doors.

## Open-source philosophy

This is a personal experiment being built in the open from day one — not a product that went public after a private beta.

The repo is public from the start. No closed preview, no "open sourcing later" roadmap. If you are reading this early, you are seeing it almost as it is being born.

The early versions may be rough and experimental. I iterate quickly in public; not every path is reviewed to production standards yet. The goal is to explore ideas, test workflows, and see what kind of creative tool can emerge.

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
- **Left-click clip** - select clip
- **Double-click clip** - open piano roll for that clip
- **Ctrl/Cmd + left-click clip** - toggle multi-select
- **Drag clip body** - move selected clip(s) on timeline
- **Shift + drag clip body** - duplicate selection, then move the copies
- **Drag clip edges** - resize clip length
- **Delete / Backspace** - remove selected clip(s)
- **Ctrl/Cmd+C** - copy selected clip(s)
- **Ctrl/Cmd+X** - cut selected clip(s)
- **Ctrl/Cmd+V** - paste clips at the playhead (same tracks)
- **Ctrl/Cmd+D** - duplicate selected clip(s) to the right by selection length (remappable)
- **Ctrl/Cmd+Z** - undo last clip/note edit (remappable)
- **Ctrl/Cmd+Shift+Z** - redo (remappable)
- **Add track** - menu: Built-in Piano or a scanned CLAP/VST3 instrument
- **M / S** on track header (or right-click **Mute** / **Solo**) - mute or solo a track; when any track is soloed, only soloed tracks play; otherwise muted tracks are silent
- **Right-click track header → Delete track** - remove the track and its clips (last track cannot be deleted; Ctrl/Cmd+Z restores a deleted track)
- **Right-click track header** - open/close plugin editor (plugin tracks) or change instrument (searchable list)
- **Left-click / drag ruler** - move playhead (snapped to 1/16)
- **Shift + left-click** or **right-click empty timeline** - move playhead
- **Wheel** - scroll; **Shift+Wheel** - horizontal scroll; **Ctrl/Cmd+Wheel** - zoom time
- **Scrollbars** - always-visible solid bars (drag to scroll)
- Track headers stay pinned on the left while the timeline scrolls horizontally (vertical scroll stays synced); the beat ruler stays pinned to the top

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

- **Play / Pause** - play arrangement notes through each track's instrument (playhead loops)
- **Stop** - stop, silence, and return to start
- **Metronome** - checkbox in the transport bar; quarter-note clicks at project BPM while playing (downbeat accent on beat 1). Default **on**; persisted in `settings.json`. No clicks when stopped or during piano-key audition alone.
- **Space** - play/pause (factory default; remappable)
- **File** menu - New / Open... / Open Recent / Projects... / Save / Save As...
- **Ctrl/Cmd+N** - new project (remappable)
- **Ctrl/Cmd+O** - open `.motif` via native file dialog (remappable)
- **Ctrl/Cmd+S** - save (or Save As when untitled; includes per-track CLAP/VST3 state; remappable)
- **Ctrl/Cmd+Shift+S** - Save As... (remappable)
- **Projects...** - in-app Recent Projects loader
- Window title and toolbar show the project name with `*` when unsaved
- **Settings** - themes, shortcut remapping (multiple keys per action, conflict Override), Plugin Manager (Rescan / extra paths), Editing (undo depth), Project (autosave recovery interval, recent list); saved to `settings.json` + `plugin_cache.json`
- **Escape** - leave piano roll or Settings (factory default; remappable)

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
       -> AudioEngine (UI clock + cpal mix; shared plugin slots + editor host)
       -> MockEngine (silent fallback / tests)
  -> PluginCatalog (scan cache; load off the audio thread)
```

If the audio device fails to open, Motif keeps the transport UI and reports it in the status line.

## Pitch range

Full MIDI range (0-127). Snap grid: quarter-beat (1/16 in 4/4).
