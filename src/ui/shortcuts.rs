//! Keyboard shortcut registry with optional remapping via Settings.
//!
//! App commands are named [`Action`]s with [`Binding`]s. Poll once per frame
//! from `DawApp` — do not match chords inside feature widgets.

use egui::{Context, Key, Modifiers};
use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";

/// Named app command triggered by a shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    TogglePlayback,
    DeleteSelection,
    /// Delete the track under the pointer (track header hover).
    DeleteTrack,
    CopySelection,
    CutSelection,
    PasteSelection,
    DuplicateSelection,
    Undo,
    Redo,
    /// Save current project (or Save As when untitled).
    Save,
    /// Open project via native file dialog.
    Open,
    /// Save As via native file dialog.
    SaveProjectAs,
    /// Start a new empty project.
    NewProject,
    /// Open the in-app Recent Projects loader.
    OpenProjectBrowser,
    /// Open the add browser on the Instruments tab (create track).
    OpenInstrumentBrowser,
    /// Open the add browser on the FX tab (add insert effect).
    OpenEffectBrowser,
    /// Open the add browser on the Samples tab (add audio clip).
    OpenSampleBrowser,
    BackToPlaylist,
    /// Toggle the Mixer view (or return to playlist when already there).
    ToggleMixer,
    /// Toggle the bottom device strip (or no-op in Settings).
    ToggleDevices,
    /// Open or close the native plugin editor for the selected track's instrument.
    TogglePluginEditor,
    /// Close the focused native plugin editor window (grabbed on the editor parent).
    ClosePluginEditor,
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "toggle_playback" => Ok(Self::TogglePlayback),
            "delete_selection" => Ok(Self::DeleteSelection),
            "delete_track" => Ok(Self::DeleteTrack),
            "copy_selection" => Ok(Self::CopySelection),
            "cut_selection" => Ok(Self::CutSelection),
            "paste_selection" => Ok(Self::PasteSelection),
            "duplicate_selection" => Ok(Self::DuplicateSelection),
            "undo" => Ok(Self::Undo),
            "redo" => Ok(Self::Redo),
            "save" | "save_project" => Ok(Self::Save),
            "open" | "load_project" => Ok(Self::Open),
            "save_project_as" => Ok(Self::SaveProjectAs),
            "new_project" => Ok(Self::NewProject),
            "open_project_browser" => Ok(Self::OpenProjectBrowser),
            "open_instrument_browser" => Ok(Self::OpenInstrumentBrowser),
            "open_effect_browser" => Ok(Self::OpenEffectBrowser),
            "open_sample_browser" => Ok(Self::OpenSampleBrowser),
            "back_to_playlist" => Ok(Self::BackToPlaylist),
            "toggle_mixer" => Ok(Self::ToggleMixer),
            "toggle_devices" => Ok(Self::ToggleDevices),
            "toggle_plugin_editor" | "open_plugin_editor" => Ok(Self::TogglePluginEditor),
            "close_plugin_editor" => Ok(Self::ClosePluginEditor),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "toggle_playback",
                    "delete_selection",
                    "delete_track",
                    "copy_selection",
                    "cut_selection",
                    "paste_selection",
                    "duplicate_selection",
                    "undo",
                    "redo",
                    "save",
                    "open",
                    "save_project_as",
                    "new_project",
                    "open_project_browser",
                    "open_instrument_browser",
                    "open_effect_browser",
                    "open_sample_browser",
                    "back_to_playlist",
                    "toggle_mixer",
                    "toggle_devices",
                    "toggle_plugin_editor",
                    "close_plugin_editor",
                ],
            )),
        }
    }
}

