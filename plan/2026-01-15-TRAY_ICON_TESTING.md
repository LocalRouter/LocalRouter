# Tray Icon Testing Guide

## Summary

The LocalRouter AI app now has a custom network router icon design with dynamic tray icon states. The implementation is complete and the icons have been generated.

## Icon Design

**Design**: Simple network router/hub
- Central circle (the router/hub)
- Four connection points in cardinal directions (north, south, east, west)
- Monochrome black on transparent background
- Works as a macOS template icon

## Icon Files Created

All icons are in RGBA format (required by Tauri):

- `icon.png` - 16x16 (base icon)
- `32x32.png` - 32x32 (tray icon)
- `128x128.png` - 128x128 (app icon)
- `128x128@2x.png` - 256x256 (app icon @2x)
- `icon.icns` - macOS icon bundle
- `icon.ico` - Windows icon bundle

## Tray Icon States

The tray icon automatically changes based on server activity:

### 1. **Stopped** (Template Mode)
- **Appearance**: Monochrome, system-colored (white in dark mode, black in light mode)
- **Tooltip**: "LocalRouter AI - Server Stopped"
- **When**: Server is not running

### 2. **Running** (Template Mode)
- **Appearance**: Monochrome, system-colored (same as stopped but with different tooltip)
- **Tooltip**: "LocalRouter AI - Server Running"
- **When**: Server is running but idle

### 3. **Active** (Full Color Mode)
- **Appearance**: Full color (black icon, stands out from system color)
- **Tooltip**: "LocalRouter AI - Processing Request"
- **Duration**: 2 seconds per request
- **When**: Server is processing an LLM request (chat, completion, or embedding)

## How to Test

### 1. Check Tray Icon Appearance

Look at the system tray (menu bar on macOS). You should see the network router icon.

**Expected in Light Mode**: Black router icon
**Expected in Dark Mode**: White router icon

### 2. Test Server Start/Stop

1. Click the tray icon
2. Click "Start Server" or "Stop Server"
3. The tooltip should change between "Server Stopped" and "Server Running"

### 3. Test Active State (Most Important!)

When the server receives an API request, the icon should briefly change to full color for 2 seconds.

**To test:**

```bash
# Make sure the server is running first (check tray menu or start it from the UI)

# Send a test chat request
curl -X POST http://127.0.0.1:3625/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": false
  }'
```

**Watch the tray icon** - it should:
1. Change from monochrome to full color (becomes more prominent/solid)
2. Stay in full color for 2 seconds
3. Automatically return to monochrome (template mode)

### 4. Test with Multiple Requests

Send multiple requests in quick succession:

```bash
for i in {1..5}; do
  curl -X POST http://127.0.0.1:3625/v1/chat/completions \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer YOUR_API_KEY" \
    -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Test '$i'"}]}' &
  sleep 0.5
done
```

The icon should blink/pulse as requests are processed.

## Implementation Details

### Code Locations

- **Icon update logic**: `src-tauri/src/ui/tray.rs:705-742` (`update_tray_icon` function)
- **Event emission**:
  - Chat: `src-tauri/src/server/routes/chat.rs:37`
  - Completions: `src-tauri/src/server/routes/completions.rs:29`
  - Embeddings: `src-tauri/src/server/routes/embeddings.rs:19`
- **Event handling**: `src-tauri/src/main.rs:312-318`

### How It Works

1. When the server receives an API request (chat/completion/embedding), it emits an "llm-request" event
2. The main app listens for "llm-request" events
3. On receiving the event, it calls `update_tray_icon(app, "active")`
4. The tray icon switches from template mode (monochrome) to non-template mode (full color)
5. After 2 seconds, it automatically switches back to "running" state (template mode)

### macOS Template Icons

macOS automatically adapts template icons based on:
- System theme (light/dark mode)
- Menu bar appearance
- User accent color settings

Template icons should be:
- Black artwork on transparent background
- Simple, recognizable designs
- Work well at small sizes (16x16 to 32x32)

## Icon Regeneration

If you need to regenerate the icons:

```bash
cd src-tauri/icons
./generate-icons.sh
```

This script:
1. Cleans up old files
2. Creates a high-res 512x512 source icon
3. Generates all required PNG sizes with RGBA format
4. Creates macOS .icns bundle
5. Creates Windows .ico bundle

## Troubleshooting

### Icon doesn't appear
- Make sure the app is running: `ps aux | grep localrouter`
- Check Tauri console for errors

### Icon doesn't change on activity
- Verify the server is running (check tray menu)
- Check that you're using a valid API key
- Look for "llm-request" events in logs
- Verify your request reaches the server (check access logs)

### Icon looks wrong
- On macOS, template icons adapt to system theme automatically
- The "active" state should look more prominent/solid than the running state
- If the design doesn't look right, edit `generate-icons.sh` and regenerate

## Status

✅ Icon design created
✅ All icon files generated (RGBA format)
✅ macOS and Windows icon bundles created
✅ Tray icon state switching implemented
✅ Event emission on all API endpoints implemented
✅ Icon generation script created and updated
⏳ Manual testing required (follow steps above)

## Next Steps

1. **Test the tray icon states** by following the testing steps above
2. **Verify the visual difference** between template and non-template modes
3. **Adjust design if needed** - edit `generate-icons.sh` and regenerate
4. **Consider adding different icon files** for active state (optional enhancement)
