//! Plugin editor sessions (native parent window + PluginEditor), keyed by
//! [`PluginRef`] so a track's instrument and each insert-FX device can each
//! have their own independent editor window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::host::HostedPlugin;

#[cfg(target_os = "linux")]
use super::editor_window::{EditorParentWindow, EditorWindowEvent, HostX11};
#[cfg(target_os = "linux")]
use truce_rack::core::editor::WindowHandle;

/// Identifies a single hosted plugin slot: a track's instrument
/// (`device_id: None`) or one insert-FX device on that track
/// (`device_id: Some(id)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginRef {
    pub track_id: u64,
    pub device_id: Option<u64>,
}

impl PluginRef {
    pub fn instrument(track_id: u64) -> Self {
        Self {
            track_id,
            device_id: None,
        }
    }

    pub fn device(track_id: u64, device_id: u64) -> Self {
        Self {
            track_id,
            device_id: Some(device_id),
        }
    }
}

struct EditorSession {
    plugin: Arc<Mutex<HostedPlugin>>,
    #[cfg(target_os = "linux")]
    window: EditorParentWindow,
    /// Display title (used by the open-editors UI strip).
    title: String,
    /// Last size applied from host resize (avoid feedback loops).
    last_size: Option<(u32, u32)>,
}

/// Aggregated result of polling all open editor windows for one frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorPoll {
    /// At least one editor window is still open.
    pub any_open: bool,
    /// A focused editor requested transport toggle (Space) this frame.
    pub toggle_playback: bool,
}

/// UI-thread manager for open plugin editor windows.
#[derive(Default)]
pub struct PluginEditorHost {
    sessions: HashMap<PluginRef, EditorSession>,
}

/// Outcome of polling a single editor window.
#[derive(Default)]
struct PollOne {
    keep_open: bool,
    toggle_playback: bool,
}

impl PluginEditorHost {
    pub fn is_open(&self, target: PluginRef) -> bool {
        self.sessions.contains_key(&target)
    }

    pub fn any_open(&self) -> bool {
        !self.sessions.is_empty()
    }

    pub fn open(
        &mut self,
        target: PluginRef,
        plugin: Arc<Mutex<HostedPlugin>>,
        title: &str,
        host_x11: Option<HostX11>,
        forward_transport: bool,
    ) -> Result<(), String> {
        if self.sessions.contains_key(&target) {
            return Ok(());
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = (plugin, title, host_x11, forward_transport);
            return Err(String::from(
                "Plugin editors are only supported on Linux (X11 / XWayland).",
            ));
        }

        #[cfg(target_os = "linux")]
        {
            let window = EditorParentWindow::create(title, host_x11, forward_transport)?;
            let parent = WindowHandle::X11(window.x11_window_id());

            // JUCE/Vital under XWayland: egui's pixels_per_point as clap set_scale
            // often desyncs hit-testing. Host scale 1.0 matches Bitwig-style X11 hosting.
            let scale = 1.0_f64;

            {
                let mut guard = plugin
                    .lock()
                    .map_err(|_| String::from("Plugin lock poisoned"))?;
                if !guard.has_editor() {
                    return Err(String::from("Plugin has no editor GUI"));
                }
                guard.open_editor(parent, scale)?;
                if let Some((w, h)) = guard.editor_size() {
                    let _ = window.resize(w, h);
                }
            }

            self.sessions.insert(
                target,
                EditorSession {
                    plugin,
                    window,
                    title: title.to_string(),
                    last_size: None,
                },
            );
            Ok(())
        }
    }

    /// Plugin refs + titles of currently open editors (for the UI strip).
    pub fn open_editors(&self) -> Vec<(PluginRef, String)> {
        self.sessions
            .iter()
            .map(|(target, session)| (*target, session.title.clone()))
            .collect()
    }

    /// Live-toggle Space transport forwarding for one open editor.
    pub fn set_forward_transport(&mut self, target: PluginRef, forward: bool) {
        if let Some(session) = self.sessions.get_mut(&target) {
            #[cfg(target_os = "linux")]
            session.window.set_forward_transport(forward);
            #[cfg(not(target_os = "linux"))]
            let _ = (session, forward);
        }
    }

    pub fn close(&mut self, target: PluginRef) {
        let Some(session) = self.sessions.remove(&target) else {
            return;
        };
        if let Ok(mut guard) = session.plugin.lock() {
            guard.close_editor();
        };
    }

    /// Close every open editor belonging to a track (its instrument and all
    /// of its insert-FX devices). Used when the whole track is removed.
    pub fn close_track(&mut self, track_id: u64) {
        let targets: Vec<PluginRef> = self
            .sessions
            .keys()
            .copied()
            .filter(|target| target.track_id == track_id)
            .collect();
        for target in targets {
            self.close(target);
        }
    }

    pub fn close_all(&mut self) {
        let targets: Vec<PluginRef> = self.sessions.keys().copied().collect();
        for target in targets {
            self.close(target);
        }
    }

    /// Drain window events, idle editors; returns aggregated poll outcome.
    pub fn poll(&mut self) -> EditorPoll {
        let targets: Vec<PluginRef> = self.sessions.keys().copied().collect();
        let mut toggle_playback = false;
        for target in targets {
            let result = self.poll_one(target);
            toggle_playback |= result.toggle_playback;
            if !result.keep_open {
                self.close(target);
            }
        }
        EditorPoll {
            any_open: self.any_open(),
            toggle_playback,
        }
    }

    fn poll_one(&mut self, target: PluginRef) -> PollOne {
        let Some(session) = self.sessions.get_mut(&target) else {
            return PollOne::default();
        };

        let mut toggle_playback = false;

        #[cfg(target_os = "linux")]
        {
            let events = match session.window.poll_events() {
                Ok(events) => events,
                Err(_) => {
                    return PollOne {
                        keep_open: false,
                        toggle_playback,
                    }
                }
            };

            for event in events {
                match event {
                    EditorWindowEvent::CloseRequested => {
                        return PollOne {
                            keep_open: false,
                            toggle_playback,
                        }
                    }
                    EditorWindowEvent::TogglePlayback => toggle_playback = true,
                    EditorWindowEvent::Resized { width, height } => {
                        if session.last_size == Some((width, height)) {
                            continue;
                        }
                        session.last_size = Some((width, height));
                        // Only notify resizable editors; avoid fighting Vital's layout.
                        if let Ok(mut guard) = session.plugin.try_lock() {
                            if guard.editor_is_resizable() {
                                let _ = guard.editor_set_size(width, height);
                            }
                        }
                    }
                }
            }
        }

        if let Ok(mut guard) = session.plugin.try_lock() {
            guard.editor_on_idle();
        }
        PollOne {
            keep_open: true,
            toggle_playback,
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[derive(Clone, Copy)]
pub struct HostX11;
