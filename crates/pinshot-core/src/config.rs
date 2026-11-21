//! User configuration. Defaults are good enough that a first run needs no decisions;
//! `config.toml` only overrides what the user changes (see `spec/02-architecture.md` §8).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Config {
    pub general: General,
    pub capture: Capture,
    pub export: Export,
    pub history: History,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct General {
    pub hotkey_snip: String,
    pub hotkey_paste: String,
    pub launch_at_login: bool,
    /// `auto` follows the OS language; `zh` / `en` pin the interface.
    pub language: String,
    /// Keep the pre-warmed overlay windows alive between captures. On: instant first
    /// capture, ~80–100 MB resident. Off: released 60 s after a capture.
    pub keep_warm: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Capture {
    /// How much the area outside the selection is darkened, 0.0 – 1.0.
    pub dim: f32,
    /// Loupe magnification, 2–16×.
    pub magnifier_zoom: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Export {
    pub save_dir: PathBuf,
    pub copy_to_clipboard: bool,
    pub save_to_file: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct History {
    pub keep_days: u32,
    pub keep_items: u32,
}

impl Default for General {
    fn default() -> Self {
        Self {
            hotkey_snip: "F1".into(),
            hotkey_paste: "F3".into(),
            launch_at_login: false,
            language: "auto".into(),
            keep_warm: true,
        }
    }
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            dim: 0.5,
            magnifier_zoom: 8,
        }
    }
}

impl Default for Export {
    fn default() -> Self {
        Self {
            save_dir: default_save_dir(),
            copy_to_clipboard: true,
            save_to_file: true,
        }
    }
}

impl Default for History {
    fn default() -> Self {
        Self {
            keep_days: 30,
            keep_items: 1000,
        }
    }
}

fn home() -> PathBuf {
    // Windows shells often have no `HOME`; the profile directory is the equivalent.
    #[cfg(target_os = "windows")]
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_save_dir() -> PathBuf {
    home().join("Pictures").join("PinShot")
}

/// Data directory: `~/Library/Application Support/ai.pinshot.desktop` on macOS.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home()
            .join("Library")
            .join("Application Support")
            .join("ai.pinshot.desktop")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("ai.pinshot.desktop")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local").join("share"))
            .join("ai.pinshot.desktop")
    }
}

impl Config {
    pub fn path() -> PathBuf {
        data_dir().join("config.toml")
    }

    /// Loads `config.toml` from the data directory, falling back to defaults.
    ///
    /// A malformed file is reported but never fatal: starting with defaults beats
    /// refusing to start (`spec/02-architecture.md` §8).
    pub fn load() -> (Self, Option<String>) {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml_like(&text) {
                Ok(config) => (config, None),
                Err(error) => (
                    Self::default(),
                    Some(format!("{}: {error}", path.display())),
                ),
            },
            Err(_) => (Self::default(), None),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write then rename: a crash mid-write must not leave a truncated config that
        // load_from would silently replace with all defaults on the next launch.
        let temporary = path.with_extension("toml.part");
        std::fs::write(&temporary, self.to_toml())?;
        // On Windows rename fails when the destination exists; remove it first (the
        // store crate's save_blob relies on the "exists" fast path for the same reason).
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(&temporary, path)
    }

    /// The file is generated by the app, so it stays flat and readable rather than
    /// pulling a full TOML writer for six sections.
    pub fn to_toml(&self) -> String {
        format!(
            "[general]\nhotkey_snip = \"{}\"\nhotkey_paste = \"{}\"\nlaunch_at_login = {}\nlanguage = \"{}\"\nkeep_warm = {}\n\n\
             [capture]\ndim = {}\nmagnifier_zoom = {}\n\n\
             [export]\nsave_dir = \"{}\"\ncopy_to_clipboard = {}\nsave_to_file = {}\n\n\
             [history]\nkeep_days = {}\nkeep_items = {}\n",
            self.general.hotkey_snip,
            self.general.hotkey_paste,
            self.general.launch_at_login,
            self.general.language,
            self.general.keep_warm,
            self.capture.dim,
            self.capture.magnifier_zoom,
            toml_escape(&self.export.save_dir.display().to_string()),
            self.export.copy_to_clipboard,
            self.export.save_to_file,
            self.history.keep_days,
            self.history.keep_items,
        )
    }
}

/// Backslashes are literal in TOML basic strings, so `C:\Users\me` would make the
/// file unreadable for anything but our own lenient parser. Escape both characters
/// that matter and undo it on the way in.
fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn toml_unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            // An older Windows config has single backslashes; keep them as they are.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Strip a trailing `#` comment, but never a `#` inside a quoted string. A `save_dir`
/// like `~/Pictures/C# shots` is legal on both macOS and Windows; `split('#')` would cut
/// the path in half while our own writer emits it as a valid quoted string.
fn strip_comment(raw: &str) -> &str {
    let mut in_quotes = false;
    let mut escaped = false;
    for (index, c) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return &raw[..index],
            _ => {}
        }
    }
    raw
}

