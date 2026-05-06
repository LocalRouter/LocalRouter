# Running LocalRouter in Docker

LocalRouter ships a container image that wraps the official Linux AppImage:

```
ghcr.io/localrouter/localrouter:latest          # most recent stable release
ghcr.io/localrouter/localrouter:<version>       # e.g. 0.1.0
```

> **Linux hosts only.** The Tauri webview needs an X11 display server. Docker
> Desktop on macOS and Windows does not expose `/tmp/.X11-unix`, so the GUI
> cannot start there. The headless API-server-only mode is tracked separately
> and would let the image work on any host — until that lands, this image is
> intended for Linux desktops and Linux homelab boxes with X forwarding.

## Quick start

```bash
# Allow X11 connections from your local user only
xhost +SI:localuser:$(whoami)

# Persistent config / secrets / logs
mkdir -p ./localrouter-home

docker run --rm -it \
  -v /tmp/.X11-unix:/tmp/.X11-unix \
  -v "$(pwd)/localrouter-home:/home/app" \
  -p 3625:3625 \
  -e DISPLAY="$DISPLAY" \
  --device /dev/dri \
  ghcr.io/localrouter/localrouter:latest
```

The API is now reachable from the host at `http://localhost:3625`. The
container's entrypoint writes `server.host: 0.0.0.0` to `settings.yaml` on
first boot so the port is reachable across the container network boundary
without manual editing.

## Why these flags

| Flag | Why |
| ---- | --- |
| `-v /tmp/.X11-unix:/tmp/.X11-unix` | Shares the host's X11 socket so the webview can render. |
| `-e DISPLAY=$DISPLAY` | Tells GTK which display to connect to. |
| `--device /dev/dri` | GPU access for hardware-accelerated rendering. Some systems also need `--group-add` for the `video` and `render` groups — see "GPU access" below. |
| `-v ./localrouter-home:/home/app` | Persists `~/.localrouter/` (config, secrets, logs) across restarts. |
| `-p 3625:3625` | Exposes the OpenAI-compatible API. |
| `xhost +SI:localuser:$(whoami)` | Authorizes connections from your local user — narrower than `xhost +local:docker`. |

## GPU access (if you see GL errors)

```bash
VIDEO_GID=$(getent group video | cut -d: -f3)
RENDER_GID=$(getent group render | cut -d: -f3)

docker run --rm -it \
  -v /tmp/.X11-unix:/tmp/.X11-unix \
  -v "$(pwd)/localrouter-home:/home/app" \
  -p 3625:3625 \
  -e DISPLAY="$DISPLAY" \
  --device /dev/dri \
  --group-add "$VIDEO_GID" \
  --group-add "$RENDER_GID" \
  ghcr.io/localrouter/localrouter:latest
```

## File locations inside the container

- Config: `/home/app/.localrouter/settings.yaml`
- Secrets: `/home/app/.localrouter/secrets.json` (file-based; the container
  has no DBus / Secret Service, so `LOCALROUTER_KEYCHAIN=file` is set
  automatically)
- Logs: `/home/app/.localrouter/logs/`

If you bind-mount the host user's `~/.localrouter` directly, do it with
matching uid/gid:

```bash
docker run --rm -it \
  -u $(id -u):$(id -g) \
  -e HOME="$HOME" \
  -v "$HOME":"$HOME" \
  -v /tmp/.X11-unix:/tmp/.X11-unix \
  -p 3625:3625 \
  -e DISPLAY="$DISPLAY" \
  --device /dev/dri \
  ghcr.io/localrouter/localrouter:latest
```

## Building the image yourself

```bash
docker build -t local-router .

# Pin to a specific release:
docker build \
  --build-arg APPIMAGE_URL=https://github.com/LocalRouter/LocalRouter/releases/download/v0.1.0/LocalRouter_0.1.0_amd64.AppImage \
  -t local-router:0.1.0 .
```

## Troubleshooting

**`Authorization required, but no authorization protocol specified`**: re-run
`xhost +SI:localuser:$(whoami)` on the host before `docker run`.

**`MESA-LOADER: failed to open ...`**: add the GPU groups (see "GPU access")
or run without `--device /dev/dri` to fall back to software rendering.

**`Failed to bind to 0.0.0.0:3625`**: another process on the host is using
the port. Map a different host port: `-p 13625:3625`.

**Tray icon missing / minimized to tray**: most desktop environments don't
proxy tray icons through X11 forwarding. The window itself still works; use
the window controls instead of the tray menu.

## Note on package visibility

The first push to `ghcr.io/localrouter/localrouter` creates a private package
by default. After the first successful release build, an org admin needs to
mark the package public from the GitHub Packages UI (Settings → Packages →
Package settings → Change visibility → Public). Once public, subsequent
versions inherit that setting automatically.

## Limitations and roadmap

- Linux hosts only (X11 forwarding requirement).
- linux/amd64 only — multi-arch (linux/arm64) support depends on adding
  aarch64 Linux to the release CI matrix.
- A true headless `--server-only` mode (no Tauri/webview) and a corresponding
  slim image are tracked as future work — they would make the image runnable
  on macOS, Windows, and ARM hosts as a server-only API gateway.

See [issue #5](https://github.com/LocalRouter/LocalRouter/issues/5) for the
original community proposal that this image is based on.
