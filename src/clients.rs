//! Which terminals the attached herdr clients run in.
//!
//! The `$oiia` token is one value shared by every client, but each client's
//! terminal draws it. Only Ghostty gets the font mapping that `install-font`
//! writes, so font frames are safe only when every attached client is inside
//! it. Herdr's API does not expose the client's terminal, so we look at the
//! client processes' environments.

/// True when at least one herdr client is attached and all of them run in
/// Ghostty. Anything we cannot inspect counts as "no", so the caller falls
/// back to the text cat.
pub fn all_can_draw_font() -> bool {
    let clients = client_environments();
    !clients.is_empty() && clients.iter().all(|env| can_draw_font(env))
}

fn can_draw_font(env: &str) -> bool {
    env.split(['\0', ' ']).any(|var| {
        var.starts_with("GHOSTTY_RESOURCES_DIR=")
            || var == "TERM=xterm-ghostty"
            || var == "TERM_PROGRAM=ghostty"
    })
}

/// Environment blocks of running herdr clients (not the server, not plugins).
#[cfg(target_os = "linux")]
fn client_environments() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|entry| {
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            let mut args = cmdline.split(|&b| b == 0).filter(|arg| !arg.is_empty());
            let program = args.next()?;
            let is_herdr = program.rsplit(|&b| b == b'/').next()? == b"herdr";
            // `herdr`, `herdr --session x`, `herdr session attach x`; not `herdr server`
            // or one-shot CLI calls like `herdr agent list`.
            let rest: Vec<&[u8]> = args.collect();
            let is_client = rest.is_empty()
                || rest[0].starts_with(b"-")
                || rest.starts_with(&[b"session".as_slice(), b"attach".as_slice()]);
            if !(is_herdr && is_client) {
                return None;
            }
            let environ = std::fs::read(entry.path().join("environ")).ok()?;
            Some(String::from_utf8_lossy(&environ).into_owned())
        })
        .collect()
}

/// macOS has no /proc; `ps -E` appends the environment of our own processes.
#[cfg(not(target_os = "linux"))]
fn client_environments() -> Vec<String> {
    let Ok(output) = std::process::Command::new("ps").args(["-Eww", "-x", "-o", "command="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            let mut words = line.split_whitespace();
            let program = words.next().unwrap_or_default();
            let next = words.next().unwrap_or_default();
            program.rsplit('/').next() == Some("herdr")
                && (next.is_empty() || next.starts_with('-') || next.contains('=') || next == "session")
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::can_draw_font;

    #[test]
    fn detects_font_capable_terminals() {
        assert!(can_draw_font("TERM=xterm-ghostty\0TERM_PROGRAM=ghostty\0"));
        // macOS `ps -E` output separates variables with spaces.
        assert!(can_draw_font("herdr TERM_PROGRAM=ghostty GHOSTTY_RESOURCES_DIR=/x"));
        assert!(!can_draw_font("TERM=xterm-256color\0WT_SESSION=abc\0"));
    }
}
