//! Keyboard shortcut registry with optional remapping via Settings.
//!
//! App commands are named [`Action`]s with [`Binding`]s. Poll once per frame
//! from `DawApp` — do not match chords inside feature widgets.

use egui::{Context, Key, Modifiers};
use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";

/// Named app command triggered by a shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    TogglePlayback,
    DeleteSelection,
    DuplicateSelection,
    SaveProject,
    LoadProject,
    BackToPlaylist,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::TogglePlayback => "Play / Pause",
            Self::DeleteSelection => "Delete selection",
            Self::DuplicateSelection => "Duplicate selection",
            Self::SaveProject => "Save project",
            Self::LoadProject => "Load project",
            Self::BackToPlaylist => "Back / close",
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
}

/// How an action can be triggered from egui input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Key(Chord),
    /// egui-winit maps Ctrl/Cmd+X to `Event::Cut` (no `Key::X` event).
    CutEvent,
}

impl Binding {
    pub fn display(&self) -> String {
        match self {
            Self::Key(chord) => chord.display(),
            Self::CutEvent => "Cut (Ctrl/Cmd+X)".to_string(),
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
                (Action::DeleteSelection, Binding::CutEvent),
                (
                    Action::DuplicateSelection,
                    Binding::Key(Chord::ctrl_or_cmd(Key::D)),
                ),
                (
                    Action::SaveProject,
                    Binding::Key(Chord::ctrl_or_cmd(Key::S)),
                ),
                (
                    Action::LoadProject,
                    Binding::Key(Chord::ctrl_or_cmd(Key::O)),
                ),
                (
                    Action::BackToPlaylist,
                    Binding::Key(Chord::new(Key::Escape)),
                ),
            ],
        }
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
        for (action, binding) in Self::defaults().bindings {
            let has_action = self.bindings.iter().any(|(existing, _)| *existing == action);
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

    /// Replace a rebindable key slot. Rejects conflicts with other actions and Cut slots.
    pub fn try_set_key_binding(&mut self, index: usize, chord: Chord) -> Result<(), String> {
        let Some((action, binding)) = self.bindings.get(index).copied() else {
            return Err("invalid binding index".into());
        };
        if !binding.is_rebindable() {
            return Err("this binding cannot be changed".into());
        }
        let new_binding = Binding::Key(chord);
        for (other_index, (other_action, other_binding)) in self.bindings.iter().enumerate() {
            if other_index == index {
                continue;
            }
            if *other_binding == new_binding && *other_action != action {
                return Err(format!(
                    "already used by {}",
                    other_action.label()
                ));
            }
        }
        self.bindings[index].1 = new_binding;
        Ok(())
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
