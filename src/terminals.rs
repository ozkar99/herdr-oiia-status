//! Points terminals at the cat font: a managed block in each installed
//! terminal's config maps the font's private-use code points to it.

use std::path::{Path, PathBuf};

use crate::managed;

struct Terminal {
    name: &'static str,
    config: PathBuf,
    installed: bool,
}

fn terminals(home: &Path) -> Vec<Terminal> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));

    let kitty_dir = std::env::var_os("KITTY_CONFIG_DIRECTORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_home.join("kitty"));
    // Ghostty reads $XDG_CONFIG_HOME/ghostty/config on Linux and macOS alike;
    // newer versions prefer config.ghostty when it exists.
    let ghostty_dir = config_home.join("ghostty");
    let ghostty_config = ["config.ghostty", "config"]
        .map(|name| ghostty_dir.join(name))
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| ghostty_dir.join("config"));

    vec![
        Terminal {
            name: "kitty",
            installed: installed(&kitty_dir, "kitty", "kitty.app"),
            config: kitty_dir.join("kitty.conf"),
        },
        Terminal {
            name: "ghostty",
            installed: installed(&ghostty_dir, "ghostty", "Ghostty.app"),
            config: ghostty_config,
        },
    ]
}

/// Whether any terminal that can show the font cat is installed.
pub fn any_installed(home: &Path) -> bool {
    terminals(home).iter().any(|t| t.installed)
}

/// Write the font mapping for every installed terminal; say what changed.
pub fn configure(home: &Path, family: &str, first: u32, last: u32) -> std::io::Result<Vec<String>> {
    let range = format!("U+{first:X}-U+{last:X}");
    let mut notes = Vec::new();
    for terminal in terminals(home).into_iter().filter(|t| t.installed) {
        let body = match terminal.name {
            // narrow_symbols keeps each glyph to exactly one cell, so a frame's
            // tiles line up instead of being squeezed or cropped.
            "kitty" => format!("symbol_map {range} {family}\nnarrow_symbols {range} 1\n"),
            _ => format!("font-codepoint-map = {range}={family}\n"),
        };
        if managed::write(&terminal.config, &body)? {
            notes.push(format!(
                "{}: mapped {range} in {}; restart it to load the font",
                terminal.name,
                terminal.config.display()
            ));
        }
    }
    Ok(notes)
}

/// Remove the font mapping from every terminal config.
pub fn unconfigure(home: &Path) -> std::io::Result<Vec<String>> {
    let mut notes = Vec::new();
    for terminal in terminals(home) {
        if managed::remove(&terminal.config)? {
            notes.push(format!("{}: removed the mapping from {}", terminal.name, terminal.config.display()));
        }
    }
    Ok(notes)
}

fn installed(config_dir: &Path, program: &str, mac_app: &str) -> bool {
    config_dir.is_dir()
        || std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
        })
        || (cfg!(target_os = "macos") && Path::new("/Applications").join(mac_app).exists())
}
