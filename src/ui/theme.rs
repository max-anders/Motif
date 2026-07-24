//! App theme: named color palettes editable in Settings and used by all UI paint code.

use egui::{Color32, Context, Visuals};
use serde::{Deserialize, Serialize};

pub const DEFAULT_THEME_NAME: &str = "Default Dark";

/// Semantic colors for Motif chrome and editors.
///
/// Agents: never hardcode `Color32::from_rgb` in UI paint paths - add a field here
/// (with a factory default) and read it from the active theme.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeColors {
    // Shell / panels
    #[serde(with = "color32_serde")]
    pub panel_bg: Color32,
    #[serde(with = "color32_serde")]
    pub text_primary: Color32,
    #[serde(with = "color32_serde")]
    pub text_muted: Color32,
    #[serde(with = "color32_serde")]
    pub accent: Color32,
    #[serde(with = "color32_serde")]
    pub accent_warning: Color32,
    #[serde(with = "color32_serde")]
    pub separator: Color32,

    // egui chrome
    #[serde(with = "color32_serde")]
    pub widget_bg: Color32,
    #[serde(with = "color32_serde")]
    pub widget_bg_hovered: Color32,
    #[serde(with = "color32_serde")]
    pub widget_bg_active: Color32,
    #[serde(with = "color32_serde")]
    pub button_text: Color32,

    // Editor scrollbars (solid ScrollArea in playlist / piano roll)
    #[serde(default = "default_scrollbar_track", with = "color32_serde")]
    pub scrollbar_track: Color32,
    #[serde(default = "default_scrollbar_handle", with = "color32_serde")]
    pub scrollbar_handle: Color32,
    #[serde(default = "default_scrollbar_handle_hovered", with = "color32_serde")]
    pub scrollbar_handle_hovered: Color32,
    #[serde(default = "default_scrollbar_handle_active", with = "color32_serde")]
    pub scrollbar_handle_active: Color32,

    // Timeline shared
    #[serde(with = "color32_serde")]
    pub ruler_bg: Color32,
    #[serde(with = "color32_serde")]
    pub gutter_bg: Color32,
    #[serde(with = "color32_serde")]
    pub tick_major: Color32,
    #[serde(with = "color32_serde")]
    pub tick_minor: Color32,
    #[serde(with = "color32_serde")]
    pub tick_sub: Color32,
    #[serde(with = "color32_serde")]
    pub ruler_text: Color32,
    #[serde(with = "color32_serde")]
    pub playhead: Color32,
    #[serde(with = "color32_serde")]
    pub grid_bar: Color32,
    #[serde(with = "color32_serde")]
    pub grid_beat: Color32,
    #[serde(with = "color32_serde")]
    pub grid_subbeat: Color32,

    // Playlist
    #[serde(with = "color32_serde")]
    pub track_header_bg: Color32,
    #[serde(with = "color32_serde")]
    pub track_header_text: Color32,
    #[serde(with = "color32_serde")]
    pub lane_bg: Color32,
    #[serde(with = "color32_serde")]
    pub clip_fill: Color32,
    #[serde(with = "color32_serde")]
    pub clip_fill_selected: Color32,
    #[serde(with = "color32_serde")]
    pub clip_stroke: Color32,
    #[serde(with = "color32_serde")]
    pub clip_stroke_selected: Color32,
    #[serde(with = "color32_serde")]
    pub clip_label: Color32,
    #[serde(with = "color32_serde")]
    pub clip_note_preview: Color32,

    // Piano roll
    #[serde(with = "color32_serde")]
    pub key_row_black: Color32,
    #[serde(with = "color32_serde")]
    pub key_row_white: Color32,
    #[serde(with = "color32_serde")]
    pub keys_bg: Color32,
    #[serde(with = "color32_serde")]
    pub white_key: Color32,
    #[serde(with = "color32_serde")]
    pub white_key_active: Color32,
    #[serde(with = "color32_serde")]
    pub white_key_border: Color32,
    #[serde(with = "color32_serde")]
    pub white_key_label: Color32,
    #[serde(with = "color32_serde")]
    pub black_key: Color32,
    #[serde(with = "color32_serde")]
    pub black_key_active: Color32,
    #[serde(with = "color32_serde")]
    pub black_key_border: Color32,
    #[serde(with = "color32_serde")]
    pub key_divider: Color32,
    #[serde(with = "color32_serde")]
    pub note_fill: Color32,
    #[serde(with = "color32_serde")]
    pub note_fill_selected: Color32,
    #[serde(with = "color32_serde")]
    pub note_stroke: Color32,
    #[serde(with = "color32_serde")]
    pub note_stroke_selected: Color32,
    /// Border for the note under the playhead while transport is playing.
    #[serde(
        default = "default_note_stroke_active",
        alias = "note_fill_active",
        with = "color32_serde"
    )]
    pub note_stroke_active: Color32,
    #[serde(with = "color32_serde")]
    pub note_velocity: Color32,
    #[serde(with = "color32_serde")]
    pub marquee_fill: Color32,
    #[serde(with = "color32_serde")]
    pub marquee_stroke: Color32,

    // Mixer meters
    #[serde(default = "default_meter_bg", with = "color32_serde")]
    pub meter_bg: Color32,
    #[serde(default = "default_meter_low", with = "color32_serde")]
    pub meter_low: Color32,
    #[serde(default = "default_meter_high", with = "color32_serde")]
    pub meter_high: Color32,
}

