//! Tray icon graph generation
//!
//! Generates 32x32 PNG sparkline graphs showing token usage over time.

use chrono::{DateTime, Utc};
use image::{codecs::png::PngEncoder, ImageEncoder, Rgba, RgbaImage};
use std::io::Cursor;
use tracing::error;

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
    /// Create config for macOS template mode (adaptive to menu bar theme)
    pub fn macos_template() -> Self {
        Self {
            foreground: Rgba([255, 255, 255, 255]), // White (inverted by macOS)
            background: Rgba([0, 0, 0, 0]),         // Transparent
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
}

/// Generate a 32x32 PNG sparkline graph from data points
///
/// Creates a filled vertical bar chart showing token usage over time.
/// Automatically normalizes values to fit the 32px height.
/// Always renders exactly 32 bars (one per pixel width), padding with zeros if needed.
/// Includes a 1-pixel border around the graph.
///
/// # Arguments
/// * `data_points` - Time-series data points (sorted by timestamp, oldest to newest)
/// * `config` - Rendering configuration (colors, template mode)
///
/// # Returns
/// PNG-encoded image as bytes, or None if generation fails
pub fn generate_graph(data_points: &[DataPoint], config: &GraphConfig) -> Option<Vec<u8>> {
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

    // Top border (skip corner regions)
    for x in CORNER_RADIUS..(WIDTH - CORNER_RADIUS) {
        img.put_pixel(x, 0, config.foreground);
    }
    // Bottom border (skip corner regions)
    for x in CORNER_RADIUS..(WIDTH - CORNER_RADIUS) {
        img.put_pixel(x, HEIGHT - 1, config.foreground);
    }
    // Left border (skip corner regions)
    for y in CORNER_RADIUS..(HEIGHT - CORNER_RADIUS) {
        img.put_pixel(0, y, config.foreground);
    }
    // Right border (skip corner regions)
    for y in CORNER_RADIUS..(HEIGHT - CORNER_RADIUS) {
        img.put_pixel(WIDTH - 1, y, config.foreground);
    }

    // Draw rounded corners (6-pixel radius)
    // Top-left corner
    img.put_pixel(1, 2, config.foreground);
    img.put_pixel(1, 3, config.foreground);
    img.put_pixel(1, 4, config.foreground);
    img.put_pixel(1, 5, config.foreground);
    img.put_pixel(2, 1, config.foreground);
    img.put_pixel(3, 1, config.foreground);
    img.put_pixel(4, 1, config.foreground);
    img.put_pixel(5, 1, config.foreground);
    img.put_pixel(2, 2, config.foreground);

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

    // Handle all-zero data
    if normalized_points.iter().all(|&t| t == 0) {
        return encode_png(&img);
    }

    // Scaling configuration: 1 pixel = 5 tokens
    const TOKENS_PER_PIXEL: u64 = 5;
    const MAX_BAR_HEIGHT: u32 = GRAPH_HEIGHT; // Full graph height (28 pixels)
    const MAX_FIXED_SCALE_TOKENS: u64 = TOKENS_PER_PIXEL * MAX_BAR_HEIGHT as u64; // 5 * 28 = 140 tokens

    // Find max value to determine scaling mode
    let max_tokens = *normalized_points.iter().max().unwrap_or(&1);

    // Avoid division by zero
    let max_tokens = if max_tokens == 0 { 1 } else { max_tokens };

    // Determine if we use fixed scale or auto-scale
    let use_fixed_scale = max_tokens <= MAX_FIXED_SCALE_TOKENS;

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
            height.min(MAX_BAR_HEIGHT).max(1)
        } else {
            // Auto-scale: fit to max value
            // When max > 145 tokens, scale proportionally to fit
            let normalized = (token_count as f64 / max_tokens as f64 * MAX_BAR_HEIGHT as f64) as u32;
            normalized.min(MAX_BAR_HEIGHT).max(1)
        };

        // Calculate x position (offset by border + margin)
        let x = GRAPH_OFFSET_X + i as u32;

        // Draw filled vertical bar from bottom up (1 pixel wide, with margin from border)
        // Start from bottom margin and go up by bar_height
        let start_y = HEIGHT - GRAPH_OFFSET_Y - bar_height;
        let end_y = HEIGHT - GRAPH_OFFSET_Y;
        for y in start_y..end_y {
            img.put_pixel(x, y, config.foreground);
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
    GraphConfig::macos_template()
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
        let png = generate_graph(&[], &config);
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
        let png = generate_graph(&data, &config);
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

        let png = generate_graph(&data, &config);
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

        let png = generate_graph(&data, &config);
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

        let png = generate_graph(&data, &config);
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
}
