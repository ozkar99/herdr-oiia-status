//! Points Ghostty at the cat font: a managed block in its config maps the
//! font's private-use code points to it.

use std::path::{Path, PathBuf};

use crate::managed;

/// Ghostty reads $XDG_CONFIG_HOME/ghostty/config on Linux and macOS alike;
/// newer versions prefer config.ghostty when it exists.
fn ghostty_config(home: &Path) -> (PathBuf, PathBuf) {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let dir = config_home.join("ghostty");
    let config = ["config.ghostty", "config"]
        .map(|name| dir.join(name))
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| dir.join("config"));
    (dir, config)
}

/// Whether Ghostty is installed, and so can show the font cat.
pub fn any_installed(home: &Path) -> bool {
    let (dir, _) = ghostty_config(home);
    dir.is_dir()
        || std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join("ghostty").is_file())
        })
        || (cfg!(target_os = "macos") && Path::new("/Applications/Ghostty.app").exists())
}

/// Write the font mapping into Ghostty's config when it is installed; say
/// what changed.
pub fn configure(home: &Path, family: &str, first: u32, last: u32) -> std::io::Result<Vec<String>> {
    let range = format!("U+{first:X}-U+{last:X}");
    let mut notes = Vec::new();
    if any_installed(home) {
        let (_, config) = ghostty_config(home);
        if managed::write(&config, &format!("font-codepoint-map = {range}={family}\n"))? {
            notes.push(format!(
                "ghostty: mapped {range} in {}; restart it to load the font",
                config.display()
            ));
        }
    }
    Ok(notes)
}

/// Remove the font mapping from Ghostty's config.
pub fn unconfigure(home: &Path) -> std::io::Result<Vec<String>> {
    let mut notes = Vec::new();
    let (_, config) = ghostty_config(home);
    if managed::remove(&config)? {
        notes.push(format!("ghostty: removed the mapping from {}", config.display()));
    }
    Ok(notes)
}