impl ThemeColors {
    /// Factory palette matching the original Motif dark UI.
    pub fn default_dark() -> Self {
        Self {
            panel_bg: Color32::from_rgb(18, 18, 22),
            text_primary: Color32::from_rgb(220, 220, 230),
            text_muted: Color32::from_rgb(160, 160, 175),
            accent: Color32::from_rgb(100, 170, 255),
            accent_warning: Color32::from_rgb(220, 180, 80),
            separator: Color32::from_rgb(55, 55, 68),

            widget_bg: Color32::from_rgb(40, 40, 50),
            widget_bg_hovered: Color32::from_rgb(55, 55, 70),
            widget_bg_active: Color32::from_rgb(70, 90, 120),
            button_text: Color32::from_rgb(230, 230, 240),

            scrollbar_track: default_scrollbar_track(),
            scrollbar_handle: default_scrollbar_handle(),
            scrollbar_handle_hovered: default_scrollbar_handle_hovered(),
            scrollbar_handle_active: default_scrollbar_handle_active(),

            ruler_bg: Color32::from_rgb(28, 28, 34),
            gutter_bg: Color32::from_rgb(22, 22, 28),
            tick_major: Color32::from_rgb(130, 130, 150),
            tick_minor: Color32::from_rgb(70, 70, 88),
            tick_sub: Color32::from_rgb(52, 52, 64),
            ruler_text: Color32::from_rgb(190, 190, 205),
            playhead: Color32::from_rgb(255, 90, 90),
            grid_bar: Color32::from_rgb(90, 90, 110),
            grid_beat: Color32::from_rgb(45, 45, 58),
            grid_subbeat: Color32::from_rgb(34, 34, 44),

            track_header_bg: Color32::from_rgb(40, 40, 50),
            track_header_text: Color32::from_rgb(210, 210, 220),
            lane_bg: Color32::from_rgb(22, 22, 28),
            clip_fill: Color32::from_rgb(60, 110, 180),
            clip_fill_selected: Color32::from_rgb(100, 170, 255),
            clip_stroke: Color32::from_rgb(140, 180, 230),
            clip_stroke_selected: Color32::WHITE,
            clip_label: Color32::from_rgb(240, 240, 250),
            clip_note_preview: Color32::from_rgba_unmultiplied(255, 255, 255, 120),

            key_row_black: Color32::from_rgb(26, 26, 32),
            key_row_white: Color32::from_rgb(32, 32, 40),
            keys_bg: Color32::from_rgb(48, 48, 56),
            white_key: Color32::from_rgb(232, 232, 238),
            white_key_active: Color32::from_rgb(255, 200, 120),
            white_key_border: Color32::from_rgb(150, 150, 160),
            white_key_label: Color32::from_rgb(40, 40, 55),
            black_key: Color32::from_rgb(28, 28, 34),
            black_key_active: Color32::from_rgb(255, 160, 70),
            black_key_border: Color32::from_rgb(12, 12, 16),
            key_divider: Color32::from_rgb(70, 70, 85),
            note_fill: Color32::from_rgb(70, 130, 220),
            note_fill_selected: Color32::from_rgb(120, 190, 255),
            note_stroke: Color32::from_rgb(180, 210, 255),
            note_stroke_selected: Color32::WHITE,
            note_stroke_active: default_note_stroke_active(),
            note_velocity: Color32::from_rgba_unmultiplied(255, 255, 255, 70),
            marquee_fill: Color32::from_rgba_unmultiplied(120, 180, 255, 40),
            marquee_stroke: Color32::from_rgb(160, 210, 255),

            meter_bg: default_meter_bg(),
            meter_low: default_meter_low(),
            meter_high: default_meter_high(),
        }
    }