impl Action {
    /// All actions in Settings / docs order (includes actions with no factory binding).
    pub const ALL: [Self; 22] = [
        Self::TogglePlayback,
        Self::DeleteSelection,
        Self::DeleteTrack,
        Self::CopySelection,
        Self::CutSelection,
        Self::PasteSelection,
        Self::DuplicateSelection,
        Self::Undo,
        Self::Redo,
        Self::NewProject,
        Self::Open,
        Self::Save,
        Self::SaveProjectAs,
        Self::OpenProjectBrowser,
        Self::OpenInstrumentBrowser,
        Self::OpenEffectBrowser,
        Self::OpenSampleBrowser,
        Self::BackToPlaylist,
        Self::ToggleMixer,
        Self::ToggleDevices,
        Self::TogglePluginEditor,
        Self::ClosePluginEditor,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TogglePlayback => "Play / Pause",
            Self::DeleteSelection => "Delete selection",
            Self::DeleteTrack => "Delete track",
            Self::CopySelection => "Copy selection",
            Self::CutSelection => "Cut selection",
            Self::PasteSelection => "Paste",
            Self::DuplicateSelection => "Duplicate selection",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Save => "Save",
            Self::Open => "Open...",
            Self::SaveProjectAs => "Save As...",
            Self::NewProject => "New project",
            Self::OpenProjectBrowser => "Projects...",
            Self::OpenInstrumentBrowser => "Add instrument...",
            Self::OpenEffectBrowser => "Add effect...",
            Self::OpenSampleBrowser => "Add sample...",
            Self::BackToPlaylist => "Back / close",
            Self::ToggleMixer => "Mixer",
            Self::ToggleDevices => "Devices",
            Self::TogglePluginEditor => "Plugin editor",
            Self::ClosePluginEditor => "Close plugin editor",
        }
    }
}

/// Key + modifier chord. `ctrl_or_cmd` matches Ctrl on Linux/Windows and Cmd on macOS
/// the same way timeline zoom does (`modifiers.ctrl || command || mac_cmd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub ctrl_or_cmd: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            ctrl_or_cmd: false,
            shift: false,
            alt: false,
        }
    }

    pub const fn ctrl_or_cmd(key: Key) -> Self {
        Self {
            key,
            ctrl_or_cmd: true,
            shift: false,
            alt: false,
        }
    }

    pub const fn ctrl_or_cmd_shift(key: Key) -> Self {
        Self {
            key,
            ctrl_or_cmd: true,
            shift: true,
            alt: false,
        }
    }

    pub fn from_modifiers(key: Key, modifiers: Modifiers) -> Self {
        Self {
            key,
            ctrl_or_cmd: modifiers.ctrl || modifiers.command || modifiers.mac_cmd,
            shift: modifiers.shift,
            alt: modifiers.alt,
        }
    }

    fn matches(&self, key: Key, modifiers: Modifiers) -> bool {
        if self.key != key {
            return false;
        }
        let ctrl_or_cmd = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
        ctrl_or_cmd == self.ctrl_or_cmd
            && modifiers.shift == self.shift
            && modifiers.alt == self.alt
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl_or_cmd {
            parts.push("Ctrl/Cmd".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        parts.push(key_display_name(self.key));
        parts.join("+")
    }

    pub fn storage_key_name(&self) -> Option<&'static str> {
        key_to_name(self.key)
    }
}

/// How an action can be triggered from egui input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Key(Chord),
    /// egui-winit maps Ctrl/Cmd+X to `Event::Cut` (no `Key::X` event).
    CutEvent,
    /// egui-winit maps Ctrl/Cmd+C to `Event::Copy`.
    CopyEvent,
    /// egui-winit maps Ctrl/Cmd+V to `Event::Paste`.
    PasteEvent,
}

impl Binding {
    pub fn display(&self) -> String {
        match self {
            Self::Key(chord) => chord.display(),
            Self::CutEvent => "Cut (Ctrl/Cmd+X)".to_string(),
            Self::CopyEvent => "Copy (Ctrl/Cmd+C)".to_string(),
            Self::PasteEvent => "Paste (Ctrl/Cmd+V)".to_string(),
        }
    }

    pub fn is_rebindable(self) -> bool {
        matches!(self, Self::Key(_))
    }
}

/// Which actions `poll` may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollFilter {
    /// All registered actions.
    All,
    /// Only [`Action::BackToPlaylist`] (e.g. Settings when not capturing).
    NavigationOnly,
    /// Suppress all shortcuts (e.g. while capturing a new chord).
    None,
}

