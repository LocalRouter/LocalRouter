#!/usr/bin/env bash
#
# Merge per-architecture flatpak-builder output into the single OSTree
# repository that LocalRouter serves from GitHub Pages (the
# LocalRouter/packages repo, at https://packages.localrouter.ai/flatpak).
#
# This is the flatpak equivalent of the Homebrew tap: our own remote, no
# Flathub review. Users run
#   flatpak install --from https://packages.localrouter.ai/flatpak/localrouter.flatpakref
# once, and `flatpak update` follows every release.
#
# Usage:
#   packaging/linux-repo/build-flatpak-repo.sh \
#     --repo-dir ./packages --src-repo ./flatpak-repo-x86_64 [--src-repo ...]
#
# `--repo-dir` is a checkout of LocalRouter/packages; the OSTree repo lives in
# its flatpak/ subdirectory. Each `--src-repo` is the (unsigned) repo a
# `flatpak-builder --repo=...` run produced for one architecture. Commits are
# copied in with `flatpak build-commit-from`, which re-signs them with the
# release key — the flatpak-documented pattern for promoting autobuilder
# output into an official signed repo.
#
# Signing uses APT_GPG_KEY_ID (the same key as the APT/YUM repos). The key
# must already be imported AND its passphrase preset in gpg-agent
# (gpg-preset-passphrase), because ostree drives gpg internally and offers no
# passphrase plumbing of its own. Without APT_GPG_KEY_ID the repo is unsigned
# and clients fall back to gpg-verify=false.
#
# Requires: flatpak, ostree, gpg (apt: flatpak ostree)
#
set -euo pipefail

BASE_URL="https://packages.localrouter.ai/flatpak"
APP_ID="ai.localrouter.app"
BRANCH="stable"

REPO_DIR=""
SRC_REPOS=()

die() { echo "error: $*" >&2; exit 1; }
log() { echo "==> $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --repo-dir) REPO_DIR="${2:-}"; shift 2 ;;
    --src-repo) SRC_REPOS+=("${2:-}"); shift 2 ;;
    *)          die "unknown argument: $1" ;;
  esac
done

[ -n "$REPO_DIR" ]           || die "--repo-dir is required"
[ -d "$REPO_DIR" ]           || die "repo dir does not exist: $REPO_DIR"
[ "${#SRC_REPOS[@]}" -gt 0 ] || die "at least one --src-repo is required"

GPG_ARGS=()
if [ -n "${APT_GPG_KEY_ID:-}" ]; then
  GPG_ARGS=(--gpg-sign="$APT_GPG_KEY_ID")
else
  echo "warning: APT_GPG_KEY_ID unset — the flatpak repo is UNSIGNED" >&2
  echo "         clients will need --no-gpg-verify" >&2
fi

DEST="$REPO_DIR/flatpak"
if [ ! -d "$DEST/objects" ]; then
  log "Initialising OSTree repo at $DEST"
  ostree init --repo="$DEST" --mode=archive-z2
fi

# ---------------------------------------------------------------------------
# Import each architecture's build
# ---------------------------------------------------------------------------

for src in "${SRC_REPOS[@]}"; do
  [ -d "$src/objects" ] || die "not an OSTree repo: $src"

  # Copy every ref the builder exported: the app itself plus the appstream
  # branches `flatpak update --appstream` reads.
  refs="$(ostree refs --repo="$src")"
  [ -n "$refs" ] || die "no refs in $src"

  while IFS= read -r ref; do
    [ -n "$ref" ] || continue
    log "importing $ref from $src"
    # --no-update-summary: the summary is regenerated once at the end.
    flatpak build-commit-from \
      --no-update-summary \
      "${GPG_ARGS[@]}" \
      --src-repo="$src" \
      "$DEST" "$ref"
  done <<< "$refs"
done

# ---------------------------------------------------------------------------
# Metadata, deltas, pruning
# ---------------------------------------------------------------------------

# GitHub Pages soft-limits a site to 1 GB, shared with the APT/YUM pool, so
# keep only the latest commit per ref. Static deltas matter on Pages: without
# them a fresh install fetches thousands of individual object files.
log "Updating repo metadata"
flatpak build-update-repo \
  "${GPG_ARGS[@]}" \
  --generate-static-deltas \
  --prune \
  --prune-depth=1 \
  --title="LocalRouter" \
  --default-branch="$BRANCH" \
  "$DEST"

# ---------------------------------------------------------------------------
# Client-facing install files
# ---------------------------------------------------------------------------

GPG_KEY_B64=""
if [ -n "${APT_GPG_KEY_ID:-}" ]; then
  GPG_KEY_B64="$(gpg --export "$APT_GPG_KEY_ID" | base64 -w0)"
fi

{
  echo "[Flatpak Repo]"
  echo "Title=LocalRouter"
  echo "Url=$BASE_URL"
  echo "Homepage=https://localrouter.ai"
  echo "Comment=LocalRouter's own flatpak repository"
  [ -n "$GPG_KEY_B64" ] && echo "GPGKey=$GPG_KEY_B64"
} > "$DEST/localrouter.flatpakrepo"

{
  echo "[Flatpak Ref]"
  echo "Name=$APP_ID"
  echo "Branch=$BRANCH"
  echo "Title=LocalRouter"
  echo "Url=$BASE_URL"
  echo "IsRuntime=false"
  # Where the org.gnome.Platform runtime dependency comes from.
  echo "RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo"
  [ -n "$GPG_KEY_B64" ] && echo "GPGKey=$GPG_KEY_B64"
} > "$DEST/localrouter.flatpakref"

# GitHub Pages runs Jekyll by default; .nojekyll stops it from mangling files.
touch "$REPO_DIR/.nojekyll"

log "Flatpak repo built at $DEST"
du -sh "$DEST" 2>/dev/null || true
