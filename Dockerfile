# LocalRouter — containerized AppImage build.
#
# This image runs the published Linux AppImage inside a container. The Tauri
# webview still needs a display server, so the container only works on Linux
# hosts that forward their X11 socket (see docs/DOCKER.md). On macOS / Windows
# Docker hosts the GUI cannot start; the API server inside still binds to
# 0.0.0.0:3625, but you would need a separate display setup (e.g. Xvfb) to
# initialize the webview, which is out of scope for this image.
#
# Build:  docker build -t local-router .
# Run:    see docs/DOCKER.md

# ---- Stage 1: download the AppImage --------------------------------------
FROM debian:bookworm-slim AS downloader

# Default to the version-agnostic stable URL produced by release.yml so a plain
# `docker build .` works without arguments. CI overrides this with the exact
# versioned URL of the release that just published.
ARG APPIMAGE_URL=https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_amd64.AppImage

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /tmp/dl
RUN set -eux; \
    curl -fL --retry 5 --retry-delay 5 -o LocalRouter.AppImage "$APPIMAGE_URL"; \
    chmod +x LocalRouter.AppImage

# ---- Stage 2: runtime ----------------------------------------------------
FROM debian:bookworm-slim

# Runtime libraries the AppImage's bundled WebKitGTK / GTK stack links against.
# Mirrors supertorpe's working list from issue #5, minus build-only tools.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libfuse2 fontconfig libfribidi0 libgl1 libegl1 \
        libx11-6 libxext6 libxrender1 libxrandr2 libxi6 libxtst6 \
        libglib2.0-0 libnss3 libasound2 libatk1.0-0 libcups2 \
        libdbus-1-3 libdrm2 libgbm1 libgtk-3-0 \
        libxkbcommon0 libgl1-mesa-dri libgles2 libayatana-appindicator3-1 \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as a non-root user. UID/GID 1000 matches the typical first user on
# Linux desktops, so a bind-mounted ~/.localrouter from the host is owned
# by the in-container user without uid remapping. Override at runtime with
# `-u $(id -u):$(id -g)` if needed.
RUN groupadd -g 1000 app && useradd -m -u 1000 -g 1000 -s /bin/bash app

WORKDIR /app
COPY --from=downloader /tmp/dl/LocalRouter.AppImage /app/LocalRouter.AppImage
RUN chown app:app /app/LocalRouter.AppImage

COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Linux Secret Service / DBus is not available in the container; route
# secrets to the file-based keychain (~/.localrouter/secrets.json). See
# crates/lr-api-keys/src/keychain_trait.rs for the env-var contract.
ENV LOCALROUTER_KEYCHAIN=file

USER app
ENV HOME=/home/app

EXPOSE 3625

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/app/LocalRouter.AppImage", "--appimage-extract-and-run"]
