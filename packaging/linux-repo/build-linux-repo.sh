#!/usr/bin/env bash
#
# Build the APT and YUM repositories that LocalRouter serves from GitHub Pages
# (the LocalRouter/packages repo).
#
# Usage:
#   packaging/linux-repo/build-linux-repo.sh \
#     --version 0.0.139 --assets-dir ./release-assets --repo-dir ./packages
#
# `--repo-dir` is a checkout of LocalRouter/packages. The script adds the new
# release to the pool, prunes old ones, and regenerates all index metadata.
# Committing and pushing is the caller's job.
#
# Signing is optional but strongly recommended: without APT_GPG_KEY_ID, apt
# clients must use `[trusted=yes]`, which disables authenticity checking.
#
# Requires (all present on ubuntu-latest runners):
#   dpkg-dev (dpkg-scanpackages), apt-utils (apt-ftparchive), createrepo-c, gpg
#
set -euo pipefail

# --------------------------------------------------------------------------
# GitHub Pages soft-limits a site to 1 GB. Each release contributes roughly
# 4 x 50 MB of .deb/.rpm across two architectures, and LocalRouter releases
# often (v0.0.138 by August 2026), so the pool MUST be pruned or the site will
# be over quota within a couple of months. Three versions keeps a rollback
# target or two while leaving plenty of headroom.
# --------------------------------------------------------------------------
KEEP_VERSIONS=3

ORIGIN="LocalRouter"
SUITE="stable"
COMPONENT="main"

VERSION=""
ASSETS_DIR=""
REPO_DIR=""

die() { echo "error: $*" >&2; exit 1; }
log() { echo "==> $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version)     VERSION="${2:-}"; shift 2 ;;
    --assets-dir)  ASSETS_DIR="${2:-}"; shift 2 ;;
    --repo-dir)    REPO_DIR="${2:-}"; shift 2 ;;
    --keep)        KEEP_VERSIONS="${2:-}"; shift 2 ;;
    *)             die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ]    || die "--version is required"
[ -n "$ASSETS_DIR" ] || die "--assets-dir is required"
[ -n "$REPO_DIR" ]   || die "--repo-dir is required"
[ -d "$ASSETS_DIR" ] || die "assets dir does not exist: $ASSETS_DIR"
[ -d "$REPO_DIR" ]   || die "repo dir does not exist: $REPO_DIR"

APT_DIR="$REPO_DIR/apt"
YUM_DIR="$REPO_DIR/yum"
POOL_DIR="$APT_DIR/pool/$COMPONENT/l/localrouter"

# ---------------------------------------------------------------------------
# Stage the new artifacts
# ---------------------------------------------------------------------------

mkdir -p "$POOL_DIR"

staged_any=0
for arch in amd64 arm64; do
  deb="$ASSETS_DIR/LocalRouter_${VERSION}_${arch}.deb"
  if [ -f "$deb" ]; then
    cp "$deb" "$POOL_DIR/"
    log "staged $(basename "$deb")"
    staged_any=1
  else
    echo "warning: no .deb for $arch" >&2
  fi
done

for arch in x86_64 aarch64; do
  rpm="$ASSETS_DIR/LocalRouter_${VERSION}_${arch}.rpm"
  if [ -f "$rpm" ]; then
    mkdir -p "$YUM_DIR/$arch"
    cp "$rpm" "$YUM_DIR/$arch/"
    log "staged $(basename "$rpm")"
    staged_any=1
  else
    echo "warning: no .rpm for $arch" >&2
  fi
done

[ "$staged_any" -eq 1 ] || die "no .deb or .rpm found for $VERSION in $ASSETS_DIR"

# ---------------------------------------------------------------------------
# Prune old versions
# ---------------------------------------------------------------------------

# Keep the newest $KEEP_VERSIONS distinct versions in a directory, by parsing
# the version out of `LocalRouter_<version>_<arch>.<ext>` and sorting with
# `sort -V`. Files whose version is not in the keep-set are deleted.
prune_dir() {
  local dir="$1" ext="$2"
  [ -d "$dir" ] || return 0

  local versions keep
  versions="$(
    find "$dir" -maxdepth 1 -name "LocalRouter_*.${ext}" -exec basename {} \; 2>/dev/null |
      sed -E "s/^LocalRouter_([^_]+)_.*\.${ext}$/\1/" |
      sort -Vu
  )"
  [ -n "$versions" ] || return 0

  keep="$(echo "$versions" | tail -n "$KEEP_VERSIONS")"

  # Read line by line rather than `for v in $versions`: that form depends on
  # IFS word-splitting, which silently does nothing in some shells and would
  # turn the whole list into a single "version" that matches nothing.
  local v
  while IFS= read -r v; do
    [ -n "$v" ] || continue
    if ! echo "$keep" | grep -qxF "$v"; then
      log "pruning $ext version $v from $(basename "$dir")"
      find "$dir" -maxdepth 1 -name "LocalRouter_${v}_*.${ext}" -delete
    fi
  done <<< "$versions"
}

