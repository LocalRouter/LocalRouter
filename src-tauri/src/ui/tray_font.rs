//! Tiny bitmap font for tray icon labels and numbers.
//!
//! The tray icon has no text renderer (every glyph in `tray_graph.rs` is a
//! hand-plotted pixel block), so labels and numbers use this fixed 5×7
//! uppercase font, scaled with nearest-neighbour sampling and drawn bold
//! (1px horizontal dilation) so it survives the menu bar's downscale.
//!
//! Stacked labels are sized to use the icon height: 4 letters get 8px
//! glyphs, 3 letters 11px, 1–2 letters 16px. Labels drawn above content use
//! an 11px glyph.

use image::{Rgba, RgbaImage};

/// Base glyph width in the 5×7 source font.
const BASE_WIDTH: u32 = 5;
/// Base glyph height in the 5×7 source font.
const BASE_HEIGHT: u32 = 7;
/// Largest glyph height used for stacked labels (1–2 letters).
const MAX_LABEL_GLYPH_HEIGHT: u32 = 16;
/// Glyph height for a label drawn above its content.
pub const ABOVE_LABEL_GLYPH_HEIGHT: u32 = 11;
/// Icon height the vertical layout is computed against.
const ICON_HEIGHT: u32 = crate::ui::tray_graph::PANE_SIZE;

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
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        '$' => [
            0b00100, 0b01111, 0b10100, 0b01110, 0b00101, 0b11110, 0b00100,
        ],
        '%' => [
            0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '¢' => [
            0b00100, 0b01110, 0b10100, 0b10100, 0b10100, 0b01110, 0b00100,
        ],
        _ => return None,
    })
}

/// Pixel size of a scaled, bold glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontMetrics {
    /// Drawn glyph width including the 1px bold dilation.
    pub glyph_width: u32,
    pub glyph_height: u32,
}

impl FontMetrics {
    /// Metrics for a glyph `height` pixels tall (width scales proportionally).
    pub fn for_height(height: u32) -> Self {
        let height = height.max(BASE_HEIGHT);
        let scaled_w = (BASE_WIDTH as f32 * height as f32 / BASE_HEIGHT as f32).ceil() as u32;
        Self {
            glyph_width: scaled_w + 1,
            glyph_height: height,
        }
    }

    /// Horizontal pitch between characters (glyph + 1px spacing).
    pub fn advance(&self) -> u32 {
        self.glyph_width + 1
    }

    /// Width of `text` drawn horizontally.
    pub fn text_width(&self, text: &str) -> u32 {
        let n = text.chars().count() as u32;
        if n == 0 {
            0
        } else {
            n * self.advance() - 1
        }
    }
}

/// Draw one glyph with its top-left corner at `(x, y)`, scaled to `m` and
/// drawn bold. Characters without a glyph draw nothing. Pixels outside the
/// image are skipped.
pub fn draw_glyph(img: &mut RgbaImage, x: u32, y: u32, ch: char, m: FontMetrics, color: Rgba<u8>) {
    let Some(rows) = glyph(ch.to_ascii_uppercase()) else {
        return;
    };
    let base_w = m.glyph_width - 1;
    for ty in 0..m.glyph_height {
        let sy = (ty * BASE_HEIGHT / m.glyph_height).min(BASE_HEIGHT - 1) as usize;
        for tx in 0..base_w {
            let sx = (tx * BASE_WIDTH / base_w).min(BASE_WIDTH - 1);
            if rows[sy] & (1 << (BASE_WIDTH - 1 - sx)) != 0 {
                for dx in 0..=1 {
                    let px = x + tx + dx;
                    let py = y + ty;
                    if px < img.width() && py < img.height() {
                        img.put_pixel(px, py, color);
                    }
                }
            }
        }
    }
}

/// Draw `text` horizontally with its top-left corner at `(x, y)`.
pub fn draw_text(img: &mut RgbaImage, x: u32, y: u32, text: &str, m: FontMetrics, color: Rgba<u8>) {
    for (i, ch) in text.chars().enumerate() {
        draw_glyph(img, x + i as u32 * m.advance(), y, ch, m, color);
    }
}

/// Metrics for a label drawn horizontally above its content.
pub fn above_label_metrics() -> FontMetrics {
    FontMetrics::for_height(ABOVE_LABEL_GLYPH_HEIGHT)
}

/// Layout of a vertically stacked label: glyph metrics, row pitch and the
/// top offset that centres the stack in the icon height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerticalLayout {
    pub metrics: FontMetrics,
    pub row_pitch: u32,
    pub y_offset: u32,
}

/// Layout for a stacked label of `n` characters (1–4): the glyph grows as
/// the label gets shorter so the whole icon height is used.
pub fn vertical_layout(n: usize) -> VerticalLayout {
    let n = n.clamp(1, 4) as u32;
    let row_pitch = ICON_HEIGHT / n;
    let glyph_height = (row_pitch - 1).min(MAX_LABEL_GLYPH_HEIGHT);
    let metrics = FontMetrics::for_height(glyph_height);
    let y_offset = (ICON_HEIGHT - n * row_pitch) / 2;
    VerticalLayout {
        metrics,
        row_pitch,
        y_offset,
    }
}

/// Width of a stacked label column for `label`.
pub fn vertical_label_width(label: &str) -> u32 {
    if label.is_empty() {
        0
    } else {
        vertical_layout(label.chars().count()).metrics.glyph_width
    }
}

