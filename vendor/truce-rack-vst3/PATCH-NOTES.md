# truce-rack-vst3 - local patch

Vendored copy of `truce-rack-vst3` 1.1.5 (crates.io), redirected from Motif's
`Cargo.toml` via `[patch.crates-io]`. Keep the crate `version` at `1.1.5` so the
patch stays semver-compatible with the `truce-rack` umbrella.

## Why it is patched

Upstream 1.1.5 attaches a VST3 editor with `IPlugView::attached` but never
installs an `IPlugFrame`, and there is no `Linux::IRunLoop` anywhere in the
crate. On Linux a VST3 editor has no message thread of its own: the plugin
registers its X11 connection fd (and, for JUCE plugins like Vital, internal
event pipes) plus timers through the host `IRunLoop`, and expects the host to
call `onFDIsSet` / `onTimer`. Without that the editor paints once and then
ignores all mouse and keyboard input.

## What changed (all Linux-gated)

- New `run_loop` module: `HostPlugFrame` implements `IPlugFrame` +
  `Linux::IRunLoop`. It records the fds/timers the plugin registers.
- `PluginEditor::open` builds the frame, calls `IPlugView::setFrame` *before*
  `attached`, and keeps the frame alive for the editor's lifetime.
- `PluginEditor::close` calls `removed()` then detaches the frame.
- `PluginEditor::on_idle` calls `run_loop::pump`, which `poll(2)`s the
  registered fds (non-blocking) and fires due timers by calling back into the
  plugin's handlers. Motif already calls `on_idle` every UI frame.
- `IPlugFrame::resizeView` records the requested size and confirms it via
  `onSize`.
- Added `libc` (Linux target only) for `poll(2)`.

Non-Linux platforms are unchanged.

## Second patch: non-NULL host context on `initialize`

Upstream passes `ptr::null_mut()` as the context to `IPluginBase::initialize`
for both the component and the edit controller. Plugins that dereference the
host `IHostApplication` during init without a NULL-check crash on this - e.g.
LSP-Plugins segfaults (`SEGV_MAPERR`) deep inside `lsp-plugins.so` during
`initialize`.

- New `HostApplication` COM object implements `IHostApplication`
  (`getName` -> "Motif"; `createInstance` declines with `kNoInterface`).
- `load_from` builds one `ComWrapper::new(HostApplication)` and passes it to
  both `initialize` calls.
- The `ComPtr<IHostApplication>` is stored in `Vst3Plugin._host_context` so the
  object outlives any pointer the plugin may retain.

This is cross-platform (not Linux-gated).

## Re-syncing after an upstream bump

If upstream adds real run-loop support, drop this vendored copy and the
`[patch.crates-io]` entry. Otherwise re-copy the new upstream `src/lib.rs` and
re-apply the `run_loop` module + the `open`/`close`/`on_idle` hooks and the
`libc` dependency.
