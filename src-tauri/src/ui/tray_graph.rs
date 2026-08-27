//! Tray icon graph generation
//!
//! Generates 32x32 PNG sparkline graphs showing token usage over time.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use image::{codecs::png::PngEncoder, ImageEncoder, Rgba, RgbaImage};
use std::io::Cursor;
use tracing::error;

use lr_providers::health_cache::AggregateHealthStatus;

/// Overlay icon to render in the top-left corner of the tray graph
#[derive(Debug, Clone, PartialEq)]
pub enum TrayOverlay {
    /// No overlay — normal rounded rect corner
    None,
    /// Exclamation mark in the given status color (for warning/error health)
    Warning(Rgba<u8>),
    /// Down-arrow in foreground color (update available)
    UpdateAvailable,
    /// Question mark in red (firewall approval pending)
    FirewallPending,
}

/// Data point for graph rendering
#[derive(Debug, Clone)]
pub struct DataPoint {
    /// Timestamp of this data point
    pub timestamp: DateTime<Utc>,
    /// Total tokens (input + output)
    pub total_tokens: u64,
}

/// Graph rendering configuration
#[derive(Debug, Clone)]
pub struct GraphConfig {
    /// Foreground color (graph line/bars)
    pub foreground: Rgba<u8>,
    /// Background color (can be transparent)
    pub background: Rgba<u8>,
    /// Whether this is a template icon (macOS adaptive)
    pub template_mode: bool,
}

impl GraphConfig {
    /// Create config for macOS — template image (black on transparent).
    ///
    /// macOS recolors template images automatically for the current menu
    /// bar appearance (light/dark/translucent/hover). RGB values are
    /// ignored by the renderer; only the alpha channel matters. Colored
    /// status overlays therefore get flattened to the menu-bar tint —
    /// the overlay shape (exclamation / question / down-arrow) is what
    /// distinguishes them.
    pub fn macos(_dark_mode: bool) -> Self {
        Self {
            foreground: Rgba([0, 0, 0, 255]), // Black; macOS recolors at draw time
            background: Rgba([0, 0, 0, 0]),   // Transparent
            template_mode: true,
        }
    }

    /// Create config for Windows/Linux (fixed color)
    pub fn windows_linux() -> Self {
        Self {
            foreground: Rgba([0, 120, 215, 255]),   // Blue
            background: Rgba([240, 240, 240, 255]), // Light gray
            template_mode: false,
        }
    }

    /// Legacy: Create config for macOS template mode (not used anymore)
    #[allow(dead_code)]
    pub fn macos_template() -> Self {
        Self {
            foreground: Rgba([255, 255, 255, 255]), // White (inverted by macOS)
            background: Rgba([0, 0, 0, 0]),         // Transparent
            template_mode: true,
        }
    }
}

/// Status dot colors with dark/light mode variants
pub struct StatusDotColors;

impl StatusDotColors {
    /// Green color for healthy status
    /// Light mode: #22c55e (darker green for contrast on light backgrounds)
    /// Dark mode: #4ade80 (brighter green for visibility on dark backgrounds)
    pub fn green(dark_mode: bool) -> Rgba<u8> {
        if dark_mode {
            Rgba([74, 222, 128, 255]) // #4ade80 - green-400
        } else {
            Rgba([34, 197, 94, 255]) // #22c55e - green-500
        }
    }

    /// Yellow color for degraded/warning status
    /// Light mode: #eab308 (darker yellow for contrast on light backgrounds)
    /// Dark mode: #facc15 (brighter yellow for visibility on dark backgrounds)
    pub fn yellow(dark_mode: bool) -> Rgba<u8> {
        if dark_mode {
            Rgba([250, 204, 21, 255]) // #facc15 - yellow-400
        } else {
            Rgba([234, 179, 8, 255]) // #eab308 - yellow-500
        }
    }

    /// Red color for unhealthy/down status
    /// Light mode: #ef4444 (standard red for contrast on light backgrounds)
    /// Dark mode: #f87171 (brighter red for visibility on dark backgrounds)
    pub fn red(dark_mode: bool) -> Rgba<u8> {
        if dark_mode {
            Rgba([248, 113, 113, 255]) // #f87171 - red-400
        } else {
            Rgba([239, 68, 68, 255]) // #ef4444 - red-500
        }
    }

    /// Get color for aggregate health status
    pub fn for_status(status: AggregateHealthStatus, dark_mode: bool) -> Rgba<u8> {
        match status {
            AggregateHealthStatus::Green => Self::green(dark_mode),
            AggregateHealthStatus::Yellow => Self::yellow(dark_mode),
            AggregateHealthStatus::Red => Self::red(dark_mode),
        }
    }
}