/// Draw `label` stacked vertically (letters upright, top to bottom) in a
/// column `column_width` wide whose left edge is at `x`; glyphs are centred
/// horizontally in the column and the stack is centred vertically.
pub fn draw_vertical_label(
    img: &mut RgbaImage,
    x: u32,
    column_width: u32,
    label: &str,
    color: Rgba<u8>,
) {
    let chars: Vec<char> = label.chars().take(4).collect();
    if chars.is_empty() {
        return;
    }
    let layout = vertical_layout(chars.len());
    let gx = x + column_width.saturating_sub(layout.metrics.glyph_width) / 2;
    let cell_pad = (layout.row_pitch - layout.metrics.glyph_height) / 2;
    for (i, ch) in chars.iter().enumerate() {
        let gy = layout.y_offset + i as u32 * layout.row_pitch + cell_pad;
        draw_glyph(img, gx, gy, *ch, layout.metrics, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_set(img: &RgbaImage) -> usize {
        img.pixels().filter(|p| p[3] != 0).count()
    }

    #[test]
    fn every_needed_char_has_a_glyph() {
        for ch in ('A'..='Z')
            .chain('0'..='9')
            .chain(['.', '$', '%', '-', '¢'])
        {
            assert!(glyph(ch).is_some(), "missing glyph for {ch}");
        }
        assert!(glyph(' ').is_none());
    }

    #[test]
    fn base_metrics_and_bold_dilation() {
        let m = FontMetrics::for_height(7);
        assert_eq!(m.glyph_width, 6); // 5 + 1px bold
        assert_eq!(m.advance(), 7);
        assert_eq!(m.text_width("24K"), 20);
        assert_eq!(m.text_width(""), 0);
        let mut img = RgbaImage::from_pixel(6, 7, Rgba([0, 0, 0, 0]));
        draw_glyph(&mut img, 0, 0, 'I', m, Rgba([0, 0, 0, 255]));
        // 'I' has 11 set source bits; dilation adds one pixel right of each run end
        assert!(count_set(&img) > 11);
        // Lowercase maps to the same glyph
        let mut img2 = RgbaImage::from_pixel(6, 7, Rgba([0, 0, 0, 0]));
        draw_glyph(&mut img2, 0, 0, 'i', m, Rgba([0, 0, 0, 255]));
        assert_eq!(img.as_raw(), img2.as_raw());
    }

    #[test]
    fn scaled_glyph_fills_its_box() {
        let m = FontMetrics::for_height(14);
        assert_eq!(m.glyph_height, 14);
        assert_eq!(m.glyph_width, 11);
        let mut img = RgbaImage::from_pixel(m.glyph_width, 14, Rgba([0, 0, 0, 0]));
        draw_glyph(&mut img, 0, 0, 'H', m, Rgba([0, 0, 0, 255]));
        // Top row and bottom row both have ink (H spans full height)
        assert!((0..m.glyph_width).any(|x| img.get_pixel(x, 0)[3] != 0));
        assert!((0..m.glyph_width).any(|x| img.get_pixel(x, 13)[3] != 0));
    }

    #[test]
    fn vertical_layout_uses_full_height() {
        let l4 = vertical_layout(4);
        assert_eq!(
            (l4.row_pitch, l4.metrics.glyph_height, l4.y_offset),
            (9, 8, 0)
        );
        let l3 = vertical_layout(3);
        assert_eq!(
            (l3.row_pitch, l3.metrics.glyph_height, l3.y_offset),
            (12, 11, 0)
        );
        let l2 = vertical_layout(2);
        assert_eq!((l2.row_pitch, l2.metrics.glyph_height), (18, 16));
        let l1 = vertical_layout(1);
        assert_eq!(l1.metrics.glyph_height, 16);
        // Wider glyphs for shorter labels
        assert!(vertical_label_width("ALL") > vertical_label_width("CLAU"));
        assert_eq!(vertical_label_width(""), 0);
        assert_eq!(above_label_metrics().glyph_height, ABOVE_LABEL_GLYPH_HEIGHT);
    }

    #[test]
    fn vertical_label_draws_each_row() {
        let w = vertical_label_width("ILLI");
        let mut img = RgbaImage::from_pixel(w, 36, Rgba([0, 0, 0, 0]));
        draw_vertical_label(&mut img, 0, w, "ILLI", Rgba([0, 0, 0, 255]));
        for row in 0..4u32 {
            let y0 = row * 9;
            assert!(
                (y0..y0 + 8).any(|y| (0..w).any(|x| img.get_pixel(x, y)[3] != 0)),
                "row {row} empty"
            );
        }
        // Gap rows between glyphs stay empty
        assert!((0..w).all(|x| img.get_pixel(x, 8)[3] == 0));
        // 3-letter label: bigger glyphs, rows at 0..11, 12..23, 24..35
        let w3 = vertical_label_width("ALL");
        let mut img3 = RgbaImage::from_pixel(w3, 36, Rgba([0, 0, 0, 0]));
        draw_vertical_label(&mut img3, 0, w3, "ALL", Rgba([0, 0, 0, 255]));
        assert!((0..w3).any(|x| img3.get_pixel(x, 0)[3] != 0));
        assert!((0..w3).all(|x| img3.get_pixel(x, 11)[3] == 0));
        assert!((0..w3).any(|x| img3.get_pixel(x, 34)[3] != 0));
        // A short label is centred in a wider column
        let mut imgc = RgbaImage::from_pixel(w3 + 4, 36, Rgba([0, 0, 0, 0]));
        draw_vertical_label(&mut imgc, 0, w3 + 4, "ALL", Rgba([0, 0, 0, 255]));
        assert!((0..36).all(|y| imgc.get_pixel(0, y)[3] == 0));
    }

    #[test]
    fn out_of_bounds_pixels_are_clipped() {
        let mut img = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 0]));
        draw_glyph(
            &mut img,
            1,
            1,
            'M',
            FontMetrics::for_height(14),
            Rgba([0, 0, 0, 255]),
        );
        assert!(count_set(&img) <= 4);
    }
}
