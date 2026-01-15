#!/bin/bash
# LocalRouter AI Icon Generator
# Generates all required icon formats from a simple network router design

set -e

echo "🎨 Generating LocalRouter AI icons..."

# Clean up existing files
echo "Cleaning up old files..."
rm -f icon.png 32x32.png 128x128.png 128x128@2x.png icon.icns icon.ico
rm -rf icon.iconset icon-source.png

# Create high-res source icon (512x512)
echo "Creating source icon (512x512)..."
magick -size 512x512 xc:none \
  -fill black -stroke black -strokewidth 24 \
  -draw "circle 256,256 256,320" \
  -draw "line 256,128 256,48" \
  -draw "circle 256,48 256,80" \
  -draw "line 384,256 464,256" \
  -draw "circle 464,256 464,288" \
  -draw "line 256,384 256,464" \
  -draw "circle 256,464 256,432" \
  -draw "line 128,256 48,256" \
  -draw "circle 48,256 48,288" \
  icon-source.png

# Generate PNG files for Tauri
echo "Generating PNG files..."

# 16x16 base icon (simplified design for small size) - RGBA format required by Tauri
magick -size 16x16 xc:none -colorspace sRGB -type TrueColorAlpha \
  -fill black -stroke black -strokewidth 1 \
  -draw "circle 8,8 8,10" \
  -draw "line 8,4 8,1" \
  -draw "circle 8,1 8,2" \
  -draw "line 12,8 15,8" \
  -draw "circle 15,8 15,9" \
  -draw "line 8,12 8,15" \
  -draw "circle 8,15 8,14" \
  -draw "line 4,8 1,8" \
  -draw "circle 1,8 1,9" \
  -depth 8 PNG32:icon.png

# 32x32 (for tray icon) - RGBA format required by Tauri
magick -size 32x32 xc:none -colorspace sRGB -type TrueColorAlpha \
  -fill black -stroke black -strokewidth 2 \
  -draw "circle 16,16 16,20" \
  -draw "line 16,8 16,3" \
  -draw "circle 16,3 16,5" \
  -draw "line 24,16 29,16" \
  -draw "circle 29,16 29,18" \
  -draw "line 16,24 16,29" \
  -draw "circle 16,29 16,27" \
  -draw "line 8,16 3,16" \
  -draw "circle 3,16 3,18" \
  -depth 8 PNG32:32x32.png

# 128x128 (for app icon) - RGBA format required by Tauri
magick -size 128x128 xc:none -colorspace sRGB -type TrueColorAlpha \
  -fill black -stroke black -strokewidth 6 \
  -draw "circle 64,64 64,80" \
  -draw "line 64,32 64,12" \
  -draw "circle 64,12 64,20" \
  -draw "line 96,64 116,64" \
  -draw "circle 116,64 116,72" \
  -draw "line 64,96 64,116" \
  -draw "circle 64,116 64,108" \
  -draw "line 32,64 12,64" \
  -draw "circle 12,64 12,72" \
  -depth 8 PNG32:128x128.png

# 128x128@2x (256x256 actual size) - RGBA format required by Tauri
magick -size 256x256 xc:none -colorspace sRGB -type TrueColorAlpha \
  -fill black -stroke black -strokewidth 12 \
  -draw "circle 128,128 128,160" \
  -draw "line 128,64 128,24" \
  -draw "circle 128,24 128,40" \
  -draw "line 192,128 232,128" \
  -draw "circle 232,128 232,144" \
  -draw "line 128,192 128,232" \
  -draw "circle 128,232 128,216" \
  -draw "line 64,128 24,128" \
  -draw "circle 24,128 24,144" \
  -depth 8 PNG32:128x128@2x.png

# Generate macOS .icns bundle
echo "Generating macOS .icns bundle..."
mkdir icon.iconset

# Create all required sizes for macOS
magick icon-source.png -resize 16x16 icon.iconset/icon_16x16.png
magick icon-source.png -resize 32x32 icon.iconset/icon_16x16@2x.png
magick icon-source.png -resize 32x32 icon.iconset/icon_32x32.png
magick icon-source.png -resize 64x64 icon.iconset/icon_32x32@2x.png
magick icon-source.png -resize 128x128 icon.iconset/icon_128x128.png
magick icon-source.png -resize 256x256 icon.iconset/icon_128x128@2x.png
magick icon-source.png -resize 256x256 icon.iconset/icon_256x256.png
magick icon-source.png -resize 512x512 icon.iconset/icon_256x256@2x.png
magick icon-source.png -resize 512x512 icon.iconset/icon_512x512.png
magick icon-source.png -resize 1024x1024 icon.iconset/icon_512x512@2x.png

# Convert to .icns
iconutil -c icns icon.iconset -o icon.icns

# Generate Windows .ico bundle
echo "Generating Windows .ico bundle..."
magick icon-source.png \
  \( -clone 0 -resize 16x16 \) \
  \( -clone 0 -resize 32x32 \) \
  \( -clone 0 -resize 48x48 \) \
  \( -clone 0 -resize 64x64 \) \
  \( -clone 0 -resize 128x128 \) \
  \( -clone 0 -resize 256x256 \) \
  -delete 0 icon.ico

# Cleanup
echo "Cleaning up temporary files..."
rm -rf icon.iconset icon-source.png

echo "✅ Icon generation complete!"
echo ""
echo "Generated files:"
ls -lh icon.png 32x32.png 128x128.png 128x128@2x.png icon.icns icon.ico
echo ""
echo "📝 Icon design: Network router with central hub and 4 connection points"
echo "🎨 Format: Black on transparent (template icon for macOS)"
echo "💡 Template mode: macOS adapts color based on system theme"
