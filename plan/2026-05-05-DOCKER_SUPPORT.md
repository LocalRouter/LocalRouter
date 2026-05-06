# Docker Support for LocalRouter (issue #5)

## Context

Community contributor `supertorpe` filed [issue #5](https://github.com/LocalRouter/LocalRouter/issues/5) with a working Dockerfile that runs LocalRouter inside a Linux container by downloading the AppImage from the GitHub release and forwarding the host X11 socket into the container. The user wants this packaged into the repo and the resulting image published automatically via GitHub Container Registry on each release.

**Decision recorded**: Ship the AppImage-in-container approach (GUI via X11 forwarding, Linux hosts only) and publish to GHCR only, on release publication. A true headless server mode is intentionally deferred because the current binary has no way to skip Tauri/webview/tray init and the server handlers are wired into `tauri::State` — that's a separate, larger refactor.

## Scope

### In scope
1. `Dockerfile` at repo root — refined version of supertorpe's proposal.
2. `docker-entrypoint.sh` — ensures `server.host: 0.0.0.0` on first boot so users don't have to edit `settings.yaml` and restart (the issue's main rough edge).
3. `.dockerignore` — keep build context small.
4. `.github/workflows/docker.yml` — builds image on release publication, pushes to `ghcr.io/localrouter/localrouter:<version>` and `:latest`. linux/amd64 only (CI does not currently build aarch64 Linux).
5. README "Docker" section + brief `docs/DOCKER.md` covering X11 forwarding, persistent volume, and the macOS/Windows host caveat.

### Out of scope (future work, do not bundle here)
- Headless `--server-only` mode + slim Alpine image.
- linux/arm64 builds (would need an `aarch64-unknown-linux-gnu` matrix entry in `release.yml`).
- VNC/Xvfb-in-image fallback for non-Linux Docker hosts.
- Docker Hub mirror.

## First steps (per CLAUDE.md)

1. Create todo list for the implementation steps below so progress is visible.
2. Save this plan into the repo via `./copy-plan.sh can-you-take-a-lovely-pancake DOCKER_SUPPORT` before writing any code.

## Implementation

### 1. `Dockerfile` (repo root)

Based on supertorpe's proposal, with these refinements:

- Build-arg `APPIMAGE_URL` so the workflow can pin the exact release asset; default to the version-agnostic stable URL `https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_amd64.AppImage` (already produced by `release.yml:296`) so a manual `docker build .` works out of the box.
- Pre-set `ENV LOCALROUTER_KEYCHAIN=file` — the Linux Secret Service / DBus is not available in the container; this env var (per `crates/lr-api-keys/src/keychain_trait.rs:283`) routes secrets to `~/.localrouter/secrets.json` instead. Without it, key storage will fail.
- Do **not** set `LOCALROUTER_ENV` so the in-container config dir is `~/.localrouter` — same layout as a host install, so users can copy/share configs.
- Install only the runtime deps from supertorpe's list (curl/jq are build-time only, drop from runtime layer).
- Use a non-root `app` user (UID/GID 1000) with `HOME=/home/app`. Caller can still override with `-u $(id -u):$(id -g) -e HOME=/home`.
- `EXPOSE 3625`.
- `ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]`, default `CMD` runs the AppImage with `--appimage-extract-and-run`.

### 2. `docker-entrypoint.sh`