/// Result of reading a key during shortcut capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    Chord(Chord),
    Cancel,
    Pending,
}

/// Result of assigning a key chord to an action (replace or add).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyChordOutcome {
    Applied,
    /// Chord already bound to this action (no change).
    Unchanged,
    /// Chord is used by another action; retry with `override_conflict: true` to steal it.
    Conflict { with: Action },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredBinding {
    action: Action,
    #[serde(flatten)]
    kind: StoredBindingKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoredBindingKind {
    Key {
        key: String,
        ctrl_or_cmd: bool,
        shift: bool,
        alt: bool,
    },
    Cut,
    Copy,
    Paste,
}

/// Editable binding table. Defaults live here; overrides persist in `settings.json`.
#[derive(Debug, Clone)]
pub struct ShortcutRegistry {
    bindings: Vec<(Action, Binding)>,
}

impl ShortcutRegistry {
    pub fn defaults() -> Self {
        Self {
            bindings: vec![
                (Action::TogglePlayback, Binding::Key(Chord::new(Key::Space))),
                (
                    Action::DeleteSelection,
                    Binding::Key(Chord::new(Key::Delete)),
                ),
                (
                    Action::DeleteSelection,
                    Binding::Key(Chord::new(Key::Backspace)),
                ),
                (Action::DeleteTrack, Binding::Key(Chord::new(Key::X))),
                (Action::CopySelection, Binding::CopyEvent),
                (Action::CutSelection, Binding::CutEvent),
                (Action::PasteSelection, Binding::PasteEvent),
                (
                    Action::DuplicateSelection,
                    Binding::Key(Chord::ctrl_or_cmd(Key::D)),
                ),
                (Action::Undo, Binding::Key(Chord::ctrl_or_cmd(Key::Z))),
                (
                    Action::Redo,
                    Binding::Key(Chord::ctrl_or_cmd_shift(Key::Z)),
                ),
                (Action::Save, Binding::Key(Chord::ctrl_or_cmd(Key::S))),
                (Action::Open, Binding::Key(Chord::ctrl_or_cmd(Key::O))),
                (
                    Action::SaveProjectAs,
                    Binding::Key(Chord::ctrl_or_cmd_shift(Key::S)),
                ),
                (
                    Action::NewProject,
                    Binding::Key(Chord::ctrl_or_cmd(Key::N)),
                ),
                (
                    Action::OpenInstrumentBrowser,
                    Binding::Key(Chord::ctrl_or_cmd(Key::W)),
                ),
                (
                    Action::OpenEffectBrowser,
                    Binding::Key(Chord::ctrl_or_cmd(Key::F)),
                ),
                (
                    Action::OpenSampleBrowser,
                    Binding::Key(Chord::ctrl_or_cmd(Key::B)),
                ),
                (
                    Action::BackToPlaylist,
                    Binding::Key(Chord::new(Key::Escape)),
                ),
                (Action::ToggleMixer, Binding::Key(Chord::ctrl_or_cmd(Key::M))),
                (
                    Action::ToggleDevices,
                    Binding::Key(Chord::ctrl_or_cmd_shift(Key::M)),
                ),
                (
                    Action::TogglePluginEditor,
                    Binding::Key(Chord::ctrl_or_cmd_shift(Key::E)),
                ),
                (
                    Action::ClosePluginEditor,
                    Binding::Key(Chord {
                        key: Key::Q,
                        ctrl_or_cmd: false,
                        shift: true,
                        alt: false,
                    }),
                ),
            ],
        }
    }

    /// First key chord bound to `action`, if any.
    pub fn primary_key_chord(&self, action: Action) -> Option<Chord> {
        self.bindings.iter().find_map(|(existing, binding)| {
            if *existing == action {
                if let Binding::Key(chord) = binding {
                    return Some(*chord);
                }
            }
            None
        })
    }

    pub fn to_stored(&self) -> Result<Vec<StoredBinding>, String> {
        let mut stored = Vec::with_capacity(self.bindings.len());
        for (action, binding) in &self.bindings {
            let kind = StoredBindingKind::from_binding(*binding)
                .ok_or_else(|| format!("cannot serialize binding for {:?}", action))?;
            stored.push(StoredBinding {
                action: *action,
                kind,
            });
        }
        Ok(stored)
    }

    pub fn from_stored(stored: Vec<StoredBinding>) -> Result<Self, String> {
        if stored.is_empty() {
            return Err("empty bindings".into());
        }
        let mut bindings = Vec::with_capacity(stored.len());
        for entry in stored {
            let binding = entry
                .kind
                .to_binding()
                .ok_or_else(|| "unknown key in settings".to_string())?;
            bindings.push((entry.action, binding));
        }
        Ok(Self { bindings })
    }

    /// Append factory bindings for any [`Action`] missing from a loaded settings file.
    pub fn ensure_default_actions(&mut self) {
        // Cut used to map to DeleteSelection; it is now CutSelection.
        self.bindings.retain(|(action, binding)| {
            !(matches!(action, Action::DeleteSelection) && matches!(binding, Binding::CutEvent))
        });

        for (action, binding) in Self::defaults().bindings {
            let has_action = self
                .bindings
                .iter()
                .any(|(existing, _)| *existing == action);
            if !has_action {
                self.bindings.push((action, binding));
            }
        }
    }

    pub fn reset_defaults(&mut self) {
        *self = Self::defaults();
    }

    pub fn get(&self, index: usize) -> Option<(Action, Binding)> {
        self.bindings.get(index).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, Action, Binding)> + '_ {
        self.bindings
            .iter()
            .enumerate()
            .map(|(index, (action, binding))| (index, *action, *binding))
    }

    /// Distinct actions in Settings order. Includes factory actions even with no binding yet.
    pub fn actions_in_order(&self) -> Vec<Action> {
        let mut seen: Vec<Action> = Action::ALL.to_vec();
        for (action, _) in &self.bindings {
            if !seen.contains(action) {
                seen.push(*action);
            }
        }
        seen
    }

    /// Indices of bindings for `action` in registry order.
    pub fn indices_for_action(&self, action: Action) -> Vec<usize> {
        self.bindings
            .iter()
            .enumerate()
            .filter(|(_, (existing, _))| *existing == action)
            .map(|(index, _)| index)
            .collect()
    }

    pub fn can_remove_binding(&self, index: usize) -> bool {
        matches!(self.bindings.get(index), Some((_, Binding::Key(_))))
    }

    /// Remove a key binding. System Copy/Cut/Paste event slots cannot be removed.
    pub fn remove_binding(&mut self, index: usize) -> Result<(), String> {
        let Some((_, binding)) = self.bindings.get(index).copied() else {
            return Err("invalid binding index".into());
        };
        if !binding.is_rebindable() {
            return Err("this binding cannot be removed".into());
        }
        self.bindings.remove(index);
        Ok(())
    }

    /// Replace a rebindable key slot. On conflict, returns [`ApplyChordOutcome::Conflict`]
    /// unless `override_conflict` is true (steals the chord from the other action).
    pub fn try_set_key_binding(
        &mut self,
        index: usize,
        chord: Chord,
        override_conflict: bool,
    ) -> Result<ApplyChordOutcome, String> {
        let Some((action, binding)) = self.bindings.get(index).copied() else {
            return Err("invalid binding index".into());
        };
        if !binding.is_rebindable() {
            return Err("this binding cannot be changed".into());
        }
        self.apply_chord(Some(index), action, chord, override_conflict)
    }

    /// Append a key binding for `action`. Same conflict / override rules as replace.
    pub fn try_add_key_binding(
        &mut self,
        action: Action,
        chord: Chord,
        override_conflict: bool,
    ) -> Result<ApplyChordOutcome, String> {
        self.apply_chord(None, action, chord, override_conflict)
    }

    fn apply_chord(
        &mut self,
        replace_index: Option<usize>,
        action: Action,
        chord: Chord,
        override_conflict: bool,
    ) -> Result<ApplyChordOutcome, String> {
        let new_binding = Binding::Key(chord);

        if let Some(index) = replace_index {
            let Some((current_action, binding)) = self.bindings.get(index).copied() else {
                return Err("invalid binding index".into());
            };
            if current_action != action {
                return Err("binding index does not match action".into());
            }
            if !binding.is_rebindable() {
                return Err("this binding cannot be changed".into());
            }
            if binding == new_binding {
                return Ok(ApplyChordOutcome::Unchanged);
            }
        } else if self
            .bindings
            .iter()
            .any(|(existing, binding)| *existing == action && *binding == new_binding)
        {
            return Ok(ApplyChordOutcome::Unchanged);
        }

        if let Some(with) = self.conflicting_action(new_binding, replace_index, action) {
            if !override_conflict {
                return Ok(ApplyChordOutcome::Conflict { with });
            }
        }

        match replace_index {
            Some(index) => {
                let target = if override_conflict {
                    self.remove_conflicts_adjusting(new_binding, index)
                } else {
                    index
                };
                self.bindings[target].1 = new_binding;
            }
            None => {
                if override_conflict {
                    self.remove_conflicts_adjusting(new_binding, usize::MAX);
                }
                let insert_at = self
                    .bindings
                    .iter()
                    .rposition(|(existing, _)| *existing == action)
                    .map(|i| i + 1)
                    .unwrap_or(self.bindings.len());
                self.bindings.insert(insert_at, (action, new_binding));
            }
        }
        Ok(ApplyChordOutcome::Applied)
    }

    fn conflicting_action(
        &self,
        binding: Binding,
        ignore_index: Option<usize>,
        action: Action,
    ) -> Option<Action> {
        for (other_index, (other_action, other_binding)) in self.bindings.iter().enumerate() {
            if Some(other_index) == ignore_index {
                continue;
            }
            if *other_binding == binding && *other_action != action {
                return Some(*other_action);
            }
        }
        None
    }

    /// Remove every row whose binding equals `binding`, except `keep_index`.
    /// Returns the (possibly shifted) keep index. Pass `usize::MAX` when adding.
    fn remove_conflicts_adjusting(&mut self, binding: Binding, keep_index: usize) -> usize {
        let mut target = keep_index;
        let mut i = 0;
        while i < self.bindings.len() {
            if i == target {
                i += 1;
                continue;
            }
            if self.bindings[i].1 == binding {
                self.bindings.remove(i);
                if i < target {
                    target -= 1;
                }
                continue;
            }
            i += 1;
        }
        target
    }

    /// Returns deduped actions triggered this frame.
    pub fn poll(&self, ctx: &Context, filter: PollFilter) -> Vec<Action> {
        if filter == PollFilter::None || ctx.wants_keyboard_input() {
            return Vec::new();
        }

        let mut triggered = Vec::new();
        ctx.input(|input| {
            for (action, binding) in &self.bindings {
                if filter == PollFilter::NavigationOnly && *action != Action::BackToPlaylist {
                    continue;
                }
                let hit = match binding {
                    Binding::Key(chord) => {
                        input.key_pressed(chord.key) && chord.matches(chord.key, input.modifiers)
                    }
                    Binding::CutEvent => input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Cut)),
                    Binding::CopyEvent => input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Copy)),
                    Binding::PasteEvent => input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Paste(_))),
                };
                if hit && !triggered.contains(action) {
                    triggered.push(*action);
                }
            }
        });
        triggered
    }

    /// Read a chord for rebinding. Escape cancels. Modifier-only keys are ignored.
    pub fn capture_from_context(ctx: &Context) -> CaptureOutcome {
        ctx.input(|input| {
            if input.key_pressed(Key::Escape) {
                return CaptureOutcome::Cancel;
            }

            for event in &input.events {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                else {
                    continue;
                };
                if is_modifier_only_key(*key) {
                    continue;
                }
                if key_to_name(*key).is_none() {
                    continue;
                }
                return CaptureOutcome::Chord(Chord::from_modifiers(*key, *modifiers));
            }
            CaptureOutcome::Pending
        })
    }
}

