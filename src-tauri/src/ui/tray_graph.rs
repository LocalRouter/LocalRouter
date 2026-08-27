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

/// Side length of one graph pane (the classic 32×32 tray icon).
pub const PANE_SIZE: u32 = 32;
const BORDER_WIDTH: u32 = 1;
const INNER_MARGIN: u32 = 2; // Additional margin inside border to prevent overlap
/// Bars per pane (pane width minus border and margin on both sides).
pub const GRAPH_WIDTH: u32 = PANE_SIZE - 2 * (BORDER_WIDTH + INNER_MARGIN); // 26
const GRAPH_HEIGHT: u32 = PANE_SIZE - 2 * (BORDER_WIDTH + INNER_MARGIN); // 26
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
const MAX_BAR_HEIGHT: u32 = GRAPH_HEIGHT;

/// Gap between panes and between a label column and its content.
pub const PANE_GAP: u32 = 3;
/// Width of the outlined usage gauge pane.
pub const USAGE_GAUGE_WIDTH: u32 = 12;
/// Glyph height used for number panes.
pub const NUMBER_GLYPH_HEIGHT: u32 = 11;
/// Horizontal padding on each side of a number pane's text.
const NUMBER_PAD: u32 = 2;

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

/// Bar height in pixels for one value under the given scale reference:
/// fixed 5 tokens/px while the reference fits the pane, otherwise scaled
/// to the reference. Zero values stay zero; non-zero values are at least 1px.
pub fn bar_height(value: u64, scale_reference: u64) -> u32 {
    bar_height_with(value, scale_reference, Some(TOKENS_PER_PIXEL))
}

/// [`bar_height`] with an explicit fixed scale. `units_per_pixel = None`
/// always scales to the reference (used for request counts, whose P95 is
/// far too small for the token fixed scale to show anything).
pub fn bar_height_with(value: u64, scale_reference: u64, units_per_pixel: Option<u64>) -> u32 {
    if value == 0 {
        return 0;
    }
    match units_per_pixel {
        Some(upp) if scale_reference <= upp * MAX_BAR_HEIGHT as u64 => {
            ((value / upp.max(1)) as u32).clamp(1, MAX_BAR_HEIGHT)
        }
        _ => ((value as f64 / scale_reference as f64 * MAX_BAR_HEIGHT as f64) as u32)
            .clamp(1, MAX_BAR_HEIGHT),
    }
}

