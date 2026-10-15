//! Zero-config setup, run whenever the animator starts: put the cat in the
//! sidebar layout and, where a terminal can show it, install the cat font.
//! Everything lives in marker-fenced blocks, so `unconfigure` can take it out
//! again. `auto_setup = false` in the plugin config turns it off.

use std::path::{Path, PathBuf};

use crate::config::{self, Config};
use crate::herdr::Client;
use crate::{Env, cat, font, managed, terminals};

/// Bump when the font layout changes, to rebuild installed fonts.
const FONT_VERSION: u32 = 1;
const FONT_SIGNATURE_FILE: &str = "font-signature";

/// Bring the setup up to date. Problems are reported, never fatal: the cat
/// still animates with whatever is in place.
pub fn ensure(env: &Env, config: &Config, client: &mut Client) -> Vec<String> {
    if !config.auto_setup {
        return Vec::new();
    }
    let mut notes = Vec::new();
    match ensure_sidebar(config) {
        Ok(Some(note)) => {
            notes.push(note);
            if let Err(err) = client.reload_config() {
                notes.push(format!("could not reload herdr's config: {err}"));
            }
        }
        Ok(None) => {}
        Err(err) => notes.push(format!("sidebar setup failed: {err}")),
    }
    if let Some(home) = home()
        && terminals::any_installed(&home)
        && font_outdated(env, config)
    {
        match install_font(env, config, None) {
            Ok(more) => notes.extend(more),
            Err(err) => notes.push(format!("font setup failed: {err}")),
        }
    }
    notes
}

/// Remove everything setup added: sidebar layout, terminal mappings, font.
pub fn unconfigure(env: &Env, client: &mut Client) -> std::io::Result<Vec<String>> {
    let mut notes = Vec::new();
    if let Some(path) = herdr_config_path()
        && managed::remove(&path)?
    {
        notes.push(format!("removed the sidebar layout from {}", path.display()));
        client.reload_config()?;
    }
    if let Some(home) = home() {
        notes.extend(terminals::unconfigure(&home)?);
        let font_path = font_dir(&home).join("oiia-cat.ttf");
        if std::fs::remove_file(&font_path).is_ok() {
            notes.push(format!("removed {}", font_path.display()));
        }
    }
    if let Some(state) = &env.state_dir {
        let _ = std::fs::remove_file(state.join(config::FONT_FRAMES_FILE));
        let _ = std::fs::remove_file(state.join(FONT_SIGNATURE_FILE));
    }
    Ok(notes)
}

// ---- sidebar ---------------------------------------------------------------

/// Herdr's config file, resolved the way herdr resolves it.
pub fn herdr_config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => home()?.join(".config"),
    };
    Some(base.join("herdr").join("config.toml"))
}