    /// Apply chrome colors to egui visuals (panels, buttons, text).
    pub fn apply_to_context(&self, ctx: &Context) {
        let mut visuals = Visuals::dark();
        visuals.panel_fill = self.panel_bg;
        visuals.window_fill = self.panel_bg;
        visuals.extreme_bg_color = self.gutter_bg;
        visuals.faint_bg_color = self.widget_bg;
        visuals.override_text_color = Some(self.text_primary);
        visuals.widgets.noninteractive.bg_fill = self.panel_bg;
        visuals.widgets.noninteractive.fg_stroke.color = self.text_muted;
        visuals.widgets.inactive.bg_fill = self.widget_bg;
        visuals.widgets.inactive.weak_bg_fill = self.widget_bg;
        visuals.widgets.inactive.fg_stroke.color = self.button_text;
        visuals.widgets.hovered.bg_fill = self.widget_bg_hovered;
        visuals.widgets.hovered.weak_bg_fill = self.widget_bg_hovered;
        visuals.widgets.hovered.fg_stroke.color = self.button_text;
        visuals.widgets.active.bg_fill = self.widget_bg_active;
        visuals.widgets.active.weak_bg_fill = self.widget_bg_active;
        visuals.widgets.active.fg_stroke.color = self.button_text;
        visuals.widgets.open.bg_fill = self.widget_bg_active;
        visuals.widgets.open.weak_bg_fill = self.widget_bg_active;
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.45);
        visuals.selection.stroke.color = self.accent;
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.accent_warning;
        ctx.set_visuals(visuals);
    }

    /// Labeled slots for the Settings color editor (stable order).
    pub fn editable_slots_mut(&mut self) -> Vec<(&'static str, &'static str, &mut Color32)> {
        vec![
            ("Shell", "Panel background", &mut self.panel_bg),
            ("Shell", "Text primary", &mut self.text_primary),
            ("Shell", "Text muted", &mut self.text_muted),
            ("Shell", "Accent", &mut self.accent),
            ("Shell", "Warning / capture", &mut self.accent_warning),
            ("Shell", "Separator", &mut self.separator),
            ("Shell", "Widget background", &mut self.widget_bg),
            ("Shell", "Widget hovered", &mut self.widget_bg_hovered),
            ("Shell", "Widget active", &mut self.widget_bg_active),
            ("Shell", "Button text", &mut self.button_text),
            ("Shell", "Scrollbar track", &mut self.scrollbar_track),
            ("Shell", "Scrollbar handle", &mut self.scrollbar_handle),
            (
                "Shell",
                "Scrollbar handle hovered",
                &mut self.scrollbar_handle_hovered,
            ),
            (
                "Shell",
                "Scrollbar handle active",
                &mut self.scrollbar_handle_active,
            ),
            ("Timeline", "Ruler background", &mut self.ruler_bg),
            ("Timeline", "Gutter background", &mut self.gutter_bg),
            ("Timeline", "Tick major (bar)", &mut self.tick_major),
            ("Timeline", "Tick minor (beat)", &mut self.tick_minor),
            ("Timeline", "Tick sub (snap)", &mut self.tick_sub),
            ("Timeline", "Ruler text", &mut self.ruler_text),
            ("Timeline", "Playhead", &mut self.playhead),
            ("Timeline", "Grid bar", &mut self.grid_bar),
            ("Timeline", "Grid beat", &mut self.grid_beat),
            ("Timeline", "Grid sub-beat", &mut self.grid_subbeat),
            (
                "Playlist",
                "Track header background",
                &mut self.track_header_bg,
            ),
            ("Playlist", "Track header text", &mut self.track_header_text),
            ("Playlist", "Lane background", &mut self.lane_bg),
            ("Playlist", "Clip fill", &mut self.clip_fill),
            (
                "Playlist",
                "Clip fill selected",
                &mut self.clip_fill_selected,
            ),
            ("Playlist", "Clip stroke", &mut self.clip_stroke),
            (
                "Playlist",
                "Clip stroke selected",
                &mut self.clip_stroke_selected,
            ),
            ("Playlist", "Clip label", &mut self.clip_label),
            ("Playlist", "Clip note preview", &mut self.clip_note_preview),
            (
                "Piano roll",
                "Key row (black pitch)",
                &mut self.key_row_black,
            ),
            (
                "Piano roll",
                "Key row (white pitch)",
                &mut self.key_row_white,
            ),
            ("Piano roll", "Keys background", &mut self.keys_bg),
            ("Piano roll", "White key", &mut self.white_key),
            ("Piano roll", "White key active", &mut self.white_key_active),
            ("Piano roll", "White key border", &mut self.white_key_border),
            ("Piano roll", "White key label", &mut self.white_key_label),
            ("Piano roll", "Black key", &mut self.black_key),
            ("Piano roll", "Black key active", &mut self.black_key_active),
            ("Piano roll", "Black key border", &mut self.black_key_border),
            ("Piano roll", "Key divider", &mut self.key_divider),
            ("Piano roll", "Note fill", &mut self.note_fill),
            (
                "Piano roll",
                "Note fill selected",
                &mut self.note_fill_selected,
            ),
            ("Piano roll", "Note stroke", &mut self.note_stroke),
            (
                "Piano roll",
                "Note stroke selected",
                &mut self.note_stroke_selected,
            ),
            (
                "Piano roll",
                "Note stroke active",
                &mut self.note_stroke_active,
            ),
            ("Piano roll", "Note velocity", &mut self.note_velocity),
            ("Piano roll", "Marquee fill", &mut self.marquee_fill),
            ("Piano roll", "Marquee stroke", &mut self.marquee_stroke),
            ("Mixer", "Meter background", &mut self.meter_bg),
            ("Mixer", "Meter low", &mut self.meter_low),
            ("Mixer", "Meter high / clip", &mut self.meter_high),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub colors: ThemeColors,
}

impl Theme {
    pub fn default_dark() -> Self {
        Self {
            name: DEFAULT_THEME_NAME.to_string(),
            colors: ThemeColors::default_dark(),
        }
    }
}

/// Named themes + which one is active. Persisted in `settings.json`.
#[derive(Debug, Clone)]
pub struct ThemeCatalog {
    active_theme: String,
    themes: Vec<Theme>,
}

impl Default for ThemeCatalog {
    fn default() -> Self {
        Self {
            active_theme: DEFAULT_THEME_NAME.to_string(),
            themes: vec![Theme::default_dark()],
        }
    }
}

impl ThemeCatalog {
    pub fn from_stored(active_theme: String, themes: Vec<Theme>) -> Self {
        let mut catalog = if themes.is_empty() {
            Self::default()
        } else {
            Self {
                active_theme,
                themes,
            }
        };
        catalog.ensure_default_theme();
        catalog.ensure_active_exists();
        catalog
    }

    pub fn active_name(&self) -> &str {
        &self.active_theme
    }

    pub fn theme_names(&self) -> Vec<String> {
        self.themes.iter().map(|theme| theme.name.clone()).collect()
    }

    pub fn colors(&self) -> &ThemeColors {
        self.active_theme_ref()
            .map(|theme| &theme.colors)
            .unwrap_or(&self.themes[0].colors)
    }

    pub fn colors_mut(&mut self) -> &mut ThemeColors {
        let name = self.active_theme.clone();
        if let Some(index) = self.themes.iter().position(|theme| theme.name == name) {
            return &mut self.themes[index].colors;
        }
        self.ensure_default_theme();
        self.active_theme = DEFAULT_THEME_NAME.to_string();
        &mut self.themes[0].colors
    }

    pub fn set_active(&mut self, name: &str) -> bool {
        if self.themes.iter().any(|theme| theme.name == name) {
            self.active_theme = name.to_string();
            true
        } else {
            false
        }
    }

    /// Save current colors under `name` (create or overwrite), then select it.
    pub fn save_as(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("theme name cannot be empty".into());
        }
        let colors = self.colors().clone();
        if let Some(existing) = self.themes.iter_mut().find(|theme| theme.name == name) {
            existing.colors = colors;
        } else {
            self.themes.push(Theme {
                name: name.to_string(),
                colors,
            });
        }
        self.active_theme = name.to_string();
        Ok(())
    }

    /// Delete a user theme. Factory Default Dark cannot be removed.
    pub fn delete(&mut self, name: &str) -> Result<(), String> {
        if name == DEFAULT_THEME_NAME {
            return Err("cannot delete the factory Default Dark theme".into());
        }
        let Some(index) = self.themes.iter().position(|theme| theme.name == name) else {
            return Err("theme not found".into());
        };
        self.themes.remove(index);
        if self.active_theme == name {
            self.active_theme = DEFAULT_THEME_NAME.to_string();
        }
        self.ensure_default_theme();
        Ok(())
    }

    /// Reset the active theme's colors to the factory Default Dark palette.
    pub fn reset_active_colors_to_factory(&mut self) {
        let colors = ThemeColors::default_dark();
        *self.colors_mut() = colors;
    }

    pub fn stored(&self) -> (String, Vec<Theme>) {
        (self.active_theme.clone(), self.themes.clone())
    }

    fn active_theme_ref(&self) -> Option<&Theme> {
        self.themes
            .iter()
            .find(|theme| theme.name == self.active_theme)
    }

    fn ensure_default_theme(&mut self) {
        if !self
            .themes
            .iter()
            .any(|theme| theme.name == DEFAULT_THEME_NAME)
        {
            self.themes.insert(0, Theme::default_dark());
        }
    }

    fn ensure_active_exists(&mut self) {
        if !self
            .themes
            .iter()
            .any(|theme| theme.name == self.active_theme)
        {
            self.active_theme = DEFAULT_THEME_NAME.to_string();
        }
    }
}