impl StoredBindingKind {
    fn from_binding(binding: Binding) -> Option<Self> {
        match binding {
            Binding::CutEvent => Some(Self::Cut),
            Binding::CopyEvent => Some(Self::Copy),
            Binding::PasteEvent => Some(Self::Paste),
            Binding::Key(chord) => Some(Self::Key {
                key: key_to_name(chord.key)?.to_string(),
                ctrl_or_cmd: chord.ctrl_or_cmd,
                shift: chord.shift,
                alt: chord.alt,
            }),
        }
    }

    fn to_binding(self) -> Option<Binding> {
        match self {
            Self::Cut => Some(Binding::CutEvent),
            Self::Copy => Some(Binding::CopyEvent),
            Self::Paste => Some(Binding::PasteEvent),
            Self::Key {
                key,
                ctrl_or_cmd,
                shift,
                alt,
            } => {
                let key = name_to_key(&key)?;
                Some(Binding::Key(Chord {
                    key,
                    ctrl_or_cmd,
                    shift,
                    alt,
                }))
            }
        }
    }
}

fn is_modifier_only_key(key: Key) -> bool {
    // Escape is handled as Cancel before this runs.
    matches!(key, Key::Copy | Key::Cut | Key::Paste)
}

fn key_display_name(key: Key) -> String {
    key_to_name(key)
        .map(|name| name.to_string())
        .unwrap_or_else(|| format!("{key:?}"))
}

