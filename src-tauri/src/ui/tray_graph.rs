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
    /// Create config for macOS (non-template mode with visible colors)
    /// Uses transparent background with white foreground
    pub fn macos() -> Self {
        Self {
            foreground: Rgba([255, 255, 255, 255]), // White
            background: Rgba([0, 0, 0, 0]),         // Transparent
            template_mode: false,
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

/// Status dot colors
pub struct StatusDotColors;

impl StatusDotColors {
    /// Green color for healthy status (#22c55e)
    pub fn green() -> Rgba<u8> {
        Rgba([34, 197, 94, 255])
    }

    /// Yellow color for degraded/warning status (#eab308)
    pub fn yellow() -> Rgba<u8> {
        Rgba([234, 179, 8, 255])
    }

    /// Red color for unhealthy/down status (#ef4444)
    pub fn red() -> Rgba<u8> {
        Rgba([239, 68, 68, 255])
    }

    /// Get color for aggregate health status
    pub fn for_status(status: AggregateHealthStatus) -> Rgba<u8> {
        match status {
            AggregateHealthStatus::Green => Self::green(),
            AggregateHealthStatus::Yellow => Self::yellow(),
            AggregateHealthStatus::Red => Self::red(),
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

/// Generate a 32x32 PNG sparkline graph from data points
///
/// Creates a filled vertical bar chart showing token usage over time.
/// Automatically normalizes values to fit the 32px height.
/// Always renders exactly 32 bars (one per pixel width), padding with zeros if needed.
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
) -> Option<Vec<u8>> {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 32;
    const BORDER_WIDTH: u32 = 1;
    const INNER_MARGIN: u32 = 2; // Additional margin inside border to prevent overlap
    const GRAPH_WIDTH: u32 = WIDTH - 2 * (BORDER_WIDTH + INNER_MARGIN); // 26 pixels for graph
    const GRAPH_HEIGHT: u32 = HEIGHT - 2 * (BORDER_WIDTH + INNER_MARGIN); // 26 pixels for graph
    const GRAPH_OFFSET_X: u32 = BORDER_WIDTH + INNER_MARGIN; // Start at x=3
    const GRAPH_OFFSET_Y: u32 = BORDER_WIDTH + INNER_MARGIN; // Start at y=3

    // Create image buffer
    let mut img = RgbaImage::from_pixel(WIDTH, HEIGHT, config.background);

    // Draw border with rounded corners (1 pixel around the entire image)
    // Using a 6-pixel radius for smoother corners
    const CORNER_RADIUS: u32 = 6;

    // Cutout: quarter-circle notch centered on the overlay icon center (6, 6)
    // with radius 9. The arc meets the top edge at x≈13 and the left edge at y≈13.
    const CUTOUT_CX: i32 = 6;
    const CUTOUT_CY: i32 = 6;
    const CUTOUT_R: i32 = 9;
    const CUTOUT_R_SQ: i32 = CUTOUT_R * CUTOUT_R; // 81
                                                  // Where the arc intersects the image edges — borders start here
    const CUTOUT_SIZE: u32 = 13;

    let has_overlay = overlay != TrayOverlay::None;

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

    // Ensure we have exactly 26 data points (graph width without border and margin)
    let mut normalized_points: Vec<u64> = Vec::with_capacity(GRAPH_WIDTH as usize);

    if data_points.is_empty() {
        // All zeros
        normalized_points.resize(GRAPH_WIDTH as usize, 0);
    } else if data_points.len() >= GRAPH_WIDTH as usize {
        // Take the last 30 points (most recent)
        for point in data_points.iter().rev().take(GRAPH_WIDTH as usize).rev() {
            normalized_points.push(point.total_tokens);
        }
    } else {
        // Pad left with zeros, then add actual data
        let padding = GRAPH_WIDTH as usize - data_points.len();
        normalized_points.resize(padding, 0);
        for point in data_points {
            normalized_points.push(point.total_tokens);
        }
    }

    // Check if we have any data to draw bars
    let has_data = !normalized_points.iter().all(|&t| t == 0);

    // Draw logo FIRST as a background watermark (bars will be drawn on top)
    // TODO: Temporarily disabled logo overlay
    // draw_logo(&mut img, config.foreground);

    // Only draw bars if we have data (drawn on top of LR letters)
    if has_data {
        // Scaling configuration: 1 pixel = 5 tokens
        const TOKENS_PER_PIXEL: u64 = 5;
        const MAX_BAR_HEIGHT: u32 = GRAPH_HEIGHT; // Full graph height (28 pixels)
        const MAX_FIXED_SCALE_TOKENS: u64 = TOKENS_PER_PIXEL * MAX_BAR_HEIGHT as u64; // 5 * 28 = 140 tokens

        // Calculate P95 (95th percentile) to avoid outliers affecting the scale
        let mut sorted_points: Vec<u64> = normalized_points
            .iter()
            .copied()
            .filter(|&t| t > 0)
            .collect();
        sorted_points.sort_unstable();

        let scale_reference = if sorted_points.is_empty() {
            1
        } else {
            // Use P95 for scaling to prevent outliers from squashing the graph
            let p95_index =
                ((sorted_points.len() as f64 * 0.95).ceil() as usize).min(sorted_points.len() - 1);
            sorted_points[p95_index].max(1)
        };

        // Determine if we use fixed scale or auto-scale based on P95
        let use_fixed_scale = scale_reference <= MAX_FIXED_SCALE_TOKENS;

        // Draw bars (each bar is exactly 1 pixel wide, inside the border)
        for (i, &token_count) in normalized_points.iter().enumerate() {
            // Skip empty data points
            if token_count == 0 {
                continue;
            }

            // Calculate bar height based on scaling mode
            let bar_height = if use_fixed_scale {
                // Fixed scale: 1 pixel = 5 tokens
                // Example: 50 tokens = 10px, 145 tokens = 29px
                let height = (token_count / TOKENS_PER_PIXEL) as u32;
                height.clamp(1, MAX_BAR_HEIGHT)
            } else {
                // Auto-scale: fit to P95 value (outliers can extend beyond max height)
                // Using P95 prevents outliers from squashing all other bars
                let normalized =
                    (token_count as f64 / scale_reference as f64 * MAX_BAR_HEIGHT as f64) as u32;
                normalized.clamp(1, MAX_BAR_HEIGHT)
            };

            // Calculate x position (offset by border + margin)
            let x = GRAPH_OFFSET_X + i as u32;

            // Draw filled vertical bar from bottom up (1 pixel wide, with margin from border)
            // Start from bottom margin and go up by bar_height
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
    }

    // Draw overlay icon in the carved-out top-left corner
    match &overlay {
        TrayOverlay::None => {}
        TrayOverlay::Warning(color) => {
            draw_exclamation_mark(&mut img, *color);
        }
        TrayOverlay::UpdateAvailable => {
            draw_down_arrow(&mut img, config.foreground);
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
pub fn platform_graph_config() -> GraphConfig {
    GraphConfig::macos()
}

/// Get platform-specific graph config
#[cfg(not(target_os = "macos"))]
pub fn platform_graph_config() -> GraphConfig {
    GraphConfig::windows_linux()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_generate_empty_graph() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(&[], &config, TrayOverlay::None);
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
        let png = generate_graph(&data, &config, TrayOverlay::None);
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

        let png = generate_graph(&data, &config, TrayOverlay::None);
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

        let png = generate_graph(&data, &config, TrayOverlay::None);
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

        let png = generate_graph(&data, &config, TrayOverlay::None);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_platform_configs() {
        let _config = platform_graph_config();
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

        let png = generate_graph(&data, &config, TrayOverlay::None);
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

        let png = generate_graph(&data, &config, TrayOverlay::None);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());

        // All bars should have the same height since token counts are consistent
    }

    #[test]
    fn test_overlay_none() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(&[], &config, TrayOverlay::None);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_overlay_warning_yellow() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(
            &[],
            &config,
            TrayOverlay::Warning(StatusDotColors::yellow()),
        );
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_overlay_warning_red() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(&[], &config, TrayOverlay::Warning(StatusDotColors::red()));
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_overlay_update_available() {
        let config = GraphConfig::macos_template();
        let png = generate_graph(&[], &config, TrayOverlay::UpdateAvailable);
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
    }

    #[test]
    fn test_status_dot_colors() {
        assert_eq!(StatusDotColors::green(), Rgba([34, 197, 94, 255]));
        assert_eq!(StatusDotColors::yellow(), Rgba([234, 179, 8, 255]));
        assert_eq!(StatusDotColors::red(), Rgba([239, 68, 68, 255]));
    }

    #[test]
    fn test_graph_with_lr_letters() {
        // Test that graph renders with LR letters overlay
        let config = GraphConfig::macos_template();
        let data = vec![DataPoint {
            timestamp: Utc::now(),
            total_tokens: 100,
        }];
        let png = generate_graph(&data, &config, TrayOverlay::None);
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
            TrayOverlay::Warning(StatusDotColors::yellow()),
        );
        assert!(png.is_some());
        assert!(!png.unwrap().is_empty());
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

        // Generate Windows/Linux version with Warning overlay
        let config = GraphConfig::windows_linux();
        let png = generate_graph(
            &data,
            &config,
            TrayOverlay::Warning(StatusDotColors::yellow()),
        );
        assert!(png.is_some());
        let png_bytes = png.unwrap();
        let mut file = File::create("/tmp/test_tray_graph.png").unwrap();
        file.write_all(&png_bytes).unwrap();
        println!("Wrote Windows/Linux graph (warning) to /tmp/test_tray_graph.png");

        // Generate macOS version with Warning overlay
        let config_mac = GraphConfig::macos();
        let png_mac = generate_graph(
            &data,
            &config_mac,
            TrayOverlay::Warning(StatusDotColors::yellow()),
        );
        assert!(png_mac.is_some());
        let png_bytes_mac = png_mac.unwrap();
        let mut file_mac = File::create("/tmp/test_tray_graph_macos.png").unwrap();
        file_mac.write_all(&png_bytes_mac).unwrap();
        println!("Wrote macOS graph (warning) to /tmp/test_tray_graph_macos.png");

        // Generate macOS version with UpdateAvailable overlay
        let png_update = generate_graph(&data, &config_mac, TrayOverlay::UpdateAvailable);
        assert!(png_update.is_some());
        let png_bytes_update = png_update.unwrap();
        let mut file_update = File::create("/tmp/test_tray_graph_macos_update.png").unwrap();
        file_update.write_all(&png_bytes_update).unwrap();
        println!("Wrote macOS graph (update) to /tmp/test_tray_graph_macos_update.png");

        // Generate macOS version with no overlay
        let png_none = generate_graph(&data, &config_mac, TrayOverlay::None);
        assert!(png_none.is_some());
        let png_bytes_none = png_none.unwrap();
        let mut file_none = File::create("/tmp/test_tray_graph_macos_none.png").unwrap();
        file_none.write_all(&png_bytes_none).unwrap();
        println!("Wrote macOS graph (no overlay) to /tmp/test_tray_graph_macos_none.png");
    }
}