prune_dir "$POOL_DIR" deb
for arch_dir in "$YUM_DIR"/*/; do
  [ -d "$arch_dir" ] && prune_dir "${arch_dir%/}" rpm
done

# ---------------------------------------------------------------------------
# APT metadata
# ---------------------------------------------------------------------------

log "Generating APT indexes"

for arch in amd64 arm64; do
  bin_dir="$APT_DIR/dists/$SUITE/$COMPONENT/binary-$arch"
  mkdir -p "$bin_dir"

  # dpkg-scanpackages needs paths relative to the repo root so that the
  # Filename: field apt reads resolves against the dists base URL.
  (
    cd "$APT_DIR"
    dpkg-scanpackages --arch "$arch" "pool/$COMPONENT/l/localrouter" /dev/null 2>/dev/null \
      > "dists/$SUITE/$COMPONENT/binary-$arch/Packages" || true
  )

  gzip -9 -c "$bin_dir/Packages" > "$bin_dir/Packages.gz"

  cat > "$bin_dir/Release" <<EOF
Archive: $SUITE
Component: $COMPONENT
Origin: $ORIGIN
Label: $ORIGIN
Architecture: $arch
EOF
done

(
  cd "$APT_DIR/dists/$SUITE"
  apt-ftparchive \
    -o "APT::FTPArchive::Release::Origin=$ORIGIN" \
    -o "APT::FTPArchive::Release::Label=$ORIGIN" \
    -o "APT::FTPArchive::Release::Suite=$SUITE" \
    -o "APT::FTPArchive::Release::Codename=$SUITE" \
    -o "APT::FTPArchive::Release::Architectures=amd64 arm64" \
    -o "APT::FTPArchive::Release::Components=$COMPONENT" \
    -o "APT::FTPArchive::Release::Description=LocalRouter APT repository" \
    release . > Release
)

# gpg invocation shared by the APT and YUM signing steps.
#
# A key with a passphrase cannot be used non-interactively without
# --pinentry-mode loopback; without it gpg tries to open a pinentry dialog and
# fails on a CI runner with an unhelpful "Inappropriate ioctl for device".
gpg_sign() {
  local args=(--batch --yes --default-key "$APT_GPG_KEY_ID")
  if [ -n "${APT_GPG_PASSPHRASE:-}" ]; then
    args+=(--pinentry-mode loopback --passphrase-fd 0)
    printf '%s' "$APT_GPG_PASSPHRASE" | gpg "${args[@]}" "$@"
  else
    gpg "${args[@]}" "$@"
  fi
}

if [ -n "${APT_GPG_KEY_ID:-}" ]; then
  log "Signing the APT Release file"
  (
    cd "$APT_DIR/dists/$SUITE"
    rm -f Release.gpg InRelease
    gpg_sign -abs -o Release.gpg Release
    gpg_sign --clearsign -o InRelease Release
  )
  # The public key apt clients import.
  gpg --batch --yes --armor --export "$APT_GPG_KEY_ID" > "$REPO_DIR/localrouter.asc"
else
  echo "warning: APT_GPG_KEY_ID unset — the repo is UNSIGNED" >&2
  echo "         clients will need [trusted=yes], which disables verification" >&2
fi

# ---------------------------------------------------------------------------
# YUM metadata
# ---------------------------------------------------------------------------

if [ -d "$YUM_DIR" ]; then
  for arch_dir in "$YUM_DIR"/*/; do
    [ -d "$arch_dir" ] || continue
    log "Generating YUM metadata for $(basename "${arch_dir%/}")"
    createrepo_c --update "$arch_dir"

    if [ -n "${APT_GPG_KEY_ID:-}" ]; then
      rm -f "${arch_dir}repodata/repomd.xml.asc"
      gpg_sign --detach-sign --armor "${arch_dir}repodata/repomd.xml"
    fi
  done

  # dnf/yum client config. $basearch expands on the client, so one file covers
  # both architectures.
  #
  # Compute the flag explicitly. `${VAR:+1}${VAR:-0}` looks like a neat
  # one-liner for "1 if set else 0" but is wrong: when VAR *is* set, the
  # second expansion yields VAR's value, producing `gpgcheck=1ABCDEF123`.
  if [ -n "${APT_GPG_KEY_ID:-}" ]; then
    gpg_flag=1
  else
    gpg_flag=0
  fi

  cat > "$YUM_DIR/localrouter.repo" <<EOF
[localrouter]
name=LocalRouter
baseurl=https://packages.localrouter.ai/yum/\$basearch/
enabled=1
gpgcheck=$gpg_flag
repo_gpgcheck=$gpg_flag
gpgkey=https://packages.localrouter.ai/localrouter.asc
EOF
fi

# GitHub Pages runs Jekyll by default, which strips directories beginning with
# an underscore and can mangle metadata files. .nojekyll turns that off.
touch "$REPO_DIR/.nojekyll"

log "Repository built at $REPO_DIR"
du -sh "$REPO_DIR" 2>/dev/null || true
