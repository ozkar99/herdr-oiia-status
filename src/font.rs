//! Builds a color bitmap font whose glyphs are the frames of the OIIA GIF.
//!
//! Herdr's sidebar is text, but the terminal decides how each character looks.
//! Mapping a few private-use code points to this font (Ghostty
//! `font-codepoint-map`) turns them into the actual cat.
//!
//! Linux (FreeType) reads Google's CBDT/CBLC color bitmaps, the format of Noto
//! Color Emoji; macOS (CoreText) only reads Apple's `sbix`. Each platform gets
//! the format it understands.

use std::io;

/// First private-use code point; glyph `n` is `FIRST_CODEPOINT + n`. Plane 16
/// (Supplementary Private Use Area-B): plane 15 is full of Nerd Font icons,
/// which terminals like Ghostty bundle and would draw instead of the cat.
pub const FIRST_CODEPOINT: u32 = 0x10_0000;
pub const FAMILY: &str = "OIIA Cat";

/// Glyph bitmap size: about the shape of one terminal cell, so a terminal
/// scales it to the cell without cropping.
pub const TILE_WIDTH: usize = 60;
pub const TILE_HEIGHT: usize = 128;

/// Bitmap strike size, like Noto Color Emoji. Terminals scale it to the cell.
const PPEM: u8 = 109;
const UNITS_PER_EM: u16 = 2048;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// CBDT/CBLC bitmaps, for FreeType (Linux).
    Cbdt,
    /// `sbix` bitmaps plus empty outlines, for CoreText (macOS).
    Sbix,
}

impl Format {
    pub fn native() -> Self {
        if cfg!(target_os = "macos") { Self::Sbix } else { Self::Cbdt }
    }
}

pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub fn build(frames: &[Image], format: Format) -> io::Result<Vec<u8>> {
    let num_glyphs = frames.len() as u16 + 1; // glyph 0 is .notdef
    let pngs = frames.iter().map(encode_png).collect::<io::Result<Vec<_>>>()?;
    let max_w = frames.iter().map(|f| f.width).max().unwrap_or(1) as u8;
    let max_h = frames.iter().map(|f| f.height).max().unwrap_or(1) as u8;
    // Bitmap metrics are in pixels at PPEM; font metrics in font units.
    let to_units = |px: u8| (u32::from(px) * u32::from(UNITS_PER_EM) / u32::from(PPEM)) as i16;
    let ascent_px = (u16::from(max_h) * 4 / 5) as u8;
    let descent_px = max_h - ascent_px;
    let ascent = to_units(ascent_px);
    let descent = -to_units(descent_px);
    let advance = to_units(max_w) as u16;

    // CBDT: header, then per glyph: smallGlyphMetrics + length + PNG (format 17).
    let mut cbdt = be(&[3u16, 0]);
    let mut offsets = vec![4u32]; // relative to the CBDT start (imageDataOffset = 0)
    for (png, frame) in pngs.iter().zip(frames) {
        cbdt.extend_from_slice(&[
            frame.height as u8,
            frame.width as u8,
            ((max_w as usize - frame.width) / 2) as u8, // bearingX: center
            ascent_px,                                  // bearingY
            max_w,                                      // advance
        ]);
        cbdt.extend_from_slice(&(png.len() as u32).to_be_bytes());
        cbdt.extend_from_slice(png);
        offsets.push(cbdt.len() as u32);
    }

    // CBLC: one strike covering glyphs 1..=n with one format-1 index subtable.
    let first = 1u16;
    let last = num_glyphs - 1;
    let line_metrics = |out: &mut Vec<u8>| {
        out.extend_from_slice(&[ascent_px, (descent_px as i8).wrapping_neg() as u8, max_w]);
        out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    };
    let mut cblc = Vec::new();
    cblc.extend(be(&[3u16, 0]));
    cblc.extend(1u32.to_be_bytes()); // numSizes
    let array_offset = 8 + 48u32;
    let subtable = {
        let mut s = be(&[1u16, 17]); // indexFormat 1, imageFormat 17
        s.extend(0u32.to_be_bytes()); // imageDataOffset
        for offset in &offsets {
            s.extend(offset.to_be_bytes());
        }
        s
    };
    let index_tables_size = 8 + subtable.len() as u32;
    cblc.extend(array_offset.to_be_bytes());
    cblc.extend(index_tables_size.to_be_bytes());
    cblc.extend(1u32.to_be_bytes()); // numberOfIndexSubTables
    cblc.extend(0u32.to_be_bytes()); // colorRef
    line_metrics(&mut cblc); // hori
    line_metrics(&mut cblc); // vert
    cblc.extend(be(&[first, last]));
    cblc.extend_from_slice(&[PPEM, PPEM, 32, 1]); // ppemX, ppemY, bitDepth, flags
    cblc.extend(be(&[first, last]));
    cblc.extend(8u32.to_be_bytes()); // additionalOffsetToIndexSubtable
    cblc.extend(subtable);

    // cmap: format 12, one group mapping the code points to glyphs 1..=n.
    let mut format12 = be(&[12u16, 0]);
    format12.extend((16u32 + 12).to_be_bytes()); // length
    format12.extend(0u32.to_be_bytes()); // language
    format12.extend(1u32.to_be_bytes()); // numGroups
    format12.extend(FIRST_CODEPOINT.to_be_bytes());
    format12.extend((FIRST_CODEPOINT + u32::from(last) - 1).to_be_bytes());
    format12.extend(1u32.to_be_bytes());
    let mut cmap = be(&[0u16, 2, 0, 4]);
    cmap.extend((4u32 + 16).to_be_bytes());
    cmap.extend(be(&[3u16, 10]));
    cmap.extend((4u32 + 16).to_be_bytes());
    cmap.extend(format12);

    let mut head = be(&[1u16, 0, 1, 0]); // version 1.0, fontRevision 1.0
    head.extend(0u32.to_be_bytes()); // checkSumAdjustment, patched below
    head.extend(0x5F0F_3CF5u32.to_be_bytes());
    head.extend(be(&[0b1011, UNITS_PER_EM]));
    head.extend([0u8; 16]); // created, modified
    head.extend(be_i(&[0, descent, advance as i16, ascent]));
    head.extend(be(&[0, 8, 2]));
    head.extend(be_i(&[0, 0])); // indexToLocFormat, glyphDataFormat

    let mut hhea = be(&[1u16, 0]);
    hhea.extend(be_i(&[ascent, descent, 0]));
    hhea.extend(be(&[advance]));
    hhea.extend(be_i(&[0, 0, advance as i16, 1, 0, 0, 0, 0, 0, 0, 0]));
    hhea.extend(be(&[num_glyphs]));

    let mut hmtx = Vec::new();
    for _ in 0..num_glyphs {
        hmtx.extend(be(&[advance, 0]));
    }

    // maxp 0.5 suffices for a bitmap-only font; with outlines it must be 1.0.
    let mut maxp = match format {
        Format::Cbdt => 0x0000_5000u32,
        Format::Sbix => 0x0001_0000u32,
    }
    .to_be_bytes()
    .to_vec();
    maxp.extend(be(&[num_glyphs]));
    if format == Format::Sbix {
        maxp.extend(be(&[0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]));
    }

    let mut os2 = be(&[4u16]);
    os2.extend(be_i(&[advance as i16]));
    os2.extend(be(&[400, 5, 0]));
    os2.extend(be_i(&[0; 11]));
    os2.extend([0u8; 10]); // panose
    os2.extend([0u8; 16]); // unicode ranges
    os2.extend(b"NONE");
    os2.extend(be(&[0x40, 0xFFFF, 0xFFFF]));
    os2.extend(be_i(&[ascent, descent, 0]));
    os2.extend(be(&[ascent as u16, (-descent) as u16]));
    os2.extend([0u8; 8]); // code page ranges
    os2.extend(be_i(&[0, ascent, 0, 0, 0]));

    let mut post = 0x0003_0000u32.to_be_bytes().to_vec();
    post.extend([0u8; 28]);

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp),
        (b"name", name_table()),
        (b"post", post),
    ];
    match format {
        Format::Cbdt => {
            tables.push((b"CBDT", cbdt));
            tables.push((b"CBLC", cblc));
        }
        Format::Sbix => {
            let sbix = sbix_table(&pngs, frames, max_w, descent_px);
            tables.push((b"sbix", sbix));
            // CoreText wants outlines to exist; every glyph's is empty.
            tables.push((b"glyf", vec![0; 4]));
            tables.push((b"loca", vec![0; 2 * (usize::from(num_glyphs) + 1)]));
        }
    }
    // The table directory must be sorted by tag.
    tables.sort_by_key(|(tag, _)| **tag);
    Ok(assemble(tables))
}

