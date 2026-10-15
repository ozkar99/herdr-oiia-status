//! User config, read from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Sidebar token the plugin reports, rendered as `$<token>` in row layouts.
    pub token: String,
    /// Animation frames per second.
    pub fps: u32,
    /// Poses of the text cat: the first is its standing pose, and a spin steps
    /// through all of them. Keep every pose the same width so the rest of the
    /// row does not jitter. Herdr trims whitespace, so pad with U+2800 (braille
    /// blank) instead of spaces.
    pub frames: Vec<String>,
    /// The font cat's standing pose and spin poses, when `install-font` has run.
    #[serde(skip)]
    pub font_poses: Option<Poses>,
    /// Glyphs shown for the other states, so `$oiia` can fully replace
    /// `state_icon`. Leave a state empty to show nothing for it.
    pub states: StateGlyphs,
    /// Cells the cat spans in the sidebar when drawn with the font:
    /// `[columns, rows]`. Rows beyond the first render as `$oiia2`, `$oiia3`...
    /// Re-run `install-font` after changing it.
    pub font_grid: [u32; 2],
    /// Put the cat in herdr's sidebar layout and install the cat font for
    /// kitty/Ghostty automatically. Off: do both yourself (see the README).
    pub auto_setup: bool,
    pub sound: SoundConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StateGlyphs {
    pub idle: String,
    pub blocked: String,
    pub done: String,
    pub unknown: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundMode {
    /// Never play anything.
    Off,
    /// Play once each time an agent starts working.
    Once,
    /// Loop for as long as any agent is working.
    Loop,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SoundConfig {
    pub mode: SoundMode,
    /// Audio file; relative paths resolve against the config dir, then the
    /// plugin's bundled `assets/`.
    pub file: PathBuf,
    /// Custom player argv. `{file}` is replaced with the audio path. Empty means
    /// auto-detect.
    pub player: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token: "oiia".into(),
            fps: 20,
            frames: Vec::new(),
            font_poses: None,
            states: StateGlyphs::default(),
            font_grid: [4, 2],
            auto_setup: true,
            sound: SoundConfig::default(),
        }
    }
}

impl Default for StateGlyphs {
    fn default() -> Self {
        // Herdr's own "symbols" style shapes, so states stay distinguishable
        // and sidebar `rules` can color them by value.
        Self {
            idle: "○".into(),
            blocked: "×".into(),
            done: "✓".into(),
            unknown: "·".into(),
        }
    }
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            mode: SoundMode::Once,
            file: "oiia.mp3".into(),
            player: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(config_dir: Option<&Path>, state_dir: Option<&Path>) -> Self {
        let path = config_dir.map(|dir| dir.join("config.toml"));
        let text = path.as_ref().and_then(|path| std::fs::read_to_string(path).ok());
        let mut config: Self = match (text, &path) {
            (Some(text), Some(path)) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("oiia: ignoring invalid {}: {err}", path.display());
                Self::default()
            }),
            _ => Self::default(),
        };
        if config.frames.is_empty() {
            config.frames = ["ᓚᘏᗢ", "⠀ᗢ⠀", "ᗢᘏᓗ", "⠀ᘏ⠀"].map(String::from).to_vec();
        }
        config.font_poses = font_poses(state_dir);
        config.fps = config.fps.clamp(1, 30);
        config
    }
}

impl SoundConfig {
    /// First existing candidate for the audio file.
    pub fn resolve_file(&self, config_dir: Option<&Path>, plugin_root: Option<&Path>) -> Option<PathBuf> {
        if self.file.is_absolute() {
            return self.file.is_file().then(|| self.file.clone());
        }
        [config_dir, plugin_root]
            .into_iter()
            .flatten()
            .flat_map(|dir| [dir.join(&self.file), dir.join("assets").join(&self.file)])
            .find(|path| path.is_file())
    }
}

/// Name of the state file `install-font` writes with the font's frame count.
pub const FONT_FRAMES_FILE: &str = "font-frames";

/// A standing pose plus the poses a spin steps through. Each pose is one or
/// more rows separated by newlines.
#[derive(Debug, Clone)]
pub struct Poses {
    pub stand: String,
    pub spin: Vec<String>,
}

/// The font cat's poses, if `install-font` has run. Its state file holds
/// `cols rows` and then the spin pose count; glyph block 0 is the standing
/// pose and blocks 1..=n the spin poses, each block `cols * rows` characters.
pub fn font_poses(state_dir: Option<&Path>) -> Option<Poses> {
    let text = std::fs::read_to_string(state_dir?.join(FONT_FRAMES_FILE)).ok()?;
    let numbers: Vec<u32> = text.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    let [cols, rows, spin_count] = numbers[..] else { return None };
    let block = |key: u32| -> Option<String> {
        let first = crate::font::FIRST_CODEPOINT + key * cols * rows;
        let lines: Option<Vec<String>> = (0..rows)
            .map(|row| {
                // Herdr indents an entry's first row by 1 cell and later rows by
                // 3, so pad the first row to keep the cat's rows aligned. U+2800
                // is blank but, unlike a space, survives herdr's trimming.
                let pad = if row == 0 && rows > 1 { "\u{2800}\u{2800}" } else { "" };
                let cells: Option<String> = (0..cols).map(|col| char::from_u32(first + row * cols + col)).collect();
                cells.map(|cells| format!("{pad}{cells}"))
            })
            .collect();
        lines.map(|lines| lines.join("\n"))
    };
    Some(Poses {
        stand: block(0)?,
        spin: (1..=spin_count).map(block).collect::<Option<_>>()?,
    })
}