/// Draw a filled circle (status dot) on the image
///
/// # Arguments
/// * `img` - The image to draw on
/// * `center_x` - X coordinate of the center
/// * `center_y` - Y coordinate of the center
/// * `radius` - Radius of the circle
/// * `color` - Fill color
fn draw_filled_circle(
    img: &mut RgbaImage,
    center_x: i32,
    center_y: i32,
    radius: i32,
    color: Rgba<u8>,
) {
    let width = img.width() as i32;
    let height = img.height() as i32;

    for y in (center_y - radius)..=(center_y + radius) {
        for x in (center_x - radius)..=(center_x + radius) {
            // Check if within image bounds
            if x >= 0 && x < width && y >= 0 && y < height {
                // Check if within circle using distance formula
                let dx = x - center_x;
                let dy = y - center_y;
                if dx * dx + dy * dy <= radius * radius {
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
}

/// Draw a hollow circle (ring) on the image
fn draw_hollow_circle(
    img: &mut RgbaImage,
    center_x: i32,
    center_y: i32,
    outer_radius: i32,
    thickness: i32,
    color: Rgba<u8>,
) {
    let inner_radius = outer_radius - thickness;
    let inner_radius_sq = inner_radius * inner_radius;
    let outer_radius_sq = outer_radius * outer_radius;
    let width = img.width() as i32;
    let height = img.height() as i32;

    for y in (center_y - outer_radius)..=(center_y + outer_radius) {
        for x in (center_x - outer_radius)..=(center_x + outer_radius) {
            if x >= 0 && x < width && y >= 0 && y < height {
                let dx = x - center_x;
                let dy = y - center_y;
                let dist_sq = dx * dx + dy * dy;
                // Draw if within the ring (between inner and outer radius)
                if dist_sq <= outer_radius_sq && dist_sq >= inner_radius_sq {
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
}

/// Draw a bold exclamation mark in the top-left corner cutout area
///
/// The exclamation mark has a 4px-wide stem and a 4x4 dot below it.
/// Total extent: x=4..7, y=0..12 — center at roughly (6, 6).
fn draw_exclamation_mark(img: &mut RgbaImage, color: Rgba<u8>) {
    // Stem: 4px wide (x=5,6,7,8), from y=0 to y=6
    for y in 0u32..=6 {
        for x in 5u32..=8 {
            img.put_pixel(x, y, color);
        }
    }

    // Dot: 4x4 block matching stem width at y=9..12 (round appearance)
    for y in 9u32..=12 {
        for x in 5u32..=8 {
            img.put_pixel(x, y, color);
        }
    }
}

/// Draw a down-arrow in the top-left corner cutout area
///
/// Downward-pointing arrow for "update available" indicator.
/// Sized to fill roughly 1/3 of the 32x32 icon (~11px tall, ~9px wide).
fn draw_down_arrow(img: &mut RgbaImage, color: Rgba<u8>) {
    // Vertical stem: 2px wide (x=6..7), from y=1 to y=6
    for y in 1u32..=6 {
        img.put_pixel(6, y, color);
        img.put_pixel(7, y, color);
    }

    // Arrow head: widening chevron pointing down
    // Row y=7: x=3..10 (8px wide)
    for x in 3u32..=10 {
        img.put_pixel(x, 7, color);
    }
    // Row y=8: x=4..9 (6px wide)
    for x in 4u32..=9 {
        img.put_pixel(x, 8, color);
    }
    // Row y=9: x=5..8 (4px wide)
    for x in 5u32..=8 {
        img.put_pixel(x, 9, color);
    }
    // Row y=10: x=6..7 (2px wide)
    img.put_pixel(6, 10, color);
    img.put_pixel(7, 10, color);
}

/// Draw a bold question mark in the top-left corner cutout area
///
/// Used for the firewall pending approval indicator.
/// Bold question mark with 2-3px thick strokes, matching exclamation mark weight.
fn draw_question_mark(img: &mut RgbaImage, color: Rgba<u8>) {
    // Bold top curve of ? with 2px thick lines
    // Top arc: rows y=0-1 (top of curve)
    for x in 5u32..=8 {
        img.put_pixel(x, 0, color);
    }
    for x in 4u32..=9 {
        img.put_pixel(x, 1, color);
    }
    // Sides of curve: y=2-3
    // Left side (outer)
    img.put_pixel(3, 2, color);
    img.put_pixel(4, 2, color);
    img.put_pixel(3, 3, color);
    img.put_pixel(4, 3, color);
    // Right side (outer)
    img.put_pixel(9, 2, color);
    img.put_pixel(10, 2, color);
    img.put_pixel(9, 3, color);
    img.put_pixel(10, 3, color);

    // Curving down on right side: y=4-5
    img.put_pixel(8, 4, color);
    img.put_pixel(9, 4, color);
    img.put_pixel(7, 5, color);
    img.put_pixel(8, 5, color);

    // Vertical stem: x=5-7, y=6-7 (bold 3px wide)
    for y in 6u32..=7 {
        for x in 5u32..=7 {
            img.put_pixel(x, y, color);
        }
    }

    // Dot: 4x3 block at y=10-12 (gap at y=8-9)
    for y in 10u32..=12 {
        for x in 5u32..=8 {
            img.put_pixel(x, y, color);
        }
    }
}

/// Draw a thick line between two points using Bresenham's algorithm with thickness
fn draw_thick_line(
    img: &mut RgbaImage,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: Rgba<u8>,
) {
    let width = img.width() as i32;
    let height = img.height() as i32;
    let half_t = thickness / 2;

    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx - dy;

    let mut x = x0;
    let mut y = y0;

    loop {
        // Draw a filled circle at each point for thickness
        for ty in -half_t..=half_t {
            for tx in -half_t..=half_t {
                if tx * tx + ty * ty <= half_t * half_t {
                    let px = x + tx;
                    let py = y + ty;
                    if px >= 0 && px < width && py >= 0 && py < height {
                        img.put_pixel(px as u32, py as u32, color);
                    }
                }
            }
        }

        if x == x1 && y == y1 {
            break;
        }

        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
}

/// Draw the LocalRouter logo (two circles connected by S-curve)
///
/// Draws two hollow circles at opposite corners connected by a wavy routing line.
/// The logo is drawn with low opacity so the graph bars can be seen through it.
fn draw_logo(img: &mut RgbaImage, base_color: Rgba<u8>) {
    // Use the base color but with very low alpha for transparency
    let color = Rgba([base_color[0], base_color[1], base_color[2], 60]); // ~24% opacity

    // Logo fits in the graph area (approximately 3-28 in both dimensions)
    // Scale from 100x100 viewBox to ~26x26 pixel area
    // Top-left circle: originally at (20, 20) with r=12 → scaled to (8, 8) with r=4
    // Bottom-right circle: originally at (80, 80) with r=12 → scaled to (24, 24) with r=4

    // Draw top-left hollow circle
    draw_hollow_circle(img, 8, 8, 5, 2, color);

    // Draw bottom-right hollow circle
    draw_hollow_circle(img, 24, 24, 5, 2, color);

    // Draw the S-curve connecting them
    // Original path: M 32 22 C 75 15, 90 40, 50 50 C 10 60, 25 85, 68 78
    // Simplified to a series of line segments approximating the curve
    // Scale factor: 0.26, offset: 3

    // Approximate the bezier curve with line segments
    // Points along the curve (scaled from 100x100 to 32x32 with offset 3):
    let curve_points: [(i32, i32); 9] = [
        (11, 9),  // Start near top-left circle
        (14, 8),  // Curve up-right
        (18, 9),  // Continue right
        (20, 12), // Curve down
        (16, 16), // Center area
        (12, 18), // Curve left
        (10, 21), // Continue down-left
        (14, 24), // Curve right
        (20, 23), // End near bottom-right circle
    ];

    // Draw lines connecting the points
    for i in 0..curve_points.len() - 1 {
        let (x0, y0) = curve_points[i];
        let (x1, y1) = curve_points[i + 1];
        draw_thick_line(img, x0, y0, x1, y1, 3, color);
    }
}

/// Height of the tray icon and side length of a full graph pane.
///
/// 36px maps 1:1 onto Retina device pixels once macOS scales the image to
/// its 18pt menu-bar height (and 2:1 on non-Retina), so 1px strokes stay
/// crisp instead of smearing across pixel boundaries.
pub const PANE_SIZE: u32 = 36;
const BORDER_WIDTH: u32 = 1;
const INNER_MARGIN: u32 = 2; // Additional margin inside border to prevent overlap
/// Bars per graph pane (pane width minus border and margin on both sides).
pub const GRAPH_WIDTH: u32 = PANE_SIZE - 2 * (BORDER_WIDTH + INNER_MARGIN); // 30
const GRAPH_OFFSET_X: u32 = BORDER_WIDTH + INNER_MARGIN; // Start at x=3
const GRAPH_OFFSET_Y: u32 = BORDER_WIDTH + INNER_MARGIN; // Start at y=3
const CORNER_RADIUS: u32 = 6;

// Cutout: quarter-circle notch centered on the overlay icon center (6, 6)
// with radius 9. The arc meets the top edge at x≈13 and the left edge at y≈13.
const CUTOUT_CX: i32 = 6;
const CUTOUT_CY: i32 = 6;
const CUTOUT_R: i32 = 9;
const CUTOUT_R_SQ: i32 = CUTOUT_R * CUTOUT_R; // 81
/// Where the arc intersects the image edges — borders start here
const CUTOUT_SIZE: u32 = 13;

/// Scaling configuration: 1 pixel = 5 tokens while the P95 fits.
pub const TOKENS_PER_PIXEL: u64 = 5;
/// Bar height available in a full-size graph pane.
pub const MAX_BAR_HEIGHT: u32 = PANE_SIZE - 2 * (BORDER_WIDTH + INNER_MARGIN); // 30

/// Gap between panes and between a stacked label and its graph.
pub const PANE_GAP: u32 = 6;
/// Height reserved for a label drawn above content (glyph + spacing).
const LABEL_ROW_HEIGHT: u32 = crate::ui::tray_font::ABOVE_LABEL_GLYPH_HEIGHT + 3;
/// Height of a graph pane drawn under a label (≈ 60 % of full size).
pub const SMALL_GRAPH_HEIGHT: u32 = PANE_SIZE - LABEL_ROW_HEIGHT;
/// Vertical margin around a number inside its box.
const NUMBER_MARGIN: u32 = 3;
/// Number glyph height range: fills the box up to this cap, never below the floor.
const NUMBER_MIN_GLYPH_HEIGHT: u32 = 12;
pub const NUMBER_MAX_GLYPH_HEIGHT: u32 = 16;
/// Horizontal padding on each side of a number pane's text.
const NUMBER_PAD: u32 = 3;
/// Thickness of the usage gauge (height when horizontal, width when vertical).
pub const GAUGE_THICKNESS: u32 = 16;
/// Length of a horizontal usage gauge.
pub const GAUGE_LENGTH: u32 = 36;
/// Ruler ticks: major at ½ (this long), minor at ¼ and ¾ (half as long),
/// measured inward from each long edge.
const GAUGE_MAJOR_TICK: u32 = 4;

/// Pad/truncate a series to exactly `GRAPH_WIDTH` values (most recent last).
pub fn normalize_series(values: &[u64]) -> Vec<u64> {
    let n = GRAPH_WIDTH as usize;
    let mut out: Vec<u64> = Vec::with_capacity(n);
    if values.len() >= n {
        out.extend(values.iter().rev().take(n).rev().copied());
    } else {
        out.resize(n - values.len(), 0);
        out.extend_from_slice(values);
    }
    out
}

/// P95 of the non-zero values (`1` when there are none). Using P95 rather
/// than the max keeps a single outlier from squashing every other bar.
pub fn scale_reference<'a>(values: impl IntoIterator<Item = &'a u64>) -> u64 {
    let mut sorted: Vec<u64> = values.into_iter().copied().filter(|&t| t > 0).collect();
    if sorted.is_empty() {
        return 1;
    }
    sorted.sort_unstable();
    let p95_index = ((sorted.len() as f64 * 0.95).ceil() as usize).min(sorted.len() - 1);
    sorted[p95_index].max(1)
}

/// Bar height in pixels for one value in a full-size pane: fixed 5
/// tokens/px while the reference fits, otherwise scaled to the reference.
/// Zero values stay zero; non-zero values are at least 1px.
pub fn bar_height(value: u64, scale_reference: u64) -> u32 {
    bar_height_with(
        value,
        scale_reference,
        Some(TOKENS_PER_PIXEL),
        MAX_BAR_HEIGHT,
    )
}

/// [`bar_height`] with an explicit fixed scale and pane height.
/// `units_per_pixel = None` always scales to the reference (used for
/// request counts, whose P95 is far too small for the token fixed scale).
pub fn bar_height_with(
    value: u64,
    scale_reference: u64,
    units_per_pixel: Option<u64>,
    max_height: u32,
) -> u32 {
    if value == 0 {
        return 0;
    }
    match units_per_pixel {
        Some(upp) if scale_reference <= upp * max_height as u64 => {
            ((value / upp.max(1)) as u32).clamp(1, max_height)
        }
        _ => ((value as f64 / scale_reference as f64 * max_height as f64) as u32)
            .clamp(1, max_height),
    }
}

/// Render one graph pane (`PANE_SIZE` wide, `height` tall: rounded frame,
/// bars, optional overlay). `heights` has `GRAPH_WIDTH` entries.
fn render_pane(
    heights: &[u32],
    config: &GraphConfig,
    overlay: &TrayOverlay,
    dark_mode: bool,
    height: u32,
) -> RgbaImage {
    let width: u32 = PANE_SIZE;
    let height = height.max(2 * CORNER_RADIUS + 2);
    let max_bar_height = height - 2 * (BORDER_WIDTH + INNER_MARGIN);

    // Create image buffer
    let mut img = RgbaImage::from_pixel(width, height, config.background);

    let has_overlay = *overlay != TrayOverlay::None;

    // Top border (skip corner regions; wider skip at top-left when overlay present)
    let top_left_border_start = if has_overlay {
        CUTOUT_SIZE
    } else {
        CORNER_RADIUS
    };
    for x in top_left_border_start..(width - CORNER_RADIUS) {
        img.put_pixel(x, 0, config.foreground);
    }
    // Bottom border (skip corner regions)
    for x in CORNER_RADIUS..(width - CORNER_RADIUS) {
        img.put_pixel(x, height - 1, config.foreground);
    }
    // Left border (skip corner regions; wider skip at top-left when overlay present)
    let left_top_border_start = if has_overlay {
        CUTOUT_SIZE
    } else {
        CORNER_RADIUS
    };
    for y in left_top_border_start..(height - CORNER_RADIUS) {
        img.put_pixel(0, y, config.foreground);
    }
    // Right border (skip corner regions)
    for y in CORNER_RADIUS..(height - CORNER_RADIUS) {
        img.put_pixel(width - 1, y, config.foreground);
    }

    // Draw rounded corners (6-pixel radius)
    // Top-left corner: normal convex arc when no overlay, concave cutout when overlay present
    if has_overlay {
        // Quarter-circle notch centered on the overlay icon at (CX, CY).
        // Everything inside the circle is cleared to background.
        let max_clear_x = (CUTOUT_CX + CUTOUT_R).min(width as i32 - 1);
        let max_clear_y = (CUTOUT_CY + CUTOUT_R).min(height as i32 - 1);
        for y in 0..=max_clear_y {
            for x in 0..=max_clear_x {
                let dx = x - CUTOUT_CX;
                let dy = y - CUTOUT_CY;
                if dx * dx + dy * dy < CUTOUT_R_SQ {
                    img.put_pixel(x as u32, y as u32, config.background);
                }
            }
        }

        // Draw the arc border — sweep the full circle and only plot
        // pixels that land on screen and outside the box interior.
        for step in 0..=400 {
            let angle = 2.0 * std::f64::consts::PI * (step as f64 / 400.0);
            let px = CUTOUT_CX as f64 + CUTOUT_R as f64 * angle.cos();
            let py = CUTOUT_CY as f64 + CUTOUT_R as f64 * angle.sin();
            // Skip points that are mathematically off-screen but round onto (0,0)
            if px < 0.0 && py < 0.0 {
                continue;
            }
            let ix = px.round() as i32;
            let iy = py.round() as i32;
            if ix >= 0
                && ix < width as i32
                && iy >= 0
                && iy < height as i32
                && (ix <= CUTOUT_SIZE as i32 || iy <= CUTOUT_SIZE as i32)
            {
                img.put_pixel(ix as u32, iy as u32, config.foreground);
            }
        }
    } else {
        // Normal convex top-left corner (6px radius)
        img.put_pixel(1, 2, config.foreground);
        img.put_pixel(1, 3, config.foreground);
        img.put_pixel(1, 4, config.foreground);
        img.put_pixel(1, 5, config.foreground);
        img.put_pixel(2, 1, config.foreground);
        img.put_pixel(3, 1, config.foreground);
        img.put_pixel(4, 1, config.foreground);
        img.put_pixel(5, 1, config.foreground);
        img.put_pixel(2, 2, config.foreground);
    }

    // Top-right corner
    img.put_pixel(width - 2, 2, config.foreground);
    img.put_pixel(width - 2, 3, config.foreground);
    img.put_pixel(width - 2, 4, config.foreground);
    img.put_pixel(width - 2, 5, config.foreground);
    img.put_pixel(width - 3, 1, config.foreground);
    img.put_pixel(width - 4, 1, config.foreground);
    img.put_pixel(width - 5, 1, config.foreground);
    img.put_pixel(width - 6, 1, config.foreground);
    img.put_pixel(width - 3, 2, config.foreground);

    // Bottom-left corner
    img.put_pixel(1, height - 3, config.foreground);
    img.put_pixel(1, height - 4, config.foreground);
    img.put_pixel(1, height - 5, config.foreground);
    img.put_pixel(1, height - 6, config.foreground);
    img.put_pixel(2, height - 2, config.foreground);
    img.put_pixel(3, height - 2, config.foreground);
    img.put_pixel(4, height - 2, config.foreground);
    img.put_pixel(5, height - 2, config.foreground);
    img.put_pixel(2, height - 3, config.foreground);

    // Bottom-right corner
    img.put_pixel(width - 2, height - 3, config.foreground);
    img.put_pixel(width - 2, height - 4, config.foreground);
    img.put_pixel(width - 2, height - 5, config.foreground);
    img.put_pixel(width - 2, height - 6, config.foreground);
    img.put_pixel(width - 3, height - 2, config.foreground);
    img.put_pixel(width - 4, height - 2, config.foreground);
    img.put_pixel(width - 5, height - 2, config.foreground);
    img.put_pixel(width - 6, height - 2, config.foreground);
    img.put_pixel(width - 3, height - 3, config.foreground);

    // Draw bars (each bar is exactly 1 pixel wide, inside the border)
    for (i, &h) in heights.iter().take(GRAPH_WIDTH as usize).enumerate() {
        if h == 0 {
            continue;
        }
        let bar_height = h.min(max_bar_height);
        let x = GRAPH_OFFSET_X + i as u32;
        let start_y = height - GRAPH_OFFSET_Y - bar_height;
        let end_y = height - GRAPH_OFFSET_Y;
        for y in start_y..end_y {
            // Skip pixels inside the circle cutout
            if has_overlay {
                let dx = x as i32 - CUTOUT_CX;
                let dy = y as i32 - CUTOUT_CY;
                if dx * dx + dy * dy < CUTOUT_R_SQ {
                    continue;
                }
            }
            img.put_pixel(x, y, config.foreground);
        }
    }

    // Draw overlay icon in the carved-out top-left corner
    match overlay {
        TrayOverlay::None => {}
        TrayOverlay::Warning(color) => {
            draw_exclamation_mark(&mut img, *color);
        }
        TrayOverlay::UpdateAvailable => {
            draw_down_arrow(&mut img, config.foreground);
        }
        TrayOverlay::FirewallPending => {
            draw_question_mark(&mut img, StatusDotColors::red(dark_mode));
        }
    }

    img
}

/// Generate a single full-size PNG sparkline graph from data points.
///
/// Creates a filled vertical bar chart showing token usage over time,
/// exactly `GRAPH_WIDTH` bars wide, with a rounded 1px frame and the
/// optional overlay in the carved-out top-left corner.
pub fn generate_graph(
    data_points: &[DataPoint],
    config: &GraphConfig,
    overlay: TrayOverlay,
    dark_mode: bool,
) -> Option<Vec<u8>> {
    let values: Vec<u64> = data_points.iter().map(|p| p.total_tokens).collect();
    let values = normalize_series(&values);
    let scale = scale_reference(values.iter());
    let heights: Vec<u32> = values.iter().map(|&v| bar_height(v, scale)).collect();
    let img = render_pane(&heights, config, &overlay, dark_mode, PANE_SIZE);
    encode_png(&img)
}

/// What one panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneContent {
    /// Sparkline values, oldest first (padded/truncated to `GRAPH_WIDTH`).
    Graph(Vec<u64>),
    /// Horizontal outlined gauge, fill `0.0..=1.0`.
    UsageBar(f32),
    /// Text drawn horizontally in the bitmap font (e.g. `24.1K`, `$0.42`).
    Number(String),
}

impl Default for PaneContent {
    fn default() -> Self {
        PaneContent::Graph(Vec::new())
    }
}

/// One panel of a multi-pane tray icon.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneSpec {
    /// Label (already normalized, ≤ 4 chars).
    pub label: Option<String>,
    pub content: PaneContent,
}

/// Where labels go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelMode {
    Off,
    /// Label stacked beside the content (full-size graph).
    Beside,
    /// Label above the content (graph at reduced height).
    Above,
}

/// Icon-wide rendering options.
#[derive(Debug, Clone, Copy)]
pub struct MultiPaneOptions {
    pub labels: LabelMode,
    /// Fixed scale for graph bars (`Some(TOKENS_PER_PIXEL)` for tokens);
    /// `None` always auto-scales to the shared P95.
    pub units_per_pixel: Option<u64>,
}

/// How one pane lays out under the current options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneLayout {
    /// Content only.
    Plain,
    /// Stacked label column (shared width), gap, content.
    Beside,
    /// Label row, content underneath (graph at reduced height).
    Above,
}

fn has_label(pane: &PaneSpec) -> bool {
    pane.label.as_deref().is_some_and(|l| !l.is_empty())
}

fn pane_layout(pane: &PaneSpec, options: MultiPaneOptions) -> PaneLayout {
    match options.labels {
        _ if !has_label(pane) => PaneLayout::Plain,
        LabelMode::Off => PaneLayout::Plain,
        LabelMode::Beside => PaneLayout::Beside,
        LabelMode::Above => PaneLayout::Above,
    }
}

/// Width of the shared stacked-label column (widest label wins); 0 unless
/// labels are beside the content.
fn beside_column_width(panes: &[PaneSpec], options: MultiPaneOptions) -> u32 {
    if options.labels != LabelMode::Beside {
        return 0;
    }
    panes
        .iter()
        .filter_map(|p| p.label.as_deref())
        .map(crate::ui::tray_font::vertical_label_width)
        .max()
        .unwrap_or(0)
}

/// Width of a label drawn horizontally above content.
fn label_row_width(pane: &PaneSpec) -> u32 {
    pane.label
        .as_deref()
        .map(|l| crate::ui::tray_font::above_label_metrics().text_width(l))
        .unwrap_or(0)
}

/// Height of the box a pane's content is drawn in under `layout`.
fn content_box_height(layout: PaneLayout) -> u32 {
    match layout {
        PaneLayout::Above => PANE_SIZE - LABEL_ROW_HEIGHT,
        _ => PANE_SIZE,
    }
}

/// Font used for a number in a content box `box_height` tall: fills the
/// box up to `NUMBER_MAX_GLYPH_HEIGHT`. Width follows the text, so a pane
/// only grows when more digits or a unit are needed.
fn number_metrics(box_height: u32) -> crate::ui::tray_font::FontMetrics {
    crate::ui::tray_font::FontMetrics::for_height(
        box_height
            .saturating_sub(2 * NUMBER_MARGIN)
            .clamp(NUMBER_MIN_GLYPH_HEIGHT, NUMBER_MAX_GLYPH_HEIGHT),
    )
}

/// Width of a pane's content alone under `layout`.
fn content_width(content: &PaneContent, layout: PaneLayout) -> u32 {
    match content {
        PaneContent::Graph(_) => PANE_SIZE,
        // Beside a stacked label the gauge stands upright, like the label
        PaneContent::UsageBar(_) if layout == PaneLayout::Beside => GAUGE_THICKNESS,
        PaneContent::UsageBar(_) => GAUGE_LENGTH,
        PaneContent::Number(text) => {
            number_metrics(content_box_height(layout)).text_width(text) + 2 * NUMBER_PAD
        }
    }
}

/// Pixel width of one pane. `column` is the shared beside-label width.
fn pane_width(pane: &PaneSpec, options: MultiPaneOptions, column: u32) -> u32 {
    let content = content_width(&pane.content, pane_layout(pane, options));
    match pane_layout(pane, options) {
        PaneLayout::Plain => content,
        PaneLayout::Beside => column + PANE_GAP + content,
        PaneLayout::Above => content.max(label_row_width(pane) + 2 * NUMBER_PAD),
    }
}

/// Total icon width for `panes` under `options`.
pub fn multi_pane_width(panes: &[PaneSpec], options: MultiPaneOptions) -> u32 {
    if panes.is_empty() {
        return 0;
    }
    let column = beside_column_width(panes, options);
    panes
        .iter()
        .map(|p| pane_width(p, options, column))
        .sum::<u32>()
        + (panes.len() as u32 - 1) * PANE_GAP
}

/// Draw a label centred horizontally in `[x, x + width)` at the top.
fn draw_label_row(img: &mut RgbaImage, x: u32, width: u32, label: &str, config: &GraphConfig) {
    let m = crate::ui::tray_font::above_label_metrics();
    let tw = m.text_width(label);
    let lx = x + width.saturating_sub(tw) / 2;
    crate::ui::tray_font::draw_text(img, lx, 0, label, m, config.foreground);
}

/// Draw an outlined usage gauge with ruler ticks in the box
/// `(x, y, width, height)`. The long axis is the fill direction: a wide box
/// fills left-to-right, a tall box bottom-to-top. Ticks sit at ¼ (minor),
/// ½ (major) and ¾ (minor) of the fill length, cut into the fill where it
/// covers them so they read like a ruler either way.
fn draw_usage_gauge(
    img: &mut RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    fill: f32,
    config: &GraphConfig,
) {
    let horizontal = width >= height;
    // 1px outline with clipped corners
    for dx in 1..width - 1 {
        img.put_pixel(x + dx, y, config.foreground);
        img.put_pixel(x + dx, y + height - 1, config.foreground);
    }
    for dy in 1..height - 1 {
        img.put_pixel(x, y + dy, config.foreground);
        img.put_pixel(x + width - 1, y + dy, config.foreground);
    }

    // Fill inset 2px from the outline
    let (length, thickness) = if horizontal {
        (width - 4, height - 4)
    } else {
        (height - 4, width - 4)
    };
    let fill = fill.clamp(0.0, 1.0);
    let filled = if fill <= 0.0 {
        0
    } else {
        ((fill * length as f32).round() as u32).clamp(1, length)
    };
    // `along` runs 0..length in the fill direction, `across` 0..thickness
    let mut put = |along: u32, across: u32, color: Rgba<u8>| {
        let (px, py) = if horizontal {
            (x + 2 + along, y + 2 + across)
        } else {
            (x + 2 + across, y + height - 3 - along)
        };
        img.put_pixel(px, py, color);
    };
    for along in 0..filled {
        for across in 0..thickness {
            put(along, across, config.foreground);
        }
    }

    // Ruler ticks from both long edges
    let minor = GAUGE_MAJOR_TICK / 2;
    for (quarter, len) in [(1, minor), (2, GAUGE_MAJOR_TICK), (3, minor)] {
        let along = (length * quarter / 4).min(length - 1);
        let color = if along < filled {
            config.background
        } else {
            config.foreground
        };
        for across in (0..len.min(thickness)).chain((thickness.saturating_sub(len))..thickness) {
            put(along, across, color);
        }
    }
}

/// Generate a wide tray icon with one pane per item.
///
/// Graph panes share one vertical scale (the P95 across all graph panes) so
/// bar heights are comparable between items. The overlay is drawn on the
/// first graph pane only. Returns `None` when there is nothing to draw.
pub fn generate_multi_pane(
    panes: &[PaneSpec],
    options: MultiPaneOptions,
    config: &GraphConfig,
    overlay: TrayOverlay,
    dark_mode: bool,
) -> Option<Vec<u8>> {
    let width = multi_pane_width(panes, options);
    if width == 0 {
        return None;
    }

    let series: Vec<Option<Vec<u64>>> = panes
        .iter()
        .map(|p| match &p.content {
            PaneContent::Graph(bars) => Some(normalize_series(bars)),
            _ => None,
        })
        .collect();
    let scale = scale_reference(series.iter().flatten().flatten());

    let mut img = RgbaImage::from_pixel(width, PANE_SIZE, config.background);
    let column = beside_column_width(panes, options);
    let mut x = 0;
    let mut overlay_pending = overlay;

    for (pane, values) in panes.iter().zip(series.iter()) {
        if x > 0 {
            x += PANE_GAP;
        }
        let w = pane_width(pane, options, column);
        let layout = pane_layout(pane, options);
        let label = pane.label.as_deref().unwrap_or("");

        // Label, and the box the content gets: (left, top, width, height)
        let (cx, cy, cw, ch) = match layout {
            PaneLayout::Plain => (x, 0, w, PANE_SIZE),
            PaneLayout::Beside => {
                crate::ui::tray_font::draw_vertical_label(
                    &mut img,
                    x,
                    column,
                    label,
                    config.foreground,
                );
                (x + column + PANE_GAP, 0, w - column - PANE_GAP, PANE_SIZE)
            }
            PaneLayout::Above => {
                draw_label_row(&mut img, x, w, label, config);
                (x, LABEL_ROW_HEIGHT, w, PANE_SIZE - LABEL_ROW_HEIGHT)
            }
        };

        match &pane.content {
            PaneContent::Graph(_) => {
                let values = values.as_ref().expect("graph pane has a series");
                let gh = ch.min(PANE_SIZE);
                let gx = cx + cw.saturating_sub(PANE_SIZE) / 2;
                let gy = cy + ch - gh;
                let max_bar = gh - 2 * (BORDER_WIDTH + INNER_MARGIN);
                let heights: Vec<u32> = values
                    .iter()
                    .map(|&v| bar_height_with(v, scale, options.units_per_pixel, max_bar))
                    .collect();
                let pane_overlay = std::mem::replace(&mut overlay_pending, TrayOverlay::None);
                let pane_img = render_pane(&heights, config, &pane_overlay, dark_mode, gh);
                image::imageops::replace(&mut img, &pane_img, gx as i64, gy as i64);
            }
            PaneContent::UsageBar(fill) => {
                if layout == PaneLayout::Beside {
                    // Upright gauge spanning the full height
                    let gx = cx + cw.saturating_sub(GAUGE_THICKNESS) / 2;
                    draw_usage_gauge(&mut img, gx, cy, GAUGE_THICKNESS, ch, *fill, config);
                } else {
                    // Fixed size regardless of label width; centred in the pane
                    let gw = GAUGE_LENGTH.min(cw);
                    let gh = GAUGE_THICKNESS.min(ch);
                    let gx = cx + (cw - gw) / 2;
                    let gy = cy + (ch - gh) / 2;
                    draw_usage_gauge(&mut img, gx, gy, gw, gh, *fill, config);
                }
            }
            PaneContent::Number(text) => {
                let m = number_metrics(ch);
                let ty = cy + ch.saturating_sub(m.glyph_height) / 2;
                let tx = cx + cw.saturating_sub(m.text_width(text)) / 2;
                crate::ui::tray_font::draw_text(&mut img, tx, ty, text, m, config.foreground);
            }
        }
        x += w;
    }

    encode_png(&img)
}

/// Decode a static icon PNG and recolor it for the current theme.
///
/// The graphic (dark pixels in the source) is recolored to white when
/// `dark_mode` is true and to black when it's false, so the icon stays
/// visible against the macOS menu bar in both appearances. Source
/// background pixels become fully transparent.
fn decode_static_icon_for_theme(static_icon_bytes: &[u8], dark_mode: bool) -> Option<RgbaImage> {
    use image::ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(static_icon_bytes))
        .with_guessed_format()
        .ok()?;
    let decoded = reader.decode().ok()?;
    let mut img = decoded.to_rgba8();

    let fg: u8 = if dark_mode { 255 } else { 0 };

    for pixel in img.pixels_mut() {
        if pixel[3] > 0 {
            let luminance = (pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3;
            if luminance < 128 {
                // Dark source pixel = part of the graphic → theme foreground
                pixel[0] = fg;
                pixel[1] = fg;
                pixel[2] = fg;
                pixel[3] = 255;
            } else {
                // Light source pixel = background → transparent
                pixel[0] = 0;
                pixel[1] = 0;
                pixel[2] = 0;
                pixel[3] = 0;
            }
        }
    }

    Some(img)
}

/// Generate the static tray icon recolored for the current theme.
///
/// White graphic on transparent background when `dark_mode` is true,
/// black graphic when it's false. On macOS the tray sets template mode
/// at install time, so RGB is ignored and the menu bar recolors per
/// appearance — the `dark_mode` choice only matters on Windows/Linux.
///
/// # Returns
/// PNG-encoded image as bytes, or None if generation fails
pub fn generate_static_icon(static_icon_bytes: &[u8], dark_mode: bool) -> Option<Vec<u8>> {
    let img = decode_static_icon_for_theme(static_icon_bytes, dark_mode)?;
    encode_png(&img)
}

/// Generate a static icon with overlay drawn on top
///
/// Decodes the static icon PNG and recolors it for the current theme,
/// draws the overlay icon in the top-left corner, and re-encodes.
/// This avoids rendering the graph frame border in static mode.
///
/// # Arguments
/// * `static_icon_bytes` - Raw PNG bytes of the static icon
/// * `overlay` - Overlay icon to draw (must not be None)
/// * `dark_mode` - Whether the system is in dark mode
///
/// # Returns
/// PNG-encoded image as bytes, or None if generation fails
pub fn generate_static_icon_with_overlay(
    static_icon_bytes: &[u8],
    overlay: TrayOverlay,
    dark_mode: bool,
) -> Option<Vec<u8>> {
    let mut img = decode_static_icon_for_theme(static_icon_bytes, dark_mode)?;

    // Clear the top-left area where the overlay will be drawn.
    // Uses the same cutout geometry as the graph mode: a circle centered
    // on (6, 6) with radius 9, clearing all pixels inside it.
    if overlay != TrayOverlay::None {
        const CUTOUT_CX: i32 = 6;
        const CUTOUT_CY: i32 = 6;
        const CUTOUT_R: i32 = 9;
        const CUTOUT_R_SQ: i32 = CUTOUT_R * CUTOUT_R;
        let max_clear_x = (CUTOUT_CX + CUTOUT_R).min(img.width() as i32 - 1);
        let max_clear_y = (CUTOUT_CY + CUTOUT_R).min(img.height() as i32 - 1);
        for y in 0..=max_clear_y {
            for x in 0..=max_clear_x {
                let dx = x - CUTOUT_CX;
                let dy = y - CUTOUT_CY;
                if dx * dx + dy * dy < CUTOUT_R_SQ {
                    img.put_pixel(x as u32, y as u32, Rgba([0, 0, 0, 0]));
                }
            }
        }
    }

    // Draw overlay icon in the top-left corner
    match &overlay {
        TrayOverlay::None => {}
        TrayOverlay::Warning(color) => {
            draw_exclamation_mark(&mut img, *color);
        }
        TrayOverlay::UpdateAvailable => {
            let fg = if dark_mode {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            };
            draw_down_arrow(&mut img, fg);
        }
        TrayOverlay::FirewallPending => {
            draw_question_mark(&mut img, StatusDotColors::red(dark_mode));
        }
    }

    encode_png(&img)
}

/// Encode image as PNG bytes
fn encode_png(img: &RgbaImage) -> Option<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());

    let encoder = PngEncoder::new(&mut buffer);
    match encoder.write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    ) {
        Ok(_) => Some(buffer.into_inner()),
        Err(e) => {
            error!("Failed to encode PNG: {}", e);
            None
        }
    }
}

/// Get platform-specific graph config
#[cfg(target_os = "macos")]
pub fn platform_graph_config(dark_mode: bool) -> GraphConfig {
    GraphConfig::macos(dark_mode)
}

/// Get platform-specific graph config
#[cfg(not(target_os = "macos"))]
pub fn platform_graph_config(_dark_mode: bool) -> GraphConfig {
    GraphConfig::windows_linux()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_generate_empty_graph() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(&[], &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_generate_single_point_graph() {
        let config = GraphConfig::macos_template();
        let data = vec![DataPoint {
            timestamp: Utc::now(),
            total_tokens: 1000,
        }];
        let png = generate_graph(&data, &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_generate_multiple_points_graph() {
        let config = GraphConfig::macos_template();
        let now = Utc::now();
        let mut data = Vec::new();

        // Create 15 data points with varying token counts
        for i in 0..15 {
            data.push(DataPoint {
                timestamp: now - Duration::minutes(15 - i),
                total_tokens: (i * 100) as u64,
            });
        }

        let png = generate_graph(&data, &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_fixed_scale() {
        let config = GraphConfig::macos_template();
        let now = Utc::now();

        // Test with small numbers (should use fixed scale: 1px = 5 tokens)
        let data = vec![
            DataPoint {
                timestamp: now - Duration::minutes(2),
                total_tokens: 50, // Should be 10px
            },
            DataPoint {
                timestamp: now - Duration::minutes(1),
                total_tokens: 100, // Should be 20px
            },
            DataPoint {
                timestamp: now,
                total_tokens: 150, // Should be 30px (max fixed scale)
            },
        ];

        let png = generate_graph(&data, &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_auto_scale() {
        let config = GraphConfig::macos_template();
        let now = Utc::now();

        // Test with large numbers (should trigger auto-scaling)
        let data = vec![
            DataPoint {
                timestamp: now - Duration::minutes(2),
                total_tokens: 1_000,
            },
            DataPoint {
                timestamp: now - Duration::minutes(1),
                total_tokens: 500,
            },
            DataPoint {
                timestamp: now,
                total_tokens: 750,
            },
        ];

        let png = generate_graph(&data, &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_platform_configs() {
        let _config = platform_graph_config(true);
        // Just ensure it doesn't panic
    }

    #[test]
    fn test_windows_linux_config() {
        let config = GraphConfig::windows_linux();
        assert!(!config.template_mode);
        assert_eq!(config.foreground, Rgba([0, 120, 215, 255]));
    }

    #[test]
    fn test_macos_template_config() {
        let config = GraphConfig::macos_template();
        assert!(config.template_mode);
        assert_eq!(config.foreground, Rgba([255, 255, 255, 255]));
        assert_eq!(config.background, Rgba([0, 0, 0, 0])); // Transparent
    }

    #[test]
    fn test_percentile_scaling_with_outlier() {
        let config = GraphConfig::macos_template();
        let now = Utc::now();

        // Test with an outlier: most values around 100-120, but one at 1000
        // P95 should be around 120, not 1000, so graph should use 120 as scale reference
        let mut data = Vec::new();
        for i in 0..20 {
            data.push(DataPoint {
                timestamp: now - Duration::minutes(20 - i),
                total_tokens: 100 + ((i % 3) * 10) as u64, // 100, 110, 120, repeated
            });
        }
        // Add one outlier
        data.push(DataPoint {
            timestamp: now,
            total_tokens: 1000,
        });

        let png = generate_graph(&data, &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());

        // The graph should successfully render without the outlier squashing all other bars
        // (Previously, all bars would be scaled relative to 1000, making them tiny)
    }

    #[test]
    fn test_consistent_tokens_over_time() {
        let config = GraphConfig::macos_template();
        let now = Utc::now();

        // Test with consistent token counts (simulating the user's scenario)
        let mut data = Vec::new();
        for i in 0..26 {
            data.push(DataPoint {
                timestamp: now - Duration::minutes(26 - i),
                total_tokens: 100, // Consistent 100 tokens per minute
            });
        }

        let png = generate_graph(&data, &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());

        // All bars should have the same height since token counts are consistent
    }

    #[test]
    fn test_overlay_none() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(&[], &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_overlay_warning_yellow() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(
            &[],
            &config,
            TrayOverlay::Warning(StatusDotColors::yellow(false)),
            false,
        );
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_overlay_warning_red() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(
            &[],
            &config,
            TrayOverlay::Warning(StatusDotColors::red(false)),
            false,
        );
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_overlay_update_available() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(&[], &config, TrayOverlay::UpdateAvailable, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_status_dot_colors_light_mode() {
        assert_eq!(StatusDotColors::green(false), Rgba([34, 197, 94, 255]));
        assert_eq!(StatusDotColors::yellow(false), Rgba([234, 179, 8, 255]));
        assert_eq!(StatusDotColors::red(false), Rgba([239, 68, 68, 255]));
    }

    #[test]
    fn test_status_dot_colors_dark_mode() {
        assert_eq!(StatusDotColors::green(true), Rgba([74, 222, 128, 255]));
        assert_eq!(StatusDotColors::yellow(true), Rgba([250, 204, 21, 255]));
        assert_eq!(StatusDotColors::red(true), Rgba([248, 113, 113, 255]));
    }

    #[test]
    fn test_graph_with_lr_letters() {
        // Test that graph renders with LR letters overlay
        let config = GraphConfig::macos_template();
        let data = vec![DataPoint {
            timestamp: Utc::now(),
            total_tokens: 100,
        }];
        let png = generate_graph(&data, &config, TrayOverlay::None, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_graph_with_warning_overlay() {
        // Test that graph renders with warning overlay
        let config = GraphConfig::macos_template();
        let data = vec![DataPoint {
            timestamp: Utc::now(),
            total_tokens: 100,
        }];
        let png = generate_graph(
            &data,
            &config,
            TrayOverlay::Warning(StatusDotColors::yellow(false)),
            false,
        );
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    fn decode(png: &[u8]) -> RgbaImage {
        use image::ImageReader;
        ImageReader::new(Cursor::new(png))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8()
    }

    fn opts(labels: LabelMode) -> MultiPaneOptions {
        MultiPaneOptions {
            labels,
            units_per_pixel: Some(TOKENS_PER_PIXEL),
        }
    }

    fn graph(bars: Vec<u64>) -> PaneSpec {
        PaneSpec {
            label: None,
            content: PaneContent::Graph(bars),
        }
    }

    fn labelled(label: &str, content: PaneContent) -> PaneSpec {
        PaneSpec {
            label: Some(label.into()),
            content,
        }
    }

    fn ink_in(img: &RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32) -> bool {
        (x0..x1.min(img.width()))
            .any(|x| (y0..y1.min(img.height())).any(|y| img.get_pixel(x, y)[3] != 0))
    }

    #[test]
    fn normalize_series_pads_left_and_keeps_latest() {
        assert_eq!(normalize_series(&[]).len(), GRAPH_WIDTH as usize);
        let short = normalize_series(&[1, 2, 3]);
        assert_eq!(short.len(), GRAPH_WIDTH as usize);
        assert_eq!(&short[GRAPH_WIDTH as usize - 3..], &[1, 2, 3]);
        assert!(short[..GRAPH_WIDTH as usize - 3].iter().all(|&v| v == 0));
        let long: Vec<u64> = (0..40).collect();
        let n = normalize_series(&long);
        assert_eq!(n[0], 10);
        assert_eq!(*n.last().unwrap(), 39);
    }

    #[test]
    fn scale_reference_uses_p95_of_nonzero() {
        assert_eq!(scale_reference([0u64, 0, 0].iter()), 1);
        assert_eq!(scale_reference([10u64].iter()), 10);
        // ceil(n * 0.95) indexes past a single outlier once n >= 21
        let mut v = vec![100u64; 39];
        v.push(10_000); // one outlier
        assert_eq!(scale_reference(v.iter()), 100);
        // ...but not for a small sample, where the P95 index is the max
        let mut small = vec![100u64; 19];
        small.push(10_000);
        assert_eq!(scale_reference(small.iter()), 10_000);
    }

    #[test]
    fn bar_height_fixed_vs_auto_scale() {
        assert_eq!(bar_height(0, 1), 0);
        assert_eq!(bar_height(50, 100), 10); // fixed: 5 tokens / px
        assert_eq!(bar_height(3, 100), 1); // non-zero never disappears
        assert_eq!(bar_height(1000, 1000), MAX_BAR_HEIGHT); // auto: fits reference
        assert_eq!(bar_height(500, 1000), MAX_BAR_HEIGHT / 2);
        // Requests: no fixed scale, small values still fill the pane
        assert_eq!(bar_height_with(2, 2, None, MAX_BAR_HEIGHT), MAX_BAR_HEIGHT);
        assert_eq!(
            bar_height_with(1, 2, None, MAX_BAR_HEIGHT),
            MAX_BAR_HEIGHT / 2
        );
        // Reduced pane height caps the bars
        assert_eq!(bar_height_with(1000, 1000, Some(5), 16), 16);
    }

    #[test]
    fn single_pane_matches_generate_graph() {
        let now = Utc::now();
        let data: Vec<DataPoint> = (0..30)
            .map(|i| DataPoint {
                timestamp: now - Duration::seconds(30 - i),
                total_tokens: (i as u64 * 7) % 90,
            })
            .collect();
        let config = GraphConfig::macos(false);
        let single = generate_graph(&data, &config, TrayOverlay::UpdateAvailable, false).unwrap();
        assert_eq!(decode(&single).dimensions(), (PANE_SIZE, PANE_SIZE));
        let bars: Vec<u64> = data.iter().map(|d| d.total_tokens).collect();
        let multi = generate_multi_pane(
            &[graph(bars)],
            opts(LabelMode::Off),
            &config,
            TrayOverlay::UpdateAvailable,
            false,
        )
        .unwrap();
        assert_eq!(decode(&single).as_raw(), decode(&multi).as_raw());
    }

    #[test]
    fn multi_pane_widths_per_layout() {
        let config = GraphConfig::macos(false);
        let am = crate::ui::tray_font::above_label_metrics();
        let big = number_metrics(PANE_SIZE); // numbers with no label / label beside
        let small = number_metrics(PANE_SIZE - LABEL_ROW_HEIGHT); // under a label
        assert!(big.glyph_height >= small.glyph_height);
        assert_eq!(big.glyph_height, NUMBER_MAX_GLYPH_HEIGHT);
        assert_eq!(number_metrics(200).glyph_height, NUMBER_MAX_GLYPH_HEIGHT);
        assert_eq!(number_metrics(5).glyph_height, NUMBER_MIN_GLYPH_HEIGHT);
        // Wider text → wider pane; same height
        let wide = vec![labelled("ALL", PaneContent::Number("$0.0042".into()))];
        let narrow = vec![labelled("ALL", PaneContent::Number("42".into()))];
        assert!(
            multi_pane_width(&wide, opts(LabelMode::Off))
                > multi_pane_width(&narrow, opts(LabelMode::Off))
        );

        let graphs = vec![
            labelled("ALL", PaneContent::Graph(vec![])),
            labelled("CLAU", PaneContent::Graph(vec![])),
        ];
        assert_eq!(
            multi_pane_width(&graphs, opts(LabelMode::Off)),
            2 * PANE_SIZE + PANE_GAP
        );
        // Beside: one shared column as wide as the widest (shortest) label
        let col = crate::ui::tray_font::vertical_label_width("ALL");
        assert!(col > crate::ui::tray_font::vertical_label_width("CLAU"));
        assert_eq!(
            multi_pane_width(&graphs, opts(LabelMode::Beside)),
            2 * (col + PANE_GAP + PANE_SIZE) + PANE_GAP
        );
        // Above: a pane is as wide as its label row or its content, whichever is wider
        let above_w = |l: &str, content: u32| content.max(am.text_width(l) + 2 * NUMBER_PAD);
        assert_eq!(
            multi_pane_width(&graphs, opts(LabelMode::Above)),
            above_w("ALL", PANE_SIZE) + above_w("CLAU", PANE_SIZE) + PANE_GAP
        );

        let gauges = vec![labelled("ALL", PaneContent::UsageBar(1.0))];
        assert_eq!(
            multi_pane_width(&gauges, opts(LabelMode::Off)),
            GAUGE_LENGTH
        );
        // Beside a stacked label the gauge stands upright
        assert_eq!(
            multi_pane_width(&gauges, opts(LabelMode::Beside)),
            col + PANE_GAP + GAUGE_THICKNESS
        );
        assert_eq!(
            multi_pane_width(&gauges, opts(LabelMode::Above)),
            above_w("ALL", GAUGE_LENGTH)
        );

        let numbers = vec![labelled("CLAU", PaneContent::Number("$1.2K".into()))];
        assert_eq!(
            multi_pane_width(&numbers, opts(LabelMode::Off)),
            big.text_width("$1.2K") + 2 * NUMBER_PAD
        );
        assert_eq!(
            multi_pane_width(&numbers, opts(LabelMode::Beside)),
            crate::ui::tray_font::vertical_label_width("CLAU")
                + PANE_GAP
                + big.text_width("$1.2K")
                + 2 * NUMBER_PAD
        );
        assert_eq!(
            multi_pane_width(&numbers, opts(LabelMode::Above)),
            above_w("CLAU", small.text_width("$1.2K") + 2 * NUMBER_PAD)
        );

        for (panes, mode) in [
            (&graphs, LabelMode::Beside),
            (&graphs, LabelMode::Above),
            (&gauges, LabelMode::Off),
            (&gauges, LabelMode::Beside),
            (&gauges, LabelMode::Above),
            (&numbers, LabelMode::Off),
            (&numbers, LabelMode::Beside),
            (&numbers, LabelMode::Above),
        ] {
            let png =
                generate_multi_pane(panes, opts(mode), &config, TrayOverlay::None, false).unwrap();
            assert_eq!(
                decode(&png).dimensions(),
                (multi_pane_width(panes, opts(mode)), PANE_SIZE)
            );
        }
        assert!(
            generate_multi_pane(&[], opts(LabelMode::Off), &config, TrayOverlay::None, false)
                .is_none()
        );
    }

    #[test]
    fn multi_pane_shares_one_scale() {
        let config = GraphConfig::macos(false);
        // Pane A peaks at 1000 tokens, pane B at 500 → B's bar is half of A's
        let panes = vec![graph(vec![1000]), graph(vec![500])];
        let img = decode(
            &generate_multi_pane(
                &panes,
                opts(LabelMode::Off),
                &config,
                TrayOverlay::None,
                false,
            )
            .unwrap(),
        );
        let col_height = |x: u32| {
            (3..PANE_SIZE - 3)
                .filter(|&y| img.get_pixel(x, y)[3] != 0)
                .count() as u32
        };
        let last_bar_x = 3 + GRAPH_WIDTH - 1;
        assert_eq!(col_height(last_bar_x), MAX_BAR_HEIGHT);
        assert_eq!(
            col_height(PANE_SIZE + PANE_GAP + last_bar_x),
            MAX_BAR_HEIGHT / 2
        );
    }

    #[test]
    fn overlay_only_on_first_graph_pane() {
        let config = GraphConfig::macos(false);
        let panes = vec![graph(vec![]), graph(vec![])];
        let img = decode(
            &generate_multi_pane(
                &panes,
                opts(LabelMode::Off),
                &config,
                TrayOverlay::UpdateAvailable,
                false,
            )
            .unwrap(),
        );
        // Down-arrow stem pixel at (6,3) is set in pane 0 but not pane 1
        assert_ne!(img.get_pixel(6, 3)[3], 0);
        let x1 = PANE_SIZE + PANE_GAP;
        assert_eq!(img.get_pixel(x1 + 6, 3)[3], 0);
        // Pane 1 keeps the convex corner (top border starts at x=6)
        assert_ne!(img.get_pixel(x1 + 6, 0)[3], 0);
        // The gap columns between panes are empty
        assert!(!ink_in(&img, PANE_SIZE, x1, 0, PANE_SIZE));
    }

    #[test]
    fn label_beside_and_above_layouts() {
        let config = GraphConfig::macos(false);
        let panes = vec![labelled("ALL", PaneContent::Graph(vec![100; 30]))];
        let lw = crate::ui::tray_font::vertical_label_width("ALL");

        // Beside: label column, empty gap, then a full-height frame
        let img = decode(
            &generate_multi_pane(
                &panes,
                opts(LabelMode::Beside),
                &config,
                TrayOverlay::None,
                false,
            )
            .unwrap(),
        );
        assert!(ink_in(&img, 0, lw, 0, PANE_SIZE));
        assert!(!ink_in(&img, lw, lw + PANE_GAP, 0, PANE_SIZE));
        let gx = lw + PANE_GAP;
        assert_ne!(img.get_pixel(gx + 10, 0)[3], 0); // top border
        assert_ne!(img.get_pixel(gx + 10, PANE_SIZE - 1)[3], 0); // bottom border

        // Above: label row on top, reduced frame at the bottom
        let img = decode(
            &generate_multi_pane(
                &panes,
                opts(LabelMode::Above),
                &config,
                TrayOverlay::None,
                false,
            )
            .unwrap(),
        );
        let lh = crate::ui::tray_font::ABOVE_LABEL_GLYPH_HEIGHT;
        assert!(ink_in(&img, 0, PANE_SIZE, 0, lh));
        let top = PANE_SIZE - SMALL_GRAPH_HEIGHT;
        assert!(!ink_in(&img, 0, PANE_SIZE, lh, top));
        assert_ne!(img.get_pixel(10, top)[3], 0); // frame top border
        assert_ne!(img.get_pixel(10, PANE_SIZE - 1)[3], 0); // frame bottom border
                                                            // Bars are capped to the small frame
        let bar_px = (top..PANE_SIZE)
            .filter(|&y| img.get_pixel(3, y)[3] != 0)
            .count() as u32;
        assert!(bar_px <= SMALL_GRAPH_HEIGHT - 4);

        // Beside a gauge: column, gap, then an upright gauge spanning the height
        let gauge = vec![labelled("ALL", PaneContent::UsageBar(0.5))];
        let img = decode(
            &generate_multi_pane(
                &gauge,
                opts(LabelMode::Beside),
                &config,
                TrayOverlay::None,
                false,
            )
            .unwrap(),
        );
        assert_ne!(img.get_pixel(gx, 10)[3], 0); // gauge left edge
        assert_ne!(img.get_pixel(gx + GAUGE_THICKNESS - 1, 10)[3], 0); // gauge right edge
        assert_ne!(img.get_pixel(gx + 8, 0)[3], 0); // top edge at the very top
        assert_ne!(img.get_pixel(gx + 8, PANE_SIZE - 1)[3], 0); // bottom edge at the very bottom
                                                                // Half full from the bottom: lower interior filled, upper empty
        assert_ne!(img.get_pixel(gx + 8, PANE_SIZE - 5)[3], 0);
        assert_eq!(img.get_pixel(gx + 8, 5)[3], 0);
    }

    #[test]
    fn usage_gauge_and_number_panes_draw() {
        let config = GraphConfig::macos(false);
        let panes = vec![
            labelled("ALL", PaneContent::UsageBar(1.0)),
            labelled("CLAU", PaneContent::UsageBar(0.0)),
            labelled("GPT5", PaneContent::Number("1.2M".into())),
        ];
        let img = decode(
            &generate_multi_pane(
                &panes,
                opts(LabelMode::Above),
                &config,
                TrayOverlay::None,
                false,
            )
            .unwrap(),
        );
        let lh = crate::ui::tray_font::ABOVE_LABEL_GLYPH_HEIGHT;
        // Labels on top of each pane
        assert!(ink_in(&img, 0, GAUGE_LENGTH, 0, lh));
        // Gauge 0: full-width outline in the lower area, interior filled
        let content_top = LABEL_ROW_HEIGHT;
        let gh = GAUGE_THICKNESS.min(PANE_SIZE - content_top);
        let gy = content_top + (PANE_SIZE - content_top - gh) / 2;
        let mid = gy + gh / 2;
        assert_ne!(img.get_pixel(0, mid)[3], 0); // left edge
        assert_ne!(img.get_pixel(GAUGE_LENGTH - 1, mid)[3], 0); // right edge
        assert_ne!(img.get_pixel(GAUGE_LENGTH - 3, mid)[3], 0); // filled to the end
                                                                // Ruler ticks: the major tick at ½ is cut out of a full gauge...
        let half_x = 2 + (GAUGE_LENGTH - 4) / 2;
        assert_eq!(img.get_pixel(half_x, gy + 2)[3], 0);
        assert_ne!(img.get_pixel(half_x, mid)[3], 0); // ...but only near the edges
                                                      // Gauge 1: its label is wider than the gauge, so the pane widens —
                                                      // but the gauge keeps its size and is centred. Outline present,
                                                      // interior empty, ticks in ink.
        let x1 = GAUGE_LENGTH + PANE_GAP;
        let w1 = multi_pane_width(&panes[1..2], opts(LabelMode::Above));
        assert!(w1 > GAUGE_LENGTH);
        let g1 = x1 + (w1 - GAUGE_LENGTH) / 2;
        assert_eq!(img.get_pixel(x1, mid)[3], 0); // pane edge is empty
        assert_ne!(img.get_pixel(g1, mid)[3], 0); // gauge left edge
        assert_ne!(img.get_pixel(g1 + GAUGE_LENGTH - 1, mid)[3], 0); // gauge right edge
        assert_eq!(img.get_pixel(g1 + 10, mid)[3], 0);
        assert_ne!(img.get_pixel(g1 + half_x, gy + 2)[3], 0);
        // Number pane: text ink sits in the content area, none in the label gap
        let x2 = x1 + w1 + PANE_GAP;
        assert!(!ink_in(&img, x2, img.width(), lh, content_top));
        assert!(ink_in(&img, x2, img.width(), content_top, PANE_SIZE));

        // Without labels the number is capped in height and vertically centred
        let big = vec![labelled("X", PaneContent::Number("42".into()))];
        let img = decode(
            &generate_multi_pane(
                &big,
                opts(LabelMode::Off),
                &config,
                TrayOverlay::None,
                false,
            )
            .unwrap(),
        );
        let top = (PANE_SIZE - NUMBER_MAX_GLYPH_HEIGHT) / 2;
        assert!(!ink_in(&img, 0, img.width(), 0, top));
        assert!(ink_in(&img, 0, img.width(), top, top + 2));
        assert!(ink_in(
            &img,
            0,
            img.width(),
            top + NUMBER_MAX_GLYPH_HEIGHT - 2,
            top + NUMBER_MAX_GLYPH_HEIGHT
        ));
        assert!(!ink_in(
            &img,
            0,
            img.width(),
            top + NUMBER_MAX_GLYPH_HEIGHT,
            PANE_SIZE
        ));
    }

    #[test]
    #[ignore] // Run with: cargo test write_test_multi_pane -- --ignored
    fn write_test_multi_pane_to_file() {
        use std::fs::File;
        use std::io::Write;

        let config = GraphConfig::windows_linux();
        let variants: [(&str, PaneContent, PaneContent, PaneContent); 3] = [
            (
                "graph",
                PaneContent::Graph((0..30).map(|i| (i * 40) % 900).collect()),
                PaneContent::Graph((0..30).map(|i| (i * 13) % 300).collect()),
                PaneContent::Graph((0..30).map(|i| if i % 5 == 0 { 600 } else { 20 }).collect()),
            ),
            (
                "gauge",
                PaneContent::UsageBar(1.0),
                PaneContent::UsageBar(0.35),
                PaneContent::UsageBar(0.7),
            ),
            (
                "number",
                PaneContent::Number("1.2M".into()),
                PaneContent::Number("24K".into()),
                PaneContent::Number("$0.42".into()),
            ),
        ];
        for (name, a, b, c) in variants {
            let panes = vec![labelled("ALL", a), labelled("CLAU", b), labelled("GPT5", c)];
            for (mode, tag) in [
                (LabelMode::Off, "off"),
                (LabelMode::Beside, "beside"),
                (LabelMode::Above, "above"),
            ] {
                let png = generate_multi_pane(
                    &panes,
                    opts(mode),
                    &config,
                    TrayOverlay::FirewallPending,
                    false,
                )
                .unwrap();
                let path = format!("/tmp/test_tray_multi_pane_{}_{}.png", name, tag);
                File::create(&path).unwrap().write_all(&png).unwrap();
                println!("Wrote {}", path);
            }
        }
    }

    #[test]
    #[ignore] // Run with: cargo test write_test_graph -- --ignored
    fn write_test_graph_to_file() {
        use std::fs::File;
        use std::io::Write;

        let now = Utc::now();
        let mut data = Vec::new();
        for i in 0..26 {
            data.push(DataPoint {
                timestamp: now - Duration::seconds(26 - i),
                total_tokens: (i as u64 * 5) + 10, // Varying values
            });
        }

        // Generate Windows/Linux version with Warning overlay (light mode)
        let config = GraphConfig::windows_linux();
        let png = generate_graph(
            &data,
            &config,
            TrayOverlay::Warning(StatusDotColors::yellow(false)),
            false,
        );
        assert!(png.is_some());
        let png_bytes = png.unwrap();
        let mut file = File::create("/tmp/test_tray_graph.png").unwrap();
        file.write_all(&png_bytes).unwrap();
        println!("Wrote Windows/Linux graph (warning) to /tmp/test_tray_graph.png");

        // Generate macOS version with Warning overlay (dark mode)
        let config_mac = GraphConfig::macos(true);
        let png_mac = generate_graph(
            &data,
            &config_mac,
            TrayOverlay::Warning(StatusDotColors::yellow(true)),
            true,
        );
        assert!(png_mac.is_some());
        let png_bytes_mac = png_mac.unwrap();
        let mut file_mac = File::create("/tmp/test_tray_graph_macos.png").unwrap();
        file_mac.write_all(&png_bytes_mac).unwrap();
        println!("Wrote macOS graph (warning, dark mode) to /tmp/test_tray_graph_macos.png");

        // Generate macOS version with UpdateAvailable overlay
        let png_update = generate_graph(&data, &config_mac, TrayOverlay::UpdateAvailable, true);
        assert!(png_update.is_some());
        let png_bytes_update = png_update.unwrap();
        let mut file_update = File::create("/tmp/test_tray_graph_macos_update.png").unwrap();
        file_update.write_all(&png_bytes_update).unwrap();
        println!("Wrote macOS graph (update) to /tmp/test_tray_graph_macos_update.png");

        // Generate macOS version with no overlay
        let png_none = generate_graph(&data, &config_mac, TrayOverlay::None, true);
        assert!(png_none.is_some());
        let png_bytes_none = png_none.unwrap();
        let mut file_none = File::create("/tmp/test_tray_graph_macos_none.png").unwrap();
        file_none.write_all(&png_bytes_none).unwrap();
        println!("Wrote macOS graph (no overlay) to /tmp/test_tray_graph_macos_none.png");
    }

    #[test]
    fn test_static_icon_with_warning_overlay() {
        let static_icon: &[u8] = include_bytes!("../../icons/32x32.png");
        let png = generate_static_icon_with_overlay(
            static_icon,
            TrayOverlay::Warning(StatusDotColors::yellow(false)),
            false,
        );
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_static_icon_with_firewall_overlay() {
        let static_icon: &[u8] = include_bytes!("../../icons/32x32.png");
        let png =
            generate_static_icon_with_overlay(static_icon, TrayOverlay::FirewallPending, true);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_static_icon_with_update_overlay() {
        let static_icon: &[u8] = include_bytes!("../../icons/32x32.png");
        let png =
            generate_static_icon_with_overlay(static_icon, TrayOverlay::UpdateAvailable, false);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    #[ignore] // Run with: cargo test write_test_static_icon -- --ignored
    fn write_test_static_icon_to_file() {
        use std::fs::File;
        use std::io::Write;

        let static_icon: &[u8] = include_bytes!("../../icons/32x32.png");

        // Plain static icon (no overlay)
        let png = generate_static_icon(static_icon, true).unwrap();
        let mut f = File::create("/tmp/test_static_icon_plain.png").unwrap();
        f.write_all(&png).unwrap();

        // Warning overlay
        let png = generate_static_icon_with_overlay(
            static_icon,
            TrayOverlay::Warning(StatusDotColors::yellow(true)),
            true,
        )
        .unwrap();
        let mut f = File::create("/tmp/test_static_icon_warning.png").unwrap();
        f.write_all(&png).unwrap();

        // Update overlay
        let png =
            generate_static_icon_with_overlay(static_icon, TrayOverlay::UpdateAvailable, true)
                .unwrap();
        let mut f = File::create("/tmp/test_static_icon_update.png").unwrap();
        f.write_all(&png).unwrap();

        // Firewall overlay
        let png =
            generate_static_icon_with_overlay(static_icon, TrayOverlay::FirewallPending, true)
                .unwrap();
        let mut f = File::create("/tmp/test_static_icon_firewall.png").unwrap();
        f.write_all(&png).unwrap();

        println!("Wrote static icon test images to /tmp/test_static_icon_*.png");
    }
}
