#!/bin/bash
# Build winXP demo and copy to website public directory

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCALROUTER_DIR="$(dirname "$SCRIPT_DIR")"
WINXP_DIR="/Users/matus/dev/winXP"
TARGET_DIR="$LOCALROUTER_DIR/website/public/winxp"

echo "Building winXP..."
cd "$WINXP_DIR"
DISABLE_ESLINT_PLUGIN=true npm run build

echo "Copying build to $TARGET_DIR..."
rm -rf "$TARGET_DIR"
cp -r "$WINXP_DIR/build" "$TARGET_DIR"

echo "Updating index.html metadata..."
cat > "$TARGET_DIR/index.html" << 'EOF'
<!doctype html><html lang="en"><head><meta name="og:image" content="https://i.imgur.com/4miokE2.jpg"/><meta name="og:title" content="LocalRouter - Windows XP Demo"/><meta name="og:description" content="LocalRouter demo running in a Windows XP environment"/><meta name="description" content="LocalRouter demo running in a Windows XP environment"/><meta charset="utf-8"/><meta name="viewport" content="width=device-width,initial-scale=1,shrink-to-fit=no"/><meta name="apple-mobile-web-app-capable" content="yes"/><meta name="theme-color" content="#fff"/><link rel="shortcut icon" href="/winxp/favicon.ico"/><link rel="apple-touch-icon" href="/winxp/favicon.ico"/><link rel="manifest" href="/winxp/manifest.json"/><title>LocalRouter - Windows XP Demo</title>
EOF

# Get the JS and CSS filenames from build
JS_FILE=$(ls "$TARGET_DIR/static/js/" | grep "^main.*\.js$")
CSS_FILE=$(ls "$TARGET_DIR/static/css/" | grep "^main.*\.css$")

cat >> "$TARGET_DIR/index.html" << EOF
<script defer="defer" src="/winxp/static/js/$JS_FILE"></script><link href="/winxp/static/css/$CSS_FILE" rel="stylesheet"></head><body><noscript>You need to enable JavaScript to run this app.</noscript><div id="root"></div></body></html>
EOF

echo "Done! winXP demo deployed to $TARGET_DIR"