- If `$HOME/.localrouter/settings.yaml` does not exist, write a minimal stub:
  ```yaml
  server:
    host: 0.0.0.0
    port: 3625
  ```
  This solves the issue's "edit settings.yaml after first run and restart" rough edge. The app's config loader merges defaults, so any other fields stay at their defaults (verify by reading `crates/lr-config/` `Config::load` path and confirming the partial-file merge behavior before relying on it — if it doesn't merge, write a full default doc instead).
- `exec "$@"` so signals propagate to the AppImage process.

### 3. `.dockerignore`

Standard exclusions: `target/`, `node_modules/`, `.git/`, `dist/`, `website/dist/`, `plan/`, `docs/`, `.idea/`, `*.log`, anything not needed for the (very minimal) build context.

### 4. `.github/workflows/docker.yml`

```yaml
name: Docker

on:
  release:
    types: [published]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to package (e.g. 0.1.0)'
        required: true

permissions:
  contents: read
  packages: write

jobs:
  build-and-push:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - name: Resolve version + AppImage URL
        # github.event.release.tag_name (e.g. v0.1.0) on release trigger,
        # or inputs.version on workflow_dispatch
      - uses: docker/metadata-action@v5
        # Tags: ghcr.io/localrouter/localrouter:<version>, :latest
      - uses: docker/build-push-action@v6
        with:
          context: .
          platforms: linux/amd64
          push: true
          build-args: |
            APPIMAGE_URL=https://github.com/LocalRouter/LocalRouter/releases/download/v${VERSION}/LocalRouter_${VERSION}_amd64.AppImage
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
```

Trigger on `release: published` (fires when `release.yml`'s `softprops/action-gh-release` step completes), so the AppImage asset is guaranteed to exist before this workflow tries to download it. `workflow_dispatch` is kept for manual re-runs.

The image will be published under the org namespace lowercased: **`ghcr.io/localrouter/localrouter`** (GitHub auto-lowercases org names for OCI; `LocalRouter/LocalRouter` → `localrouter/localrouter`).

### 5. Documentation

- `README.md`: add a short "Docker (Linux hosts, experimental)" subsection with the `docker run` invocation and a link to `docs/DOCKER.md`.
- `docs/DOCKER.md`: full instructions covering
  - The X11 forwarding model + why it's Linux-host only (macOS Docker Desktop and Docker on Windows don't expose `/tmp/.X11-unix`).
  - Persistent config: `-v ./.localrouter:/home/app/.localrouter`.
  - The `xhost +SI:localuser:$(whoami)` invocation (from supertorpe's follow-up comment, safer than `xhost +local:docker`).
  - `--device /dev/dri`, `--group-add` for `video`/`render` groups (also from the follow-up comment).
  - Note that the API is reachable at `http://localhost:3625` from the host once the entrypoint has written `server.host: 0.0.0.0`.

## Critical files

- **New**: `Dockerfile`, `docker-entrypoint.sh`, `.dockerignore`, `.github/workflows/docker.yml`, `docs/DOCKER.md`.
- **Modified**: `README.md` (add Docker subsection).
- **Read-only references**:
  - `.github/workflows/release.yml:79-198` — release matrix, asset naming, version-agnostic copies (line 296).
  - `crates/lr-utils/src/paths.rs:14-30` — `config_dir()` and `LOCALROUTER_ENV` semantics.
  - `crates/lr-api-keys/src/keychain_trait.rs:277-307` — `LOCALROUTER_KEYCHAIN=file` mode.
  - `src-tauri/src/config/types.rs:1447-1462` — `ServerConfig` default `host: "127.0.0.1"`.
  - `src-tauri/src/server/mod.rs:101-133` — bind address construction (no env-var override exists, so the entrypoint-writes-settings-yaml approach is the right lever).

## Verification

Run end-to-end before opening the PR:

1. **Local build** (Linux host required for the GUI step):
   ```bash
   docker build -t local-router-test .
   ```
2. **First-boot smoke** — no pre-existing config:
   ```bash
   rm -rf ./test-home && mkdir -p ./test-home
   xhost +SI:localuser:$(whoami)
   docker run --rm \
     -v /tmp/.X11-unix:/tmp/.X11-unix \
     -v $(pwd)/test-home:/home/app \
     -p 3625:3625 \
     -e DISPLAY=$DISPLAY \
     --device /dev/dri \
     local-router-test &
   sleep 15
   curl -fsS http://localhost:3625/health      # must succeed (proves 0.0.0.0 default kicked in)
   grep -q "host: 0.0.0.0" test-home/.localrouter/settings.yaml  # entrypoint wrote it
   ```
3. **Persistence** — restart with same volume, confirm config carries over and UI loads.
4. **Workflow dry-run** — push to a branch, trigger `docker.yml` via `workflow_dispatch` against an existing released version, confirm the image lands on `ghcr.io/localrouter/localrouter:<version>` and `:latest`, and that `docker run --rm -p 3625:3625 ghcr.io/localrouter/localrouter:<version>` (no X11) at minimum boots without crashing the entrypoint (the GUI itself will fail without a display — that's expected).

## Mandatory final steps (per CLAUDE.md)

1. **Plan review** — diff this plan against the implementation; close any gaps (especially: did the entrypoint settings-merge behavior need adjustment? did we forget the README link?).
2. **Test coverage review** — Docker artifacts aren't unit-tested, but verify the entrypoint script's settings.yaml-write branch with a shell-only run (`docker run ... true`) and confirm the file is created.
3. **Bug hunt** — re-read `Dockerfile` and `docker-entrypoint.sh` cold, looking for: signal handling (does AppImage receive SIGTERM?), permission issues on the bind-mounted volume when `-u` is overridden, race between entrypoint write and AppImage start, and the version-resolution logic in `docker.yml` for both trigger paths.
4. **Commit** — one focused commit, only files I modified, no auto-stash/auto-pull. Follow Conventional Commits (`feat(docker): ...`). Do not push unless asked.

## Open question to resolve during implementation, not blocking the plan

Confirm that `Config::load` in `crates/lr-config/` merges a partial `settings.yaml` against defaults. If it doesn't, the entrypoint must write a complete default config rather than just the `server:` block — read `crates/lr-config/src/lib.rs` (or wherever the load logic lives) to verify before finalizing the entrypoint script.
