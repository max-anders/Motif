//! CLAP / VST3 scan, cache, load, and editor hosting (truce-rack).

use std::path::Path;

mod catalog;
mod editor;
#[cfg(target_os = "linux")]
mod editor_window;
mod host;

pub use catalog::{CatalogEntry, EntryCategory, PluginCatalog, PLUGIN_CACHE_FILE};
pub use editor::{EditorCloseBinding, EditorPoll, PluginEditorHost, PluginRef};
pub use host::{load_and_activate, HostedPlugin, PluginParamInfo};

/// Packed semver used by truce-rack / CLAP scan: `(major << 16) | (minor << 8) | patch`.
pub const fn packed_semver(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 16) | (minor << 8) | (patch & 0xff)
}

/// Vital 1.6.0 is the last Linux build known to open on NVIDIA EGL without abort().
const VITAL_SAFE_PACKED: u32 = packed_semver(1, 6, 0);

fn looks_like_vital(unique_id: &str, name: &str) -> bool {
    unique_id.to_ascii_lowercase().contains("vital") || name.to_ascii_lowercase().contains("vital")
}

/// Plugins whose Linux GUI abort()s the host (in-process). Vital 1.6.4 bgfx EGL
/// is the known case; 1.6.0 and earlier skip the warning. `packed_version` 0
/// means unknown (stale cache, VST3 with no probe hit) and still warns.
pub fn plugin_gui_may_abort_host(unique_id: &str, name: &str, packed_version: u32) -> bool {
    if !looks_like_vital(unique_id, name) {
        return false;
    }
    packed_version == 0 || packed_version > VITAL_SAFE_PACKED
}

/// CLAP scan version, or a `1.6.x` string found in the plugin file (VST3 has no version).
pub fn packed_version_for_entry(entry: &CatalogEntry) -> u32 {
    if entry.version != 0 {
        return entry.version;
    }
    probe_packed_semver_in_plugin_path(&entry.path)
}

fn probe_packed_semver_in_plugin_path(path: &Path) -> u32 {
    if path.is_file() {
        return std::fs::read(path)
            .map(|bytes| max_vital_16x_in_bytes(&bytes))
            .unwrap_or(0);
    }
    for so in [
        path.join("Contents/x86_64-linux/Vital.so"),
        path.join("Vital.vst3/Contents/x86_64-linux/Vital.so"),
    ] {
        if so.is_file() {
            return std::fs::read(so)
                .map(|bytes| max_vital_16x_in_bytes(&bytes))
                .unwrap_or(0);
        }
    }
    0
}

/// Highest `1.6.[0-9]` (single patch digit) found in a plugin binary.
fn max_vital_16x_in_bytes(bytes: &[u8]) -> u32 {
    let needle = b"1.6.";
    let mut best = 0u32;
    let mut i = 0usize;
    while i + 5 <= bytes.len() {
        if &bytes[i..i + 4] == needle {
            let d = bytes[i + 4];
            let next_ok = bytes.get(i + 5).map(|c| !c.is_ascii_digit()).unwrap_or(true);
            if d.is_ascii_digit() && next_ok {
                best = best.max(packed_semver(1, 6, u32::from(d - b'0')));
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    best
}

/// True when `main` applied MOTIF_PLUGIN_GL=software (Mesa llvmpipe for plugin EGL).
pub fn plugin_gl_software_enabled() -> bool {
    std::env::var("LIBGL_ALWAYS_SOFTWARE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        max_vital_16x_in_bytes, packed_semver, plugin_gui_may_abort_host, VITAL_SAFE_PACKED,
    };

    #[test]
    fn vital_identities_are_flagged() {
        assert!(plugin_gui_may_abort_host(
            "com.vital.synth",
            "Vital",
            packed_semver(1, 6, 4)
        ));
        assert!(plugin_gui_may_abort_host("Vital.vst3", "Track 1", 0));
        assert!(!plugin_gui_may_abort_host(
            "org.surge-synth.surge",
            "Surge XT",
            0
        ));
        assert!(!plugin_gui_may_abort_host(
            "audio.vital.synth",
            "Vital",
            VITAL_SAFE_PACKED
        ));
    }

    #[test]
    fn probe_ignores_1_6_37_false_positive() {
        let bytes = b"xx1.6.37yy1.6.0zz1.6.4";
        assert_eq!(max_vital_16x_in_bytes(bytes), packed_semver(1, 6, 4));
    }
}

#[cfg(not(target_os = "linux"))]
pub use editor::HostX11;
#[cfg(target_os = "linux")]
pub use editor_window::{init_xlib_threads, HostX11};