fn default_note_stroke_active() -> Color32 {
    Color32::from_rgb(255, 180, 70)
}

fn default_meter_bg() -> Color32 {
    Color32::from_rgb(28, 28, 36)
}

fn default_meter_low() -> Color32 {
    Color32::from_rgb(70, 180, 100)
}

fn default_meter_high() -> Color32 {
    Color32::from_rgb(230, 80, 70)
}

fn default_scrollbar_track() -> Color32 {
    Color32::from_rgb(42, 42, 54)
}

fn default_scrollbar_handle() -> Color32 {
    Color32::from_rgb(120, 120, 145)
}

fn default_scrollbar_handle_hovered() -> Color32 {
    Color32::from_rgb(155, 155, 180)
}

fn default_scrollbar_handle_active() -> Color32 {
    Color32::from_rgb(100, 170, 255)
}

mod color32_serde {
    use egui::Color32;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(color: &Color32, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let [r, g, b, a] = color.to_array();
        [r, g, b, a].serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Color32, D::Error>
    where
        D: Deserializer<'de>,
    {
        let rgba = <[u8; 4]>::deserialize(deserializer)?;
        Ok(Color32::from_rgba_unmultiplied(
            rgba[0], rgba[1], rgba[2], rgba[3],
        ))
    }
}
