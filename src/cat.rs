//! Turns the green-screen OIIA GIF into text frames.

use std::fmt::Write as _;
use std::fs::File;
use std::io;
use std::path::Path;

pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// RGBA, row-major.
    pub pixels: Vec<u8>,
    pub delay_ms: u32,
}

impl Frame {
    fn rgb(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 4;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }

    /// True for cat pixels: anything that is not the green screen.
    fn is_cat(&self, x: usize, y: usize) -> bool {
        let [r, g, b] = self.rgb(x, y).map(i32::from);
        !(g > 120 && g > r + 50 && g > b + 50)
    }
}

/// Decode and composite every frame of the GIF.
pub fn load(path: &Path) -> io::Result<Vec<Frame>> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(File::open(path)?).map_err(io::Error::other)?;
    let (width, height) = (usize::from(decoder.width()), usize::from(decoder.height()));
    // Start from green so undrawn areas key out.
    let mut canvas: Vec<u8> = [0, 255, 0, 255].repeat(width * height);
    let mut frames = Vec::new();

    while let Some(frame) = decoder.read_next_frame().map_err(io::Error::other)? {
        let before = canvas.clone();
        let (left, top) = (usize::from(frame.left), usize::from(frame.top));
        let frame_width = usize::from(frame.width);
        for (i, px) in frame.buffer.chunks_exact(4).enumerate() {
            let (x, y) = (left + i % frame_width, top + i / frame_width);
            if px[3] > 0 && x < width && y < height {
                let at = (y * width + x) * 4;
                canvas[at..at + 4].copy_from_slice(px);
            }
        }
        frames.push(Frame {
            width,
            height,
            pixels: canvas.clone(),
            delay_ms: u32::from(frame.delay).max(2) * 10,
        });
        match frame.dispose {
            gif::DisposalMethod::Previous => canvas = before,
            gif::DisposalMethod::Background => {
                for y in top..(top + usize::from(frame.height)).min(height) {
                    for x in left..(left + frame_width).min(width) {
                        let at = (y * width + x) * 4;
                        canvas[at..at + 4].copy_from_slice(&[0, 255, 0, 255]);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(frames)
}

/// The GIF resampled to a fixed tick, so the sidebar keeps the GIF's own
/// rhythm (stand still, spin, stand still) at whatever rate tokens update.
pub struct Plan {
    /// For each tick of one loop, which distinct picture to show; equal
    /// neighbours mean the cat is standing still.
    pub sequence: Vec<usize>,
}

/// Below this `motion`, two frames count as the same picture. Still stretches
/// collapse into one keyframe, which keeps the font small.
const STILL: f32 = 0.002;

pub fn plan(frames: &[Frame], tick_ms: u32) -> Plan {
    let total: u32 = frames.iter().map(|f| f.delay_ms).sum::<u32>().max(1);
    let ticks = (total / tick_ms.max(1)).max(1);
    let mut keyframes: Vec<usize> = Vec::new();
    let mut sequence = Vec::new();
    for tick in 0..ticks {
        let time = tick * tick_ms;
        let mut elapsed = 0;
        let index = frames
            .iter()
            .position(|f| {
                elapsed += f.delay_ms;
                elapsed > time
            })
            .unwrap_or(frames.len() - 1);
        let same_as_last = keyframes
            .last()
            .is_some_and(|&last| last == index || motion(&frames[last], &frames[index]) < STILL);
        if !same_as_last {
            keyframes.push(index);
        }
        sequence.push(keyframes.len() - 1);
    }
    Plan { sequence }
}

/// The GIF's standing pose (its first frame) and every frame where the cat
/// moves, in order: the poses a spin can step through at any speed.
pub fn poses(frames: &[Frame]) -> (usize, Vec<usize>) {
    let spin = (1..frames.len())
        .filter(|&i| motion(&frames[i - 1], &frames[i]) >= STILL)
        .collect();
    (0, spin)
}

/// Fraction of pixels whose cat/background classification differs between
/// two frames: near zero while the cat holds still.
pub fn motion(a: &Frame, b: &Frame) -> f32 {
    let mut changed = 0usize;
    for y in (0..a.height).step_by(2) {
        for x in (0..a.width).step_by(2) {
            changed += usize::from(a.is_cat(x, y) != b.is_cat(x, y));
        }
    }
    changed as f32 / ((a.width / 2) * (a.height / 2)) as f32
}

/// Bounding box `(x0, y0, x1, y1)` of the cat across all frames, so it fills
/// the output and does not drift between frames.
pub fn bounds(frames: &[Frame]) -> (usize, usize, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for frame in frames {
        // Ignore the outermost margin, where watermarks live.
        let (mx, my) = (frame.width / 20, frame.height / 20);
        for y in my..frame.height - my {
            for x in mx..frame.width - mx {
                if frame.is_cat(x, y) {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                }
            }
        }
    }
    (x0, y0, x1, y1)
}

/// Fraction of cat pixels in a source rectangle.
fn coverage(frame: &Frame, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (xa, ya) = (x0 as usize, y0 as usize);
    let (xb, yb) = ((x1.ceil() as usize).max(xa + 1), (y1.ceil() as usize).max(ya + 1));
    let mut hits = 0;
    for y in ya..yb.min(frame.height) {
        for x in xa..xb.min(frame.width) {
            hits += usize::from(frame.is_cat(x, y));
        }
    }
    hits as f32 / ((xb - xa) * (yb - ya)) as f32
}

/// Render one frame as `rows` lines of `cols` braille cells (2x4 dots each).
pub fn braille(frame: &Frame, area: (usize, usize, usize, usize), cols: usize, rows: usize) -> Vec<String> {
    const DOTS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let (ax, ay, bx, by) = area;
    let sx = (bx - ax) as f32 / (cols * 2) as f32;
    let sy = (by - ay) as f32 / (rows * 4) as f32;
    (0..rows)
        .map(|row| {
            (0..cols)
                .map(|col| {
                    let mut bits = 0;
                    for (dy, line) in DOTS.iter().enumerate() {
                        for (dx, bit) in line.iter().enumerate() {
                            let px = ax as f32 + (col * 2 + dx) as f32 * sx;
                            let py = ay as f32 + (row * 4 + dy) as f32 * sy;
                            if coverage(frame, px, py, px + sx, py + sy) > 0.45 {
                                bits |= bit;
                            }
                        }
                    }
                    // An empty cell would be trimmed or collapse; U+2800 keeps width.
                    char::from_u32(0x2800 + bits).unwrap_or('⠀')
                })
                .collect()
        })
        .collect()
}

/// Crop to `area` and scale to `height` pixels, with the green screen made
/// transparent. Each output pixel averages its source box, so edges come out
/// anti-aliased.
pub fn rgba(frame: &Frame, area: (usize, usize, usize, usize), height: usize) -> (usize, usize, Vec<u8>) {
    let (ax, ay, bx, by) = area;
    let scale = (by - ay) as f32 / height as f32;
    let width = (((bx - ax) as f32 / scale) as usize).max(1);
    let mut out = Vec::with_capacity(width * height * 4);
    for oy in 0..height {
        for ox in 0..width {
            let (x0, y0) = (ax + (ox as f32 * scale) as usize, ay + (oy as f32 * scale) as usize);
            let x1 = (ax + ((ox + 1) as f32 * scale) as usize).clamp(x0 + 1, frame.width);
            let y1 = (ay + ((oy + 1) as f32 * scale) as usize).clamp(y0 + 1, frame.height);
            let (mut sum, mut hits, mut total) = ([0u32; 3], 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    total += 1;
                    if frame.is_cat(x, y) {
                        hits += 1;
                        let [r, g, b] = frame.rgb(x, y);
                        // Despill: green fringe from the screen bleeding into fur.
                        let g = g.min(r.max(b).saturating_add(20));
                        sum = [sum[0] + u32::from(r), sum[1] + u32::from(g), sum[2] + u32::from(b)];
                    }
                }
            }
            if hits == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let avg = sum.map(|c| (c / hits) as u8);
                out.extend_from_slice(&[avg[0], avg[1], avg[2], (hits * 255 / total) as u8]);
            }
        }
    }
    (width, height, out)
}

/// Fit the cat into a `cols` x `rows` grid of `tile_w` x `tile_h` tiles,
/// centered and transparent around it, and return the tiles row by row.
pub fn tiles(
    frame: &Frame,
    area: (usize, usize, usize, usize),
    cols: usize,
    rows: usize,
    tile_w: usize,
    tile_h: usize,
) -> Vec<crate::font::Image> {
    let (canvas_w, canvas_h) = (cols * tile_w, rows * tile_h);
    let aspect = (area.2 - area.0) as f32 / (area.3 - area.1) as f32;
    let height = canvas_h.min((canvas_w as f32 / aspect) as usize).max(1);
    let (cat_w, cat_h, cat) = rgba(frame, area, height);
    let (left, top) = (canvas_w.saturating_sub(cat_w) / 2, canvas_h.saturating_sub(cat_h) / 2);

    let mut canvas = vec![0u8; canvas_w * canvas_h * 4];
    for y in 0..cat_h.min(canvas_h) {
        for x in 0..cat_w.min(canvas_w) {
            let from = (y * cat_w + x) * 4;
            let to = ((top + y) * canvas_w + left + x) * 4;
            canvas[to..to + 4].copy_from_slice(&cat[from..from + 4]);
        }
    }

    let mut tiles = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let mut rgba = Vec::with_capacity(tile_w * tile_h * 4);
            for y in row * tile_h..(row + 1) * tile_h {
                let start = (y * canvas_w + col * tile_w) * 4;
                rgba.extend_from_slice(&canvas[start..start + tile_w * 4]);
            }
            tiles.push(crate::font::Image { width: tile_w, height: tile_h, rgba });
        }
    }
    tiles
}