fn key_to_name(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::Space => "Space",
        Key::Escape => "Escape",
        Key::Delete => "Delete",
        Key::Backspace => "Backspace",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Insert => "Insert",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::Minus => "Minus",
        Key::Equals => "Equals",
        Key::OpenBracket => "OpenBracket",
        Key::CloseBracket => "CloseBracket",
        Key::Semicolon => "Semicolon",
        Key::Quote => "Quote",
        Key::Backtick => "Backtick",
        Key::Comma => "Comma",
        Key::Period => "Period",
        Key::Slash => "Slash",
        _ => return None,
    })
}

fn name_to_key(name: &str) -> Option<Key> {
    Some(match name {
        "Space" => Key::Space,
        "Escape" => Key::Escape,
        "Delete" => Key::Delete,
        "Backspace" => Key::Backspace,
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "Insert" => Key::Insert,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Left" => Key::ArrowLeft,
        "Right" => Key::ArrowRight,
        "Up" => Key::ArrowUp,
        "Down" => Key::ArrowDown,
        "A" => Key::A,
        "B" => Key::B,
        "C" => Key::C,
        "D" => Key::D,
        "E" => Key::E,
        "F" => Key::F,
        "G" => Key::G,
        "H" => Key::H,
        "I" => Key::I,
        "J" => Key::J,
        "K" => Key::K,
        "L" => Key::L,
        "M" => Key::M,
        "N" => Key::N,
        "O" => Key::O,
        "P" => Key::P,
        "Q" => Key::Q,
        "R" => Key::R,
        "S" => Key::S,
        "T" => Key::T,
        "U" => Key::U,
        "V" => Key::V,
        "W" => Key::W,
        "X" => Key::X,
        "Y" => Key::Y,
        "Z" => Key::Z,
        "0" => Key::Num0,
        "1" => Key::Num1,
        "2" => Key::Num2,
        "3" => Key::Num3,
        "4" => Key::Num4,
        "5" => Key::Num5,
        "6" => Key::Num6,
        "7" => Key::Num7,
        "8" => Key::Num8,
        "9" => Key::Num9,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "Minus" => Key::Minus,
        "Equals" => Key::Equals,
        "OpenBracket" => Key::OpenBracket,
        "CloseBracket" => Key::CloseBracket,
        "Semicolon" => Key::Semicolon,
        "Quote" => Key::Quote,
        "Backtick" => Key::Backtick,
        "Comma" => Key::Comma,
        "Period" => Key::Period,
        "Slash" => Key::Slash,
        _ => return None,
    })
}
