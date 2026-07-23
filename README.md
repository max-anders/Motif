# Motif

Open-source music sketchpad: piano roll + mock transport. No real audio yet.

**Status:** This project literally just started. There is almost nothing here yet — and that is the point. It is **free and open source from minute zero**: public from the first commit, not opened up later after something polished existed behind closed doors.

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

Linux may also need egui native deps (Wayland/X11 + OpenGL or wgpu stack). On Arch:

```bash
sudo pacman -S --needed base-devel pkgconf openssl libxkbcommon wayland
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

- **Click empty grid** - add a note (1 beat, snapped to 1/16)
- **Drag note body** - move pitch/start
- **Drag left/right edge** - resize
- **Click grid** - move playhead
- **Play / Pause** - mock transport (playhead loops)
- **Stop** - stop and return to start
- **Delete / Backspace** - remove selected note
- **Right-click note** - delete note
- **Space** - play/pause
- **Save / Load** - writes `project.json` in the project directory

## Architecture

```text
UI (egui)
  -> Project model (notes, tempo, loop)
  -> DawEngine trait
       -> MockEngine (advances playhead by wall clock)
```

Swap `MockEngine` for a real audio engine later without rewriting the piano roll.

## Pitch range

C3 (48) through C6 (84). Snap grid: quarter-beat (1/16 in 4/4).
