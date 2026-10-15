//! Marker-fenced blocks the plugin owns inside other programs' config files.
//! Everything outside the markers is left untouched.

use std::path::Path;

const BEGIN: &str = "# BEGIN oiia.status (managed by the herdr OIIA plugin)";
const END: &str = "# END oiia.status";

/// The file's text with our block removed, and whether it had one.
pub fn without_block(text: &str) -> (String, bool) {
    let end_line = format!("{END}\n");
    match (text.find(BEGIN), text.find(&end_line)) {
        (Some(start), Some(end)) if start < end => {
            (format!("{}{}", &text[..start], &text[end + end_line.len()..]), true)
        }
        _ => (text.to_string(), false),
    }
}

/// Text with our block (re)written at the end. `body` ends with a newline.
pub fn with_block(text: &str, body: &str) -> String {
    let (rest, _) = without_block(text);
    let block = format!("{BEGIN}\n{body}{END}\n");
    if rest.is_empty() || rest.ends_with('\n') {
        format!("{rest}{block}")
    } else {
        format!("{rest}\n{block}")
    }
}

/// Write our block into `path`, creating the file if needed. Returns whether
/// the file changed.
pub fn write(path: &Path, body: &str) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let updated = with_block(&text, body);
    if updated == text {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, updated)?;
    Ok(true)
}

/// Remove our block from `path`, if present. Returns whether it changed.
pub fn remove(path: &Path) -> std::io::Result<bool> {
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(false) };
    let (rest, had) = without_block(&text);
    if had {
        std::fs::write(path, rest)?;
    }
    Ok(had)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_and_removes_only_the_block() {
        let user = "a = 1\n";
        let once = with_block(user, "x = 1\n");
        assert!(once.starts_with(user) && once.contains("x = 1"));
        let twice = with_block(&once, "x = 2\n");
        assert!(twice.contains("x = 2") && !twice.contains("x = 1"));
        assert_eq!(without_block(&twice), (user.to_string(), true));
        assert_eq!(with_block("a = 1", "x\n").lines().next(), Some("a = 1"));
    }
}