/// Render one frame in true color with half blocks (two pixels per cell).
/// Green-screen pixels are left as the terminal background.
pub fn halfblock(frame: &Frame, area: (usize, usize, usize, usize), cols: usize, rows: usize) -> String {
    let (ax, ay, bx, by) = area;
    let sx = (bx - ax) as f32 / cols as f32;
    let sy = (by - ay) as f32 / (rows * 2) as f32;
    let sample = |col: usize, half: usize| {
        let x = ax as f32 + (col as f32 + 0.5) * sx;
        let y = ay as f32 + (half as f32 + 0.5) * sy;
        let (x, y) = (x as usize, y as usize);
        frame.is_cat(x, y).then(|| frame.rgb(x, y))
    };
    let mut out = String::new();
    for row in 0..rows {
        for col in 0..cols {
            match (sample(col, row * 2), sample(col, row * 2 + 1)) {
                (Some([r, g, b]), Some([r2, g2, b2])) => {
                    let _ = write!(out, "\x1b[38;2;{r};{g};{b};48;2;{r2};{g2};{b2}m▀\x1b[0m");
                }
                (Some([r, g, b]), None) => {
                    let _ = write!(out, "\x1b[38;2;{r};{g};{b}m▀\x1b[0m");
                }
                (None, Some([r, g, b])) => {
                    let _ = write!(out, "\x1b[38;2;{r};{g};{b}m▄\x1b[0m");
                }
                (None, None) => out.push(' '),
            }
        }
        out.push('\n');
    }
    out
}