/// Minimal TOML reader for the flat `[section] key = value` files this app writes.
///
/// A full TOML parser is not worth a dependency: the file is generated by the app,
/// hand edits are rare, and anything unrecognised falls back to the default.
fn toml_like(text: &str) -> Result<Config, String> {
    let mut config = Config::default();
    let mut section = String::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected key = value", line_no + 1));
        };
        let key = key.trim();
        let value = toml_unescape(value.trim().trim_matches('"'));
        let enabled = value.eq_ignore_ascii_case("true");
        match (section.as_str(), key) {
            ("general", "hotkey_snip") => config.general.hotkey_snip = value.clone(),
            ("general", "hotkey_paste") => config.general.hotkey_paste = value.clone(),
            ("general", "launch_at_login") => config.general.launch_at_login = enabled,
            ("general", "language") => config.general.language = value.clone(),
            ("general", "keep_warm") => config.general.keep_warm = enabled,

            ("capture", "dim") => {
                let parsed = value.parse().unwrap_or(config.capture.dim);
                // f32::clamp(NaN) returns NaN (both comparisons are false), which would
                // leak an invalid rgba() string into the Canvas. Fall back instead.
                config.capture.dim = if parsed.is_finite() {
                    parsed.clamp(0.0, 1.0)
                } else {
                    config.capture.dim
                }
            }
            ("capture", "magnifier_zoom") => {
                config.capture.magnifier_zoom = value.parse().unwrap_or(8).clamp(2, 16)
            }
            ("export", "save_dir") => config.export.save_dir = PathBuf::from(&value),
            ("export", "copy_to_clipboard") => config.export.copy_to_clipboard = enabled,
            ("export", "save_to_file") => config.export.save_to_file = enabled,
            // Same range guard as dim/zoom: a hand-edited `keep_days = 0` would set the
            // prune cutoff to "now" and silently delete the entire non-favourite history.
            ("history", "keep_days") => {
                config.history.keep_days = value.parse().unwrap_or(30).clamp(1, 3650)
            }
            ("history", "keep_items") => {
                config.history.keep_items = value.parse().unwrap_or(1000).clamp(1, 1_000_000)
            }
            _ => {}
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_usable_without_a_file() {
        let config = Config::default();
        assert_eq!(config.general.hotkey_snip, "F1");
        assert!((config.capture.dim - 0.5).abs() < f32::EPSILON);
        assert!(config.export.save_dir.ends_with("PinShot"));
    }

    #[test]
    fn reads_the_sections_we_write() {
        let config = toml_like(
            r#"
            [general]
            hotkey_snip = "F2"

            [capture]
            dim = 0.35

            [export]
            save_dir = "/tmp/shots"
            "#,
        )
        .unwrap();
        assert_eq!(config.general.hotkey_snip, "F2");
        assert!((config.capture.dim - 0.35).abs() < 1e-6);
        assert_eq!(config.export.save_dir, PathBuf::from("/tmp/shots"));
    }

    #[test]
    fn hash_inside_a_quoted_path_is_not_a_comment() {
        let config = toml_like(
            r#"
            [export]
            save_dir = "/tmp/C# shots"
            "#,
        )
        .unwrap();
        assert_eq!(config.export.save_dir, PathBuf::from("/tmp/C# shots"));
    }

    #[test]
    fn history_retention_values_are_clamped() {
        let config = toml_like(
            r#"
            [history]
            keep_days = 0
            keep_items = 0
            "#,
        )
        .unwrap();
        assert!(config.history.keep_days >= 1);
        assert!(config.history.keep_items >= 1);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("pinshot-config-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut config = Config::default();
        config.general.hotkey_snip = "F2".into();
        config.general.keep_warm = false;
        config.capture.dim = 0.25;
        config.export.save_dir = PathBuf::from("/tmp/shots");
        config.history.keep_items = 42;

        config.save_to(&path).unwrap();
        let (loaded, warning) = Config::load_from(&path);
        assert!(warning.is_none());
        assert_eq!(loaded, config);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn windows_paths_survive_the_round_trip() {
        let mut config = Config::default();
        config.export.save_dir = PathBuf::from(r"C:\Users\me\Pictures\PinShot");
        let text = config.to_toml();
        assert!(
            text.contains(r#"save_dir = "C:\\Users\\me\\Pictures\\PinShot""#),
            "{text}"
        );
        let loaded = toml_like(&text).unwrap();
        assert_eq!(loaded.export.save_dir, config.export.save_dir);
    }

    #[test]
    fn out_of_range_values_fall_back() {
        let config = toml_like("[capture]\ndim = 4.0\nmagnifier_zoom = 99\n").unwrap();
        assert!((config.capture.dim - 1.0).abs() < f32::EPSILON);
        assert_eq!(config.capture.magnifier_zoom, 16);
    }
}
