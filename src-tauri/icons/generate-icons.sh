#!/bin/bash
# LocalRouter Icon Generator
# Generates all required icon formats from a simple network router design

set -e

echo "🎨 Generating LocalRouter icons..."

# Check if icon.svg exists
if [ ! -f "icon.svg" ]; then
  echo "❌ Error: icon.svg not found!"
  echo "Please ensure icon.svg exists in the icons directory"
  exit 1
fi

# Clean up existing files
echo "Cleaning up old files..."
rm -f icon.png 32x32.png 32x32-active.png 128x128.png 128x128@2x.png icon.icns icon.ico
rm -rf icon.iconset

# Generate PNG files for Tauri from SVG
echo "Generating PNG files from icon.svg..."

# Main app icon (512x512)
magick icon.svg -resize 512x512 -background none -colorspace sRGB -depth 8 PNG32:icon.png

# 32x32 (for tray icon - use template for macOS)
if [ -f "iconTemplate.svg" ]; then
  echo "Using template icon for tray (macOS compatible)"
  # Convert template to PNG with proper alpha channel and sRGB colorspace
  magick iconTemplate.svg \
    -resize 32x32 \
    -background none \
    -alpha on \
    -colorspace sRGB \
    -define png:color-type=6 \
    -depth 8 \
    PNG32:32x32.png
else
  echo "Template icon not found, using colored icon"
  magick icon.svg -resize 32x32 -background none -colorspace sRGB -depth 8 PNG32:32x32.png
fi

# 32x32 active state (for tray icon)
if [ -f "icon-active.svg" ]; then
  magick icon-active.svg -resize 32x32 -background none -colorspace sRGB -depth 8 PNG32:32x32-active.png
else
  echo "⚠️  icon-active.svg not found, skipping active state icon"
fi

# 128x128 (for app icon)
magick icon.svg -resize 128x128 -background none -colorspace sRGB -depth 8 PNG32:128x128.png

# 128x128@2x (256x256 actual size)
magick icon.svg -resize 256x256 -background none -colorspace sRGB -depth 8 PNG32:128x128@2x.png

# Generate macOS .icns bundle
echo "Generating macOS .icns bundle..."
mkdir icon.iconset

# Create all required sizes for macOS from SVG
magick icon.svg -resize 16x16 -background none icon.iconset/icon_16x16.png
magick icon.svg -resize 32x32 -background none icon.iconset/icon_16x16@2x.png
magick icon.svg -resize 32x32 -background none icon.iconset/icon_32x32.png
magick icon.svg -resize 64x64 -background none icon.iconset/icon_32x32@2x.png
magick icon.svg -resize 128x128 -background none icon.iconset/icon_128x128.png
magick icon.svg -resize 256x256 -background none icon.iconset/icon_128x128@2x.png
magick icon.svg -resize 256x256 -background none icon.iconset/icon_256x256.png
magick icon.svg -resize 512x512 -background none icon.iconset/icon_256x256@2x.png
magick icon.svg -resize 512x512 -background none icon.iconset/icon_512x512.png
magick icon.svg -resize 1024x1024 -background none icon.iconset/icon_512x512@2x.png

# Convert to .icns
iconutil -c icns icon.iconset -o icon.icns

# Generate Windows .ico bundle
echo "Generating Windows .ico bundle..."
magick icon.svg -background none \
  \( -clone 0 -resize 16x16 \) \
  \( -clone 0 -resize 32x32 \) \
  \( -clone 0 -resize 48x48 \) \
  \( -clone 0 -resize 64x64 \) \
  \( -clone 0 -resize 128x128 \) \
  \( -clone 0 -resize 256x256 \) \
  -delete 0 icon.ico

# Cleanup
echo "Cleaning up temporary files..."
rm -rf icon.iconset

echo "✅ Icon generation complete!"
echo ""
echo "Generated files:"
ls -lh icon.png 32x32.png 128x128.png 128x128@2x.png icon.icns icon.ico
echo ""
echo "📝 Icon design: Material Design 3D router with routing arrows"
echo "🎨 Format: Full color with transparency and gradients"
echo "💡 Source: icon.svg (edit this file to change the design)"