/// Write the cat layout into herdr's config, unless the user has their own
/// `[ui.sidebar.agents]`. Returns a note when the file changed.
fn ensure_sidebar(config: &Config) -> std::io::Result<Option<String>> {
    let Some(path) = herdr_config_path() else { return Ok(None) };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let (rest, _) = managed::without_block(&text);
    let Ok(user) = rest.parse::<toml::Table>() else {
        return Ok(Some(format!("{} does not parse; leaving the sidebar alone", path.display())));
    };
    let has_own_layout = user
        .get("ui")
        .and_then(|ui| ui.get("sidebar"))
        .and_then(|sidebar| sidebar.get("agents"))
        .is_some();
    if has_own_layout {
        // Respect it, and drop a stale block of ours that would now clash.
        managed::remove(&path)?;
        return Ok(None);
    }

    let updated = managed::with_block(&text, &layout(config, &palette(&user)));
    if updated == text {
        return Ok(None);
    }
    if updated.parse::<toml::Table>().is_err() {
        return Ok(Some("the sidebar layout would not merge cleanly; leaving it alone".into()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, updated)?;
    Ok(Some(format!("put the cat in the sidebar layout in {}", path.display())))
}

/// The default agent layout with `state_icon` replaced by the cat token, plus
/// one row per extra row of the font cat.
fn layout(config: &Config, palette: &Palette) -> String {
    let token = &config.token;
    let states = &config.states;
    let rules: Vec<String> = [
        (&states.idle, &palette.idle),
        (&states.blocked, &palette.blocked),
        (&states.done, &palette.done),
        (&states.unknown, &palette.unknown),
    ]
    .into_iter()
    .filter(|(glyph, _)| !glyph.is_empty())
    .map(|(glyph, color)| format!("{{ equals = {}, fg = \"{color}\" }}", quote(glyph)))
    .collect();

    let rows = config.font_grid[1].clamp(1, 8) as usize;
    let mut out = String::from("[ui.sidebar.agents]\nrows = [\n");
    out += &format!(
        "  [{{ token = \"${token}\", fg = \"{}\", rules = [{}] }}, \"machine\", \"workspace\", \"tab\"],\n",
        palette.working,
        rules.join(", ")
    );
    for row in 2..=rows.max(2) {
        let cat = (row <= rows).then(|| format!("{{ token = \"${token}{row}\", fg = \"{}\" }}", palette.working));
        let agent = (row == 2).then(|| "\"agent\"".to_string());
        let cells: Vec<String> = cat.into_iter().chain(agent).collect();
        out += &format!("  [{}],\n", cells.join(", "));
    }
    out += "]\n";
    out
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Herdr's state colors in the user's theme: working, idle, blocked, done,
/// unknown map to its yellow, green, red, teal and overlay0.
struct Palette {
    working: String,
    idle: String,
    blocked: String,
    done: String,
    unknown: String,
}

/// (theme, yellow, red, teal, green, overlay0), from herdr's built-in themes.
const THEMES: &[(&str, [&str; 5])] = &[
    ("catppuccin", ["#f9e2af", "#f38ba8", "#94e2d5", "#a6e3a1", "#6c7086"]),
    ("catppuccin-latte", ["#df8e1d", "#d20f39", "#179299", "#40a02b", "#9ca0b0"]),
    // ANSI colors in herdr; these are common terminal renderings of them.
    ("terminal", ["#e5c07b", "#ff6b6b", "#56b6c2", "#98c379", "#808080"]),
    ("tokyo-night", ["#e0af68", "#f7768e", "#7dcfff", "#9ece6a", "#565f89"]),
    ("tokyo-night-day", ["#8c6c3e", "#f52a65", "#118c74", "#587539", "#8990b3"]),
    ("dracula", ["#f1fa8c", "#ff5555", "#8be9fd", "#50fa7b", "#6272a4"]),
    ("nord", ["#ebcb8b", "#bf616a", "#8fbcbb", "#a3be8c", "#4c566a"]),
    ("gruvbox", ["#fabd2f", "#fb4934", "#8ec07c", "#b8bb26", "#928374"]),
    ("gruvbox-light", ["#b57614", "#9d0006", "#427b58", "#79740e", "#928374"]),
    ("one-dark", ["#e5c07b", "#e06c75", "#56b6c2", "#98c379", "#5c6370"]),
    ("one-light", ["#c18401", "#e45649", "#0184bc", "#50a14f", "#a0a1a7"]),
    ("solarized", ["#b58900", "#dc322f", "#2aa198", "#859900", "#586e75"]),
    ("solarized-light", ["#b58900", "#dc322f", "#2aa198", "#859900", "#93a1a1"]),
    ("kanagawa", ["#c0a36e", "#c34043", "#7fb4ca", "#76946a", "#727169"]),
    ("kanagawa-lotus", ["#77713f", "#c84053", "#4e8ca2", "#6f894e", "#a09cac"]),
    ("rose-pine", ["#f6c177", "#eb6f92", "#9ccfd8", "#31748f", "#6e6a86"]),
    ("rose-pine-dawn", ["#ea9d34", "#b4637a", "#56949f", "#286983", "#9893a5"]),
    ("vesper", ["#ffc799", "#ff8080", "#66ddcc", "#99ffe4", "#5c5c5c"]),
];

fn palette(herdr: &toml::Table) -> Palette {
    let theme = herdr.get("theme");
    let get = |key: &str| theme.and_then(|t| t.get(key));
    let auto = get("auto_switch").and_then(toml::Value::as_bool).unwrap_or(false);
    let name = (if auto { get("dark_name").or(get("name")) } else { get("name") })
        .and_then(toml::Value::as_str)
        .unwrap_or("catppuccin");
    let base = THEMES
        .iter()
        .find(|(theme, _)| *theme == canonical_theme(name))
        .map_or(THEMES[0].1, |(_, colors)| *colors);
    let custom = |key: &str, fallback: &str| {
        get("custom")
            .and_then(|c| c.get(key))
            .and_then(toml::Value::as_str)
            .filter(|color| color.starts_with('#'))
            .unwrap_or(fallback)
            .to_string()
    };
    Palette {
        working: custom("yellow", base[0]),
        blocked: custom("red", base[1]),
        done: custom("teal", base[2]),
        idle: custom("green", base[3]),
        unknown: custom("overlay0", base[4]),
    }
}

/// Herdr's theme-name aliases.
fn canonical_theme(name: &str) -> String {
    let name = name.to_lowercase().replace([' ', '_'], "-");
    match name.as_str() {
        "catppuccin-mocha" => "catppuccin",
        "latte" | "light" => "catppuccin-latte",
        "tokyonight" => "tokyo-night",
        "tokyo-day" | "tokyonight-day" => "tokyo-night-day",
        "gruvbox-dark" => "gruvbox",
        "onedark" => "one-dark",
        "onelight" => "one-light",
        "solarized-dark" => "solarized",
        "lotus" => "kanagawa-lotus",
        "rosepine" => "rose-pine",
        "rosepine-dawn" | "dawn" => "rose-pine-dawn",
        other => other,
    }
    .to_string()
}

// ---- font -------------------------------------------------------------------

/// Build the GIF font, install it for this user, and map its code points in
/// kitty and Ghostty, which then draw the cat token as the real cat.
pub fn install_font(env: &Env, config: &Config, gif: Option<PathBuf>) -> std::io::Result<Vec<String>> {
    let gif = gif.unwrap_or_else(|| env.gif());
    let frames = cat::load(&gif)?;
    let area = cat::bounds(&frames);
    // One glyph per terminal cell, so the terminal never has to squeeze or
    // crop a wide picture into one character.
    let [cols, rows] = config.font_grid.map(|n| n.clamp(1, 8) as usize);
    let (stand, spin) = cat::poses(&frames);
    let mut images = Vec::new();
    for &index in std::iter::once(&stand).chain(&spin) {
        images.extend(cat::tiles(&frames[index], area, cols, rows, font::TILE_WIDTH, font::TILE_HEIGHT));
    }
    let bytes = font::build(&images, font::Format::native())?;

    let home = home().ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    let dir = font_dir(&home);
    std::fs::create_dir_all(&dir)?;
    let font_path = dir.join("oiia-cat.ttf");
    std::fs::write(&font_path, bytes)?;
    if !cfg!(target_os = "macos") {
        let _ = std::process::Command::new("fc-cache").arg("-f").arg(&dir).status();
    }
    let mut notes = vec![format!(
        "installed {} (1 standing pose + {} spin poses)",
        font_path.display(),
        spin.len()
    )];

    let state_dir = env.state_dir.clone().unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&state_dir)?;
    std::fs::write(state_dir.join(config::FONT_FRAMES_FILE), format!("{cols} {rows}\n{}\n", spin.len()))?;
    std::fs::write(state_dir.join(FONT_SIGNATURE_FILE), font_signature(&gif, config))?;

    // Never signal a terminal to reload: older kitty exits on SIGUSR1.
    let last = font::FIRST_CODEPOINT + images.len() as u32 - 1;
    notes.extend(terminals::configure(&home, font::FAMILY, font::FIRST_CODEPOINT, last)?);
    Ok(notes)
}

/// Whether the installed font is missing or was built from something else.
fn font_outdated(env: &Env, config: &Config) -> bool {
    let Some(home) = home() else { return false };
    let Some(state) = &env.state_dir else { return false };
    let Some(gif) = env.gif_or_none() else { return false };
    let installed = std::fs::read_to_string(state.join(FONT_SIGNATURE_FILE)).unwrap_or_default();
    !font_dir(&home).join("oiia-cat.ttf").is_file() || installed != font_signature(&gif, config)
}

fn font_signature(gif: &Path, config: &Config) -> String {
    let meta = std::fs::metadata(gif).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    format!(
        "v{FONT_VERSION} {} {} {} {modified} {}",
        config.font_grid[0],
        config.font_grid[1],
        meta.map_or(0, |m| m.len()),
        gif.display()
    )
}

fn font_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Fonts")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("fonts")
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_valid_toml_with_theme_colors() {
        // The default 4x2 font cat spans two sidebar rows.
        let config = Config::default();
        let herdr: toml::Table = "[theme]\nname = \"Tokyo Night\"\n[theme.custom]\nred = \"#123456\"\n".parse().unwrap();
        let text = layout(&config, &palette(&herdr));
        let parsed: toml::Table = text.parse().unwrap();
        let rows = parsed["ui"]["sidebar"]["agents"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(text.contains("#e0af68"), "tokyo-night yellow for working");
        assert!(text.contains("#123456"), "custom red for blocked");
        assert!(text.contains("$oiia2"));
    }
}
