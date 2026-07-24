# truce-rack-clap - local patch

Vendored copy of `truce-rack-clap` 1.1.5 (crates.io), redirected from Motif's
`Cargo.toml` via `[patch.crates-io]`. Keep the crate `version` at `1.1.5` so the
patch stays semver-compatible with the `truce-rack` umbrella.

## Why it is patched

Upstream 1.1.5 creates the plugin with a null `clap_host`
(`let host = ptr::null();`). With a null host the plugin cannot query
`clap.timer-support` or `clap.posix-fd-support`, so on Linux its editor has no
way to register the X11 fd/timers it needs pumped. The GUI paints but never
receives mouse or keyboard input (same root cause as the VST3 patch).

## What changed (all Linux-gated)

- New `run_loop` module: `ClapHost` owns a heap-stable `clap_host` advertising
  `clap.timer-support` + `clap.posix-fd-support` (plus no-op `request_*` and an
  `on_main_thread` bridge for `request_callback`). Host callbacks record the
  plugin's timers/fds in a shared registry.
- `load_from` builds the host and passes it to `create_plugin` (was null), then
  caches the plugin-side `clap.timer-support` / `clap.posix-fd-support`.
- `PluginEditor::on_idle` calls `run_loop::pump`, which fires due timers, runs a
  pending `on_main_thread`, and `poll(2)`s registered fds (non-blocking),
  calling the plugin's `on_timer` / `on_fd`. Motif calls `on_idle` every frame.
- Added `libc` (Linux target only) for `poll(2)`.

Non-Linux platforms keep the historical null host (native run loop).

## Re-syncing after an upstream bump

If upstream supplies a real `clap_host` with timer + posix-fd support, drop this
vendored copy and the `[patch.crates-io]` entry. Otherwise re-copy the new
upstream `src/lib.rs` and re-apply the `run_loop` module, the `load_from` host
wiring, and the `on_idle` hook plus the `libc` dependency.
