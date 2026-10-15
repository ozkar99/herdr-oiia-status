//! Points terminals at the cat font: a managed block in each installed
//! terminal's config maps the font's private-use code points to it.

use std::path::{Path, PathBuf};

const BEGIN: &str = "# BEGIN oiia.status (managed by the herdr OIIA plugin)";
const END: &str = "# END oiia.status";

/// Write the block for every terminal that is installed or already has a
/// config directory, and say what was done.
pub fn configure(home: &Path, family: &str, first: u32, last: u32) -> std::io::Result<Vec<String>> {
    let range = format!("U+{first:X}-U+{last:X}");
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let mut notes = Vec::new();

    let kitty_dir = std::env::var_os("KITTY_CONFIG_DIRECTORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_home.join("kitty"));
    if installed(&kitty_dir, "kitty", "kitty.app") {
        let path = kitty_dir.join("kitty.conf");
        // narrow_symbols keeps each glyph to exactly one cell, so a frame's
        // tiles line up instead of being squeezed or cropped.
        write_block(&path, &format!("symbol_map {range} {family}\nnarrow_symbols {range} 1\n"))?;
        notes.push(format!("kitty: mapped {range} in {}; restart kitty (or ctrl+shift+F5) to load it", path.display()));
    }

    // Ghostty reads $XDG_CONFIG_HOME/ghostty/config on Linux and macOS alike;
    // newer versions prefer config.ghostty when it exists.
    let ghostty_dir = config_home.join("ghostty");
    if installed(&ghostty_dir, "ghostty", "Ghostty.app") {
        let path = ["config.ghostty", "config"]
            .map(|name| ghostty_dir.join(name))
            .into_iter()
            .find(|path| path.is_file())
            .unwrap_or_else(|| ghostty_dir.join("config"));
        write_block(&path, &format!("font-codepoint-map = {range}={family}\n"))?;
        notes.push(format!("ghostty: mapped {range} in {}; reload its config or open a new window", path.display()));
    }

    if notes.is_empty() {
        notes.push("no kitty or Ghostty found; the sidebar will keep the text cat".into());
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

/// Replace (or append) the managed block in a config file.
fn write_block(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let block = format!("{BEGIN}\n{body}{END}\n");
    let conf = std::fs::read_to_string(path).unwrap_or_default();
    let end_line = format!("{END}\n");
    let conf = match (conf.find(BEGIN), conf.find(&end_line)) {
        (Some(start), Some(end)) if start < end => {
            format!("{}{block}{}", &conf[..start], &conf[end + end_line.len()..])
        }
        _ if conf.is_empty() || conf.ends_with('\n') => format!("{conf}{block}"),
        _ => format!("{conf}\n{block}"),
    };
    std::fs::write(path, conf)
}