/// `sbix` with one strike at PPEM: per glyph an origin offset, a `png ` tag
/// and the PNG. Glyph 0 (.notdef) has no image.
fn sbix_table(pngs: &[Vec<u8>], frames: &[Image], max_w: u8, descent_px: u8) -> Vec<u8> {
    let num_glyphs = pngs.len() + 1;
    let mut table = be(&[1, 1]); // version 1, flags: bit 0 must be set
    table.extend(1u32.to_be_bytes()); // numStrikes
    table.extend(12u32.to_be_bytes()); // offset of the strike from the table start

    let header = 4 + 4 * (num_glyphs + 1);
    let mut data = Vec::new();
    let mut offsets = vec![header as u32, header as u32]; // glyph 0 is empty
    for (png, frame) in pngs.iter().zip(frames) {
        data.extend((((usize::from(max_w) - frame.width) / 2) as i16).to_be_bytes());
        data.extend((-(i16::from(descent_px))).to_be_bytes());
        data.extend_from_slice(b"png ");
        data.extend_from_slice(png);
        offsets.push((header + data.len()) as u32);
    }
    table.extend(be(&[u16::from(PPEM), 72]));
    for offset in offsets {
        table.extend(offset.to_be_bytes());
    }
    table.extend(data);
    table
}

fn name_table() -> Vec<u8> {
    let records = [(1u16, FAMILY), (2, "Regular"), (4, FAMILY), (6, "OIIACat-Regular")];
    let encoded: Vec<Vec<u8>> = records
        .iter()
        .map(|(_, text)| text.encode_utf16().flat_map(u16::to_be_bytes).collect())
        .collect();
    let mut out = be(&[0, records.len() as u16, 6 + 12 * records.len() as u16]);
    let mut offset = 0u16;
    for ((id, _), bytes) in records.iter().zip(&encoded) {
        out.extend(be(&[3, 1, 0x409, *id, bytes.len() as u16, offset]));
        offset += bytes.len() as u16;
    }
    for bytes in encoded {
        out.extend(bytes);
    }
    out
}

fn assemble(tables: Vec<(&[u8; 4], Vec<u8>)>) -> Vec<u8> {
    let n = tables.len() as u16;
    let pow2 = 1u16 << (15 - n.leading_zeros() as u16);
    let mut font = 0x0001_0000u32.to_be_bytes().to_vec();
    font.extend(be(&[n, pow2 * 16, pow2.trailing_zeros() as u16, n * 16 - pow2 * 16]));
    let mut offset = 12 + 16 * u32::from(n);
    let mut body = Vec::new();
    let mut head_at = 0;
    for (tag, data) in &tables {
        if *tag == b"head" {
            head_at = offset as usize;
        }
        font.extend_from_slice(*tag);
        font.extend(checksum(data).to_be_bytes());
        font.extend(offset.to_be_bytes());
        font.extend((data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * u32::from(n) + body.len() as u32;
    }
    font.extend(body);
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
    font[head_at + 8..head_at + 12].copy_from_slice(&adjustment.to_be_bytes());
    font
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn encode_png(image: &Image) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width as u32, image.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&image.rgba).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    Ok(out)
}

fn be(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn be_i(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(font: &[u8]) -> Vec<[u8; 4]> {
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        (0..count)
            .map(|i| font[12 + 16 * i..16 + 16 * i].try_into().unwrap())
            .collect()
    }

    #[test]
    fn builds_both_formats_with_sorted_tables() {
        let image = Image { width: TILE_WIDTH, height: TILE_HEIGHT, rgba: vec![200; TILE_WIDTH * TILE_HEIGHT * 4] };
        for (format, must_have) in [(Format::Cbdt, *b"CBDT"), (Format::Sbix, *b"sbix")] {
            let font = build(&[Image { rgba: image.rgba.clone(), ..image }], format).unwrap();
            let tags = tags(&font);
            assert!(tags.contains(&must_have));
            assert!(tags.windows(2).all(|pair| pair[0] < pair[1]), "table directory must be sorted");
            if let Some(dir) = std::env::var_os("OIIA_FONT_TEST_OUT") {
                let name = if format == Format::Sbix { "sbix.ttf" } else { "cbdt.ttf" };
                std::fs::write(std::path::Path::new(&dir).join(name), &font).unwrap();
            }
        }
    }
}
