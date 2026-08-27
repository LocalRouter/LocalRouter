//! Tiny 5×7 bitmap font for tray icon panel labels.
//!
//! The tray icon has no text renderer (every glyph in `tray_graph.rs` is a
//! hand-plotted pixel block), so panel labels use this fixed uppercase
//! alphanumeric font. Letters are stacked vertically — one glyph per 8px
//! row — so a 4-character label fills the 32px icon height exactly.

use image::{Rgba, RgbaImage};

/// Glyph width in pixels.
pub const GLYPH_WIDTH: u32 = 5;
/// Glyph height in pixels.
pub const GLYPH_HEIGHT: u32 = 7;
/// Vertical pitch between stacked glyphs (7px glyph + 1px gap).
pub const ROW_PITCH: u32 = GLYPH_HEIGHT + 1;

/// 5×7 glyphs, one `u8` per row (top to bottom), bit 4 = leftmost pixel.
const fn glyph(ch: char) -> Option<[u8; 7]> {
    Some(match ch {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        _ => return None,
    })
}

/// Draw one glyph with its top-left corner at `(x, y)`. Characters without
/// a glyph (anything outside `A-Z0-9`, after uppercasing) draw nothing.
/// Pixels outside the image are skipped.
pub fn draw_glyph(img: &mut RgbaImage, x: u32, y: u32, ch: char, color: Rgba<u8>) {
    let Some(rows) = glyph(ch.to_ascii_uppercase()) else {
        return;
    };
    for (dy, row) in rows.iter().enumerate() {
        for dx in 0..GLYPH_WIDTH {
            if row & (1 << (GLYPH_WIDTH - 1 - dx)) != 0 {
                let px = x + dx;
                let py = y + dy as u32;
                if px < img.width() && py < img.height() {
                    img.put_pixel(px, py, color);
                }
            }
        }
    }
}

/// Draw `label` stacked vertically (letters upright, top to bottom) with
/// the column's left edge at `x`, starting at the top of the image. Only
/// as many characters as fit in the image height are drawn.
pub fn draw_vertical_label(img: &mut RgbaImage, x: u32, label: &str, color: Rgba<u8>) {
    let max_rows = (img.height() / ROW_PITCH) as usize;
    for (i, ch) in label.chars().take(max_rows).enumerate() {
        draw_glyph(img, x, i as u32 * ROW_PITCH, ch, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_set(img: &RgbaImage) -> usize {
        img.pixels().filter(|p| p[3] != 0).count()
    }

    #[test]
    fn every_alphanumeric_has_a_glyph() {
        for ch in ('A'..='Z').chain('0'..='9') {
            assert!(glyph(ch).is_some(), "missing glyph for {ch}");
        }
        assert!(glyph('-').is_none());
        assert!(glyph(' ').is_none());
    }

    #[test]
    fn glyph_draws_within_5x7_box() {
        let mut img = RgbaImage::from_pixel(5, 7, Rgba([0, 0, 0, 0]));
        draw_glyph(&mut img, 0, 0, 'A', Rgba([0, 0, 0, 255]));
        // 'A' from the table has 18 set bits (3+2+2+5+2+2+2)
        assert_eq!(count_set(&img), 18);
        // Lowercase maps to the same glyph
        let mut img2 = RgbaImage::from_pixel(5, 7, Rgba([0, 0, 0, 0]));
        draw_glyph(&mut img2, 0, 0, 'a', Rgba([0, 0, 0, 255]));
        assert_eq!(img.as_raw(), img2.as_raw());
    }

    #[test]
    fn vertical_label_stacks_four_rows_in_32px() {
        let mut img = RgbaImage::from_pixel(6, 32, Rgba([0, 0, 0, 0]));
        draw_vertical_label(&mut img, 0, "ILLI", Rgba([0, 0, 0, 255]));
        // 'I' has 11 set bits, 'L' has 11 → 44 total across 4 rows
        assert_eq!(count_set(&img), 44);
        // Fourth glyph occupies rows 24..31
        assert!((24..31).any(|y| img.get_pixel(2, y)[3] != 0));
        // A fifth character does not fit and is ignored
        let mut img5 = RgbaImage::from_pixel(6, 32, Rgba([0, 0, 0, 0]));
        draw_vertical_label(&mut img5, 0, "IIIII", Rgba([0, 0, 0, 255]));
        assert_eq!(count_set(&img5), 44);
    }

    #[test]
    fn out_of_bounds_pixels_are_clipped() {
        let mut img = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 0]));
        draw_glyph(&mut img, 1, 1, 'M', Rgba([0, 0, 0, 255]));
        // no panic; only the visible 2×2 corner was drawn
        assert!(count_set(&img) <= 4);
    }
}
