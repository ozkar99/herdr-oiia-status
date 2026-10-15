//! Just enough of the Kitty graphics protocol to flip between uploaded frames.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

/// Ask the terminal (herdr, inside a pane) whether it understands Kitty
/// graphics. A DA1 query follows the graphics query, so terminals that ignore
/// graphics still answer something and we never wait for the full timeout.
pub fn supported() -> bool {
    let mut out = io::stdout();
    if out
        .write_all(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c")
        .and_then(|()| out.flush())
        .is_err()
    {
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(1_000);
    let mut reply = Vec::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        let mut fds = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        let wait = deadline.saturating_duration_since(Instant::now()).as_millis() as i32;
        // SAFETY: one valid pollfd for stdin.
        if unsafe { libc::poll(&mut fds, 1, wait) } <= 0 {
            break;
        }
        match io::stdin().read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => reply.extend_from_slice(&buf[..n]),
        }
        let text = String::from_utf8_lossy(&reply);
        if text.contains("_Gi=31;OK") {
            return true;
        }
        // DA1 answer (ESC [ ? ... c) arrived without a graphics OK.
        if let Some(at) = text.find("\x1b[?")
            && text[at..].contains('c')
        {
            return false;
        }
    }
    false
}

/// Transmit RGBA pixels as image `id` without displaying them.
pub fn upload(out: &mut impl Write, id: u32, width: usize, height: usize, rgba: &[u8]) -> io::Result<()> {
    let data = base64(rgba);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    for (n, chunk) in chunks.iter().enumerate() {
        let more = u8::from(n + 1 < chunks.len());
        if n == 0 {
            write!(out, "\x1b_Ga=t,q=2,f=32,i={id},s={width},v={height},m={more};")?;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    Ok(())
}

/// Place image `id` at the cursor, scaled to `rows` cells tall.
pub fn place(out: &mut impl Write, id: u32, rows: usize) -> io::Result<()> {
    write!(out, "\x1b_Ga=p,q=2,i={id},p=1,r={rows},C=1\x1b\\")
}

/// Remove the placements of image `id`, keeping its data for reuse.
pub fn hide(out: &mut impl Write, id: u32) -> io::Result<()> {
    write!(out, "\x1b_Ga=d,q=2,d=i,i={id}\x1b\\")
}

/// Delete image `id` and its data.
pub fn free(out: &mut impl Write, id: u32) -> io::Result<()> {
    write!(out, "\x1b_Ga=d,q=2,d=I,i={id}\x1b\\")
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