/// Render one 32×32 pane (rounded frame, bars, optional overlay) into an
/// image. `heights` must have `GRAPH_WIDTH` entries.
fn render_pane(
    heights: &[u32],
    config: &GraphConfig,
    overlay: &TrayOverlay,
    dark_mode: bool,
) -> RgbaImage {
    const WIDTH: u32 = PANE_SIZE;
    const HEIGHT: u32 = PANE_SIZE;

    // Create image buffer
    let mut img = RgbaImage::from_pixel(WIDTH, HEIGHT, config.background);

    let has_overlay = *overlay != TrayOverlay::None;

    // Top border (skip corner regions; wider skip at top-left when overlay present)
    let top_left_border_start = if has_overlay {
        CUTOUT_SIZE
    } else {
        CORNER_RADIUS
    };
    for x in top_left_border_start..(WIDTH - CORNER_RADIUS) {
        img.put_pixel(x, 0, config.foreground);
    }
    // Bottom border (skip corner regions)
    for x in CORNER_RADIUS..(WIDTH - CORNER_RADIUS) {
        img.put_pixel(x, HEIGHT - 1, config.foreground);
    }
    // Left border (skip corner regions; wider skip at top-left when overlay present)
    let left_top_border_start = if has_overlay {
        CUTOUT_SIZE
    } else {
        CORNER_RADIUS
    };
    for y in left_top_border_start..(HEIGHT - CORNER_RADIUS) {
        img.put_pixel(0, y, config.foreground);
    }
    // Right border (skip corner regions)
    for y in CORNER_RADIUS..(HEIGHT - CORNER_RADIUS) {
        img.put_pixel(WIDTH - 1, y, config.foreground);
    }

    // Draw rounded corners (6-pixel radius)
    // Top-left corner: normal convex arc when no overlay, concave cutout when overlay present
    if has_overlay {
        // Quarter-circle notch centered on the overlay icon at (CX, CY).
        // The arc has radius R and sweeps the portion visible in the
        // top-left corner, from the top edge to the left edge.
        // Everything inside the circle is cleared to background.

        // Clear all pixels inside the circle that are in the top-left region
        let max_clear_x = (CUTOUT_CX + CUTOUT_R).min(WIDTH as i32 - 1);
        let max_clear_y = (CUTOUT_CY + CUTOUT_R).min(HEIGHT as i32 - 1);
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
        // pixels that land on screen and outside the box interior
        // (i.e. in the top-left cutout region, up to where the
        // straight borders begin).
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
                && ix < WIDTH as i32
                && iy >= 0
                && iy < HEIGHT as i32
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
    img.put_pixel(WIDTH - 2, 2, config.foreground);
    img.put_pixel(WIDTH - 2, 3, config.foreground);
    img.put_pixel(WIDTH - 2, 4, config.foreground);
    img.put_pixel(WIDTH - 2, 5, config.foreground);
    img.put_pixel(WIDTH - 3, 1, config.foreground);
    img.put_pixel(WIDTH - 4, 1, config.foreground);
    img.put_pixel(WIDTH - 5, 1, config.foreground);
    img.put_pixel(WIDTH - 6, 1, config.foreground);
    img.put_pixel(WIDTH - 3, 2, config.foreground);

    // Bottom-left corner
    img.put_pixel(1, HEIGHT - 3, config.foreground);
    img.put_pixel(1, HEIGHT - 4, config.foreground);
    img.put_pixel(1, HEIGHT - 5, config.foreground);
    img.put_pixel(1, HEIGHT - 6, config.foreground);
    img.put_pixel(2, HEIGHT - 2, config.foreground);
    img.put_pixel(3, HEIGHT - 2, config.foreground);
    img.put_pixel(4, HEIGHT - 2, config.foreground);
    img.put_pixel(5, HEIGHT - 2, config.foreground);
    img.put_pixel(2, HEIGHT - 3, config.foreground);

    // Bottom-right corner
    img.put_pixel(WIDTH - 2, HEIGHT - 3, config.foreground);
    img.put_pixel(WIDTH - 2, HEIGHT - 4, config.foreground);
    img.put_pixel(WIDTH - 2, HEIGHT - 5, config.foreground);
    img.put_pixel(WIDTH - 2, HEIGHT - 6, config.foreground);
    img.put_pixel(WIDTH - 3, HEIGHT - 2, config.foreground);
    img.put_pixel(WIDTH - 4, HEIGHT - 2, config.foreground);
    img.put_pixel(WIDTH - 5, HEIGHT - 2, config.foreground);
    img.put_pixel(WIDTH - 6, HEIGHT - 2, config.foreground);
    img.put_pixel(WIDTH - 3, HEIGHT - 3, config.foreground);

    // Draw logo FIRST as a background watermark (bars will be drawn on top)
    // TODO: Temporarily disabled logo overlay
    // draw_logo(&mut img, config.foreground);

    // Draw bars (each bar is exactly 1 pixel wide, inside the border)
    for (i, &h) in heights.iter().take(GRAPH_WIDTH as usize).enumerate() {
        if h == 0 {
            continue;
        }
        let bar_height = h.min(MAX_BAR_HEIGHT);

        // Calculate x position (offset by border + margin)
        let x = GRAPH_OFFSET_X + i as u32;

        // Draw filled vertical bar from bottom up (1 pixel wide, with margin from border)
        let start_y = HEIGHT - GRAPH_OFFSET_Y - bar_height;
        let end_y = HEIGHT - GRAPH_OFFSET_Y;
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

/// Generate a 32x32 PNG sparkline graph from data points
///
/// Creates a filled vertical bar chart showing token usage over time.
/// Automatically normalizes values to fit the 32px height.
/// Always renders exactly 26 bars (one per pixel width), padding with zeros if needed.
/// Includes a 1-pixel border around the graph.
/// When an overlay is present, the top-left corner is carved out with a concave arc
/// and the overlay icon is drawn in that area.
///
/// # Arguments
/// * `data_points` - Time-series data points (sorted by timestamp, oldest to newest)
/// * `config` - Rendering configuration (colors, template mode)
/// * `overlay` - Overlay icon for the top-left corner
///
/// # Returns
/// PNG-encoded image as bytes, or None if generation fails
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
    let img = render_pane(&heights, config, &overlay, dark_mode);
    encode_png(&img)
}

/// What one panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneContent {
    /// Sparkline values, oldest first (padded/truncated to `GRAPH_WIDTH`).
    Graph(Vec<u64>),
    /// Outlined vertical gauge, fill `0.0..=1.0`.
    UsageBar(f32),
    /// Text drawn horizontally in the bitmap font (e.g. `24K`, `$0.42`).
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
    /// Stacked label drawn left of the content (already normalized, ≤ 4 chars).
    pub label: Option<String>,
    pub content: PaneContent,
}

/// Icon-wide rendering options.
#[derive(Debug, Clone, Copy)]
pub struct MultiPaneOptions {
    /// Draw the label column (same width for every pane).
    pub show_labels: bool,
    /// Fixed scale for graph bars (`Some(TOKENS_PER_PIXEL)` for tokens);
    /// `None` always auto-scales to the shared P95.
    pub units_per_pixel: Option<u64>,
}

/// Pixel width of a pane's content.
fn content_width(content: &PaneContent) -> u32 {
    match content {
        PaneContent::Graph(_) => PANE_SIZE,
        PaneContent::UsageBar(_) => USAGE_GAUGE_WIDTH,
        PaneContent::Number(text) => {
            crate::ui::tray_font::FontMetrics::for_height(NUMBER_GLYPH_HEIGHT).text_width(text)
                + 2 * NUMBER_PAD
        }
    }
}

/// Width of the shared label column (widest label wins), or 0 when labels
/// are off or every label is empty.
fn label_column_width(panes: &[PaneSpec], options: MultiPaneOptions) -> u32 {
    if !options.show_labels {
        return 0;
    }
    panes
        .iter()
        .filter_map(|p| p.label.as_deref())
        .map(crate::ui::tray_font::vertical_label_width)
        .max()
        .unwrap_or(0)
}

/// Total icon width for `panes` under `options`.
pub fn multi_pane_width(panes: &[PaneSpec], options: MultiPaneOptions) -> u32 {
    if panes.is_empty() {
        return 0;
    }
    let label_w = label_column_width(panes, options);
    let label_w = if label_w > 0 { label_w + PANE_GAP } else { 0 };
    let content: u32 = panes.iter().map(|p| content_width(&p.content)).sum();
    content + panes.len() as u32 * label_w + (panes.len() as u32 - 1) * PANE_GAP
}

/// Draw the outlined usage gauge with its left edge at `x`.
fn draw_usage_gauge(img: &mut RgbaImage, x: u32, fill: f32, config: &GraphConfig) {
    let h = img.height();
    let w = USAGE_GAUGE_WIDTH;
    // 1px outline with clipped corners
    for dx in 1..w - 1 {
        img.put_pixel(x + dx, 0, config.foreground);
        img.put_pixel(x + dx, h - 1, config.foreground);
    }
    for y in 1..h - 1 {
        img.put_pixel(x, y, config.foreground);
        img.put_pixel(x + w - 1, y, config.foreground);
    }
    // Fill from the bottom, inset 2px from the outline
    let inner_h = h - 4;
    let fill = fill.clamp(0.0, 1.0);
    let filled = if fill <= 0.0 {
        0
    } else {
        ((fill * inner_h as f32).round() as u32).clamp(1, inner_h)
    };
    for y in (h - 2 - filled)..(h - 2) {
        for dx in 2..w - 2 {
            img.put_pixel(x + dx, y, config.foreground);
        }
    }
}

/// Draw a number pane's text with the pane's left edge at `x`.
fn draw_number(img: &mut RgbaImage, x: u32, text: &str, config: &GraphConfig) {
    let m = crate::ui::tray_font::FontMetrics::for_height(NUMBER_GLYPH_HEIGHT);
    let y = (img.height() - m.glyph_height) / 2;
    crate::ui::tray_font::draw_text(img, x + NUMBER_PAD, y, text, m, config.foreground);
}

/// Generate a wide tray icon with one labelled pane per item.
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
    let label_w = label_column_width(panes, options);

    let series: Vec<Option<Vec<u64>>> = panes
        .iter()
        .map(|p| match &p.content {
            PaneContent::Graph(bars) => Some(normalize_series(bars)),
            _ => None,
        })
        .collect();
    let scale = scale_reference(series.iter().flatten().flatten());

    let mut img = RgbaImage::from_pixel(width, PANE_SIZE, config.background);
    let mut x = 0;
    let mut overlay_pending = overlay;

    for (pane, values) in panes.iter().zip(series.iter()) {
        if x > 0 {
            x += PANE_GAP;
        }
        if label_w > 0 {
            if let Some(label) = &pane.label {
                crate::ui::tray_font::draw_vertical_label(
                    &mut img,
                    x,
                    label_w,
                    label,
                    config.foreground,
                );
            }
            x += label_w + PANE_GAP;
        }
        match &pane.content {
            PaneContent::Graph(_) => {
                let values = values.as_ref().expect("graph pane has a series");
                let heights: Vec<u32> = values
                    .iter()
                    .map(|&v| bar_height_with(v, scale, options.units_per_pixel))
                    .collect();
                let pane_overlay = std::mem::replace(&mut overlay_pending, TrayOverlay::None);
                let pane_img = render_pane(&heights, config, &pane_overlay, dark_mode);
                image::imageops::replace(&mut img, &pane_img, x as i64, 0);
            }
            PaneContent::UsageBar(fill) => draw_usage_gauge(&mut img, x, *fill, config),
            PaneContent::Number(text) => draw_number(&mut img, x, text, config),
        }
        x += content_width(&pane.content);
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

    fn opts(labels: bool) -> MultiPaneOptions {
        MultiPaneOptions {
            show_labels: labels,
            units_per_pixel: Some(TOKENS_PER_PIXEL),
        }
    }

    fn graph(bars: Vec<u64>) -> PaneSpec {
        PaneSpec {
            label: None,
            content: PaneContent::Graph(bars),
        }
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
        assert_eq!(n[0], 14);
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
        assert_eq!(bar_height(1000, 1000), 26); // auto: fits reference
        assert_eq!(bar_height(500, 1000), 13);
        // Requests: no fixed scale, small values still fill the pane
        assert_eq!(bar_height_with(2, 2, None), 26);
        assert_eq!(bar_height_with(1, 2, None), 13);
    }

    #[test]
    fn single_pane_matches_generate_graph() {
        let now = Utc::now();
        let data: Vec<DataPoint> = (0..26)
            .map(|i| DataPoint {
                timestamp: now - Duration::seconds(26 - i),
                total_tokens: (i as u64 * 7) % 90,
            })
            .collect();
        let config = GraphConfig::macos(false);
        let single = generate_graph(&data, &config, TrayOverlay::UpdateAvailable, false).unwrap();
        let bars: Vec<u64> = data.iter().map(|d| d.total_tokens).collect();
        let multi = generate_multi_pane(
            &[graph(bars)],
            opts(false),
            &config,
            TrayOverlay::UpdateAvailable,
            false,
        )
        .unwrap();
        assert_eq!(decode(&single).as_raw(), decode(&multi).as_raw());
    }

    #[test]
    fn multi_pane_width_follows_content_and_gaps() {
        let config = GraphConfig::macos(false);
        let panes = vec![graph(vec![]), graph(vec![]), graph(vec![])];
        let png =
            generate_multi_pane(&panes, opts(false), &config, TrayOverlay::None, false).unwrap();
        assert_eq!(decode(&png).dimensions(), (3 * 32 + 2 * PANE_GAP, 32));
        // Labels: shared column = widest label + gap, per pane
        let labelled = vec![
            PaneSpec {
                label: Some("ALL".into()),
                content: PaneContent::Graph(vec![]),
            },
            PaneSpec {
                label: Some("CLAU".into()),
                content: PaneContent::UsageBar(0.5),
            },
            PaneSpec {
                label: Some("X".into()),
                content: PaneContent::Number("24K".into()),
            },
        ];
        // Shared column = the widest label ("X" is a single 14px-tall glyph)
        let label_w = ["ALL", "CLAU", "X"]
            .iter()
            .map(|l| crate::ui::tray_font::vertical_label_width(l))
            .max()
            .unwrap();
        assert_eq!(label_w, crate::ui::tray_font::vertical_label_width("X"));
        let number_w = crate::ui::tray_font::FontMetrics::for_height(NUMBER_GLYPH_HEIGHT)
            .text_width("24K")
            + 4;
        let expected = 3 * (label_w + PANE_GAP) + 32 + USAGE_GAUGE_WIDTH + number_w + 2 * PANE_GAP;
        assert_eq!(multi_pane_width(&labelled, opts(true)), expected);
        let png =
            generate_multi_pane(&labelled, opts(true), &config, TrayOverlay::None, false).unwrap();
        assert_eq!(decode(&png).dimensions(), (expected, 32));
        // Labels off → no label columns even though labels are set
        assert_eq!(
            multi_pane_width(&labelled, opts(false)),
            32 + USAGE_GAUGE_WIDTH + number_w + 2 * PANE_GAP
        );
        assert!(generate_multi_pane(&[], opts(true), &config, TrayOverlay::None, false).is_none());
    }

    #[test]
    fn multi_pane_shares_one_scale() {
        let config = GraphConfig::macos(false);
        // Pane A peaks at 1000 tokens, pane B at 500 → B's bar is half of A's
        let panes = vec![graph(vec![1000]), graph(vec![500])];
        let img = decode(
            &generate_multi_pane(&panes, opts(false), &config, TrayOverlay::None, false).unwrap(),
        );
        let col_height = |x: u32| (3..29).filter(|&y| img.get_pixel(x, y)[3] != 0).count();
        let last_bar_x = 3 + GRAPH_WIDTH - 1;
        assert_eq!(col_height(last_bar_x), 26);
        assert_eq!(col_height(32 + PANE_GAP + last_bar_x), 13);
    }

    #[test]
    fn overlay_only_on_first_graph_pane() {
        let config = GraphConfig::macos(false);
        let panes = vec![graph(vec![]), graph(vec![])];
        let img = decode(
            &generate_multi_pane(
                &panes,
                opts(false),
                &config,
                TrayOverlay::UpdateAvailable,
                false,
            )
            .unwrap(),
        );
        // Down-arrow stem pixel at (6,3) is set in pane 0 but not pane 1
        assert_ne!(img.get_pixel(6, 3)[3], 0);
        let x1 = 32 + PANE_GAP;
        assert_eq!(img.get_pixel(x1 + 6, 3)[3], 0);
        // Pane 1 keeps the convex corner (top border starts at x=6)
        assert_ne!(img.get_pixel(x1 + 6, 0)[3], 0);
        // The gap column between panes is empty
        assert!((0..32).all(|y| img.get_pixel(32, y)[3] == 0));
    }

    #[test]
    fn usage_gauge_and_number_panes_draw() {
        let config = GraphConfig::macos(false);
        let panes = vec![
            PaneSpec {
                label: None,
                content: PaneContent::UsageBar(1.0),
            },
            PaneSpec {
                label: None,
                content: PaneContent::UsageBar(0.0),
            },
            PaneSpec {
                label: None,
                content: PaneContent::Number("1.2M".into()),
            },
        ];
        let img = decode(
            &generate_multi_pane(&panes, opts(false), &config, TrayOverlay::None, false).unwrap(),
        );
        // Gauge 0: outline on left/right edges, fully filled interior
        assert_ne!(img.get_pixel(0, 10)[3], 0);
        assert_ne!(img.get_pixel(USAGE_GAUGE_WIDTH - 1, 10)[3], 0);
        assert_ne!(img.get_pixel(5, 2)[3], 0);
        assert_ne!(img.get_pixel(5, 29)[3], 0);
        // Gauge 1: outline present, interior empty
        let x1 = USAGE_GAUGE_WIDTH + PANE_GAP;
        assert_ne!(img.get_pixel(x1, 10)[3], 0);
        assert_eq!(img.get_pixel(x1 + 5, 16)[3], 0);
        // Number pane has ink vertically centred
        let x2 = x1 + USAGE_GAUGE_WIDTH + PANE_GAP;
        let ink_rows: Vec<u32> = (0..32)
            .filter(|&y| (x2..img.width()).any(|x| img.get_pixel(x, y)[3] != 0))
            .collect();
        assert_eq!(
            ink_rows.first().copied(),
            Some((32 - NUMBER_GLYPH_HEIGHT) / 2)
        );
        assert_eq!(ink_rows.len() as u32, NUMBER_GLYPH_HEIGHT);
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
                PaneContent::Graph((0..26).map(|i| (i * 40) % 900).collect()),
                PaneContent::Graph((0..26).map(|i| (i * 13) % 300).collect()),
                PaneContent::Graph((0..26).map(|i| if i % 5 == 0 { 600 } else { 20 }).collect()),
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
            let panes = vec![
                PaneSpec {
                    label: Some("ALL".into()),
                    content: a,
                },
                PaneSpec {
                    label: Some("CLAU".into()),
                    content: b,
                },
                PaneSpec {
                    label: Some("GPT5".into()),
                    content: c,
                },
            ];
            for labels in [true, false] {
                let png = generate_multi_pane(
                    &panes,
                    opts(labels),
                    &config,
                    TrayOverlay::FirewallPending,
                    false,
                )
                .unwrap();
                let path = format!(
                    "/tmp/test_tray_multi_pane_{}_{}.png",
                    name,
                    if labels { "labels" } else { "nolabels" }
                );
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
