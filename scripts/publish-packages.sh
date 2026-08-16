#!/usr/bin/env bash
#
# Render the package-manager manifests in packaging/ for a released version,
# and optionally push them to the repositories that serve them.
#
# The same script backs the CI `publish-packages` job and manual runs, so that
# a hand-published package is byte-identical to a CI-published one.
#
# Usage:
#   scripts/publish-packages.sh --version 0.0.139 [options]
#
# Options:
#   --version X.Y.Z     Released version, without a leading "v" (required)
#   --only a,b,c        Channels to render. Default: all.
#                       One or more of: homebrew scoop aur winget flatpak snap
#   --assets-dir DIR    Directory holding the release assets. When omitted the
#                       assets are downloaded from the GitHub release, which
#                       needs `gh` to be authenticated.
#   --out-dir DIR       Where to write rendered files. Default: a temp dir.
#   --push              Push the rendered files to the tap / AUR. Without this
#                       the script only renders, which is the safe default.
#   -h, --help          Show this help.
#
# Pushing requires:
#   PACKAGING_PAT        token with write access to LocalRouter/homebrew-tap
#   AUR_SSH_PRIVATE_KEY  path to a key registered with the AUR account
#
set -euo pipefail

REPO_SLUG="LocalRouter/LocalRouter"
TAP_REPO="LocalRouter/homebrew-tap"
AUR_REMOTE="ssh://aur@aur.archlinux.org/localrouter-bin.git"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PACKAGING_DIR="$ROOT_DIR/packaging"

ALL_CHANNELS="homebrew scoop aur winget flatpak snap"

VERSION=""
CHANNELS="$ALL_CHANNELS"
ASSETS_DIR=""
OUT_DIR=""
DO_PUSH=0

die() {
  echo "error: $*" >&2
  exit 1
}

log() {
  echo "==> $*"
}

usage() {
  sed -n '3,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)     VERSION="${2:-}"; shift 2 ;;
    --only)        CHANNELS="$(echo "${2:-}" | tr ',' ' ')"; shift 2 ;;
    --assets-dir)  ASSETS_DIR="${2:-}"; shift 2 ;;
    --out-dir)     OUT_DIR="${2:-}"; shift 2 ;;
    --push)        DO_PUSH=1; shift ;;
    -h|--help)     usage; exit 0 ;;
    *)             die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ] || die "--version is required"
# Reject a leading "v": every asset name and template interpolates the bare
# version, and a stray "v" would silently produce 404 download URLs.
case "$VERSION" in
  v*) die "--version must not start with 'v' (got '$VERSION')" ;;
esac
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$' \
  || die "--version must be semver (got '$VERSION')"

for channel in $CHANNELS; do
  case " $ALL_CHANNELS " in
    *" $channel "*) ;;
    *) die "unknown channel '$channel' (valid: $ALL_CHANNELS)" ;;
  esac
done

if [ -z "$OUT_DIR" ]; then
  OUT_DIR="$(mktemp -d)"
fi
mkdir -p "$OUT_DIR"

# ---------------------------------------------------------------------------
# Assets
# ---------------------------------------------------------------------------

# Only the assets the requested channels actually need, so a `--only homebrew`
# run does not download 300 MB of Linux and Windows artifacts.
needed_assets() {
  for channel in $CHANNELS; do
    case "$channel" in
      homebrew) echo "LocalRouter_${VERSION}_aarch64.dmg"
                echo "LocalRouter_${VERSION}_x64.dmg" ;;
      scoop)    echo "LocalRouter_${VERSION}_x64_portable.exe" ;;
      winget)   echo "LocalRouter_${VERSION}_x64-setup.exe" ;;
      # snapcraft downloads the deb itself at build time, but the recipe
      # pins its sha256, which is computed here from the same assets.
      aur|flatpak|snap)
                echo "LocalRouter_${VERSION}_amd64.deb"
                echo "LocalRouter_${VERSION}_arm64.deb" ;;
    esac
  done | sort -u
}

if [ -z "$ASSETS_DIR" ]; then
  ASSETS_DIR="$OUT_DIR/assets"
  mkdir -p "$ASSETS_DIR"
  assets="$(needed_assets)"
  if [ -n "$assets" ]; then
    log "Downloading release assets for v$VERSION"
    # shellcheck disable=SC2086,SC2046 # deliberate splitting: each line is one -p pattern
    gh release download "v$VERSION" \
      --repo "$REPO_SLUG" \
      --dir "$ASSETS_DIR" \
      --clobber \
      $(echo "$assets" | sed 's/^/-p /') \
      || die "failed to download release assets — is 'gh' authenticated?"
  fi
fi

# SHA256 of a release asset.
#
# A missing asset is fatal. It is tempting to warn and continue, but the
# placeholder guard in render_template only catches a literal `__NAME__` — an
# empty substitution sails straight past it and produces a manifest with
# `sha256 ""`, which every package manager will either reject at install time
# or, worse, treat as "skip verification". If a channel was requested, all of
# its assets must exist.
asset_sha256() {
  local name="$1" path="$ASSETS_DIR/$1"
  if [ ! -f "$path" ]; then
    die "release asset missing: $name (looked in $ASSETS_DIR)"
  fi
  local sum
  if command -v sha256sum >/dev/null 2>&1; then
    sum="$(sha256sum "$path" | cut -d' ' -f1)"
  else
    # macOS
    sum="$(shasum -a 256 "$path" | cut -d' ' -f1)"
  fi

  # Guard against a truncated or malformed hash reaching a manifest.
  echo "$sum" | grep -Eq '^[0-9a-f]{64}$' \
    || die "implausible sha256 for $name: '$sum'"

  echo "$sum"
}

# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------

# The release date, used by manifests that record one. Derived from the git
# tag so a re-run reproduces the same output rather than stamping "today".
release_date() {
  local date_str
  date_str="$(git -C "$ROOT_DIR" log -1 --format=%cs "v$VERSION" 2>/dev/null || true)"
  if [ -z "$date_str" ]; then
    date_str="$(date -u +%Y-%m-%d)"
  fi
  echo "$date_str"
}

RELEASE_DATE="$(release_date)"

SHA_DMG_ARM=""
SHA_DMG_INTEL=""
SHA_PORTABLE_X64=""
SHA_NSIS_X64=""
SHA_DEB_AMD64=""
SHA_DEB_ARM64=""

# render_template <template> <destination> [extra sed expressions...]
#
# Substitutes the standard placeholders, then refuses to write the file if any
# `__PLACEHOLDER__` survived — an unsubstituted hash would otherwise ship a
# manifest that fails checksum verification on the user's machine.
render_template() {
  local template="$1" dest="$2"
  shift 2

  [ -f "$template" ] || die "template not found: $template"
  mkdir -p "$(dirname "$dest")"

  sed \
    -e "s|__VERSION__|$VERSION|g" \
    -e "s|__RELEASE_DATE__|$RELEASE_DATE|g" \
    -e "s|__SHA256_ARM__|$SHA_DMG_ARM|g" \
    -e "s|__SHA256_INTEL__|$SHA_DMG_INTEL|g" \
    -e "s|__SHA256_X64__|$SHA_PORTABLE_X64|g" \
    -e "s|__SHA256_X64_NSIS__|$SHA_NSIS_X64|g" \
    -e "s|__SHA256_AMD64__|$SHA_DEB_AMD64|g" \
    -e "s|__SHA256_ARM64__|$SHA_DEB_ARM64|g" \
    "$@" \
    "$template" > "$dest"

  if grep -q '__[A-Z0-9_]*__' "$dest"; then
    echo "unresolved placeholders in $dest:" >&2
    grep -o '__[A-Z0-9_]*__' "$dest" | sort -u >&2
    rm -f "$dest"
    die "refusing to publish a manifest with unresolved placeholders"
  fi

  log "rendered $dest"
}

wants() {
  case " $CHANNELS " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}

if wants homebrew; then
  SHA_DMG_ARM="$(asset_sha256 "LocalRouter_${VERSION}_aarch64.dmg")"
  SHA_DMG_INTEL="$(asset_sha256 "LocalRouter_${VERSION}_x64.dmg")"
  render_template "$PACKAGING_DIR/homebrew/localrouter.rb.tmpl" \
    "$OUT_DIR/homebrew-tap/Casks/localrouter.rb"
fi

if wants scoop; then
  SHA_PORTABLE_X64="$(asset_sha256 "LocalRouter_${VERSION}_x64_portable.exe")"
  render_template "$PACKAGING_DIR/scoop/localrouter.json.tmpl" \
    "$OUT_DIR/homebrew-tap/bucket/localrouter.json"
fi

if wants aur; then
  SHA_DEB_AMD64="$(asset_sha256 "LocalRouter_${VERSION}_amd64.deb")"
  SHA_DEB_ARM64="$(asset_sha256 "LocalRouter_${VERSION}_arm64.deb")"
  render_template "$PACKAGING_DIR/aur/PKGBUILD.tmpl" "$OUT_DIR/aur/PKGBUILD"
fi

if wants winget; then
  SHA_NSIS_X64="$(asset_sha256 "LocalRouter_${VERSION}_x64-setup.exe")"
  winget_dir="$OUT_DIR/winget/manifests/l/LocalRouter/LocalRouter/$VERSION"
  for tmpl in "$PACKAGING_DIR"/winget/manifests/*.yaml.tmpl; do
    render_template "$tmpl" "$winget_dir/$(basename "$tmpl" .tmpl)"
  done
fi

if wants flatpak; then
  SHA_DEB_AMD64="$(asset_sha256 "LocalRouter_${VERSION}_amd64.deb")"
  SHA_DEB_ARM64="$(asset_sha256 "LocalRouter_${VERSION}_arm64.deb")"
  render_template "$PACKAGING_DIR/flatpak/ai.localrouter.app.yml" \
    "$OUT_DIR/flatpak/ai.localrouter.app.yml"
  render_template "$PACKAGING_DIR/flatpak/ai.localrouter.app.metainfo.xml" \
    "$OUT_DIR/flatpak/ai.localrouter.app.metainfo.xml"
fi

if wants snap; then
  SHA_DEB_AMD64="$(asset_sha256 "LocalRouter_${VERSION}_amd64.deb")"
  SHA_DEB_ARM64="$(asset_sha256 "LocalRouter_${VERSION}_arm64.deb")"
  render_template "$PACKAGING_DIR/snap/snapcraft.yaml" "$OUT_DIR/snap/snapcraft.yaml"
fi

log "Rendered manifests are in $OUT_DIR"

# ---------------------------------------------------------------------------
# Publishing
# ---------------------------------------------------------------------------

if [ "$DO_PUSH" -eq 0 ]; then
  log "Dry run (no --push). Nothing was published."
  exit 0
fi

push_tap() {
  [ -d "$OUT_DIR/homebrew-tap" ] || return 0
  [ -n "${PACKAGING_PAT:-}" ] || die "PACKAGING_PAT is required to push the tap"

  local clone="$OUT_DIR/.tap-clone"
  rm -rf "$clone"
  log "Cloning $TAP_REPO"
  git clone --depth 1 \
    "https://x-access-token:${PACKAGING_PAT}@github.com/${TAP_REPO}.git" "$clone" \
    >/dev/null 2>&1 || die "failed to clone $TAP_REPO"

  cp -R "$OUT_DIR/homebrew-tap/." "$clone/"

  git -C "$clone" config user.name "Matus Faro"
  git -C "$clone" config user.email "matus@matus.io"
  git -C "$clone" add -A

  if git -C "$clone" diff --cached --quiet; then
    log "Tap already up to date at $VERSION"
    return 0
  fi

  git -C "$clone" commit -m "localrouter $VERSION"
  git -C "$clone" push origin HEAD
  log "Pushed $TAP_REPO"
}

push_aur() {
  [ -d "$OUT_DIR/aur" ] || return 0
  [ -n "${AUR_SSH_PRIVATE_KEY:-}" ] || die "AUR_SSH_PRIVATE_KEY is required to push the AUR package"

  local clone="$OUT_DIR/.aur-clone"
  rm -rf "$clone"

  log "Cloning $AUR_REMOTE"
  GIT_SSH_COMMAND="ssh -i $AUR_SSH_PRIVATE_KEY -o StrictHostKeyChecking=accept-new" \
    git clone "$AUR_REMOTE" "$clone" >/dev/null 2>&1 \
    || die "failed to clone the AUR package"

  cp "$OUT_DIR/aur/PKGBUILD" "$clone/PKGBUILD"

  # .SRCINFO must match PKGBUILD or the AUR rejects the push. makepkg only
  # exists on Arch, so generate it there and fall back to a warning elsewhere.
  if command -v makepkg >/dev/null 2>&1; then
    (cd "$clone" && makepkg --printsrcinfo > .SRCINFO)
  else
    echo "warning: makepkg not available; .SRCINFO not regenerated" >&2
    echo "         push from an Arch container or the AUR will reject it" >&2
  fi

  git -C "$clone" config user.name "Matus Faro"
  git -C "$clone" config user.email "matus@matus.io"
  git -C "$clone" add -A

  if git -C "$clone" diff --cached --quiet; then
    log "AUR package already up to date at $VERSION"
    return 0
  fi

  git -C "$clone" commit -m "localrouter-bin $VERSION"
  GIT_SSH_COMMAND="ssh -i $AUR_SSH_PRIVATE_KEY -o StrictHostKeyChecking=accept-new" \
    git -C "$clone" push origin HEAD
  log "Pushed the AUR package"
}

push_tap
push_aur

# Flatpak, Snap and WinGet are not pushed from here, but they ARE automated —
# each has a dedicated consumer in .github/workflows/release.yml:
#   flatpak — the build-flatpak jobs run flatpak-builder on the rendered
#             manifest, then publish-packages merges the result into the
#             self-hosted repo via packaging/linux-repo/build-flatpak-repo.sh
#   snap    — the build-snap jobs run snapcraft on the rendered recipe and
#             upload to the release (and to the Snap Store when credentialed)
#   winget  — packaging/winget/submit-winget.sh PRs microsoft/winget-pkgs
if wants flatpak || wants snap || wants winget; then
  log "flatpak/snap/winget rendered — publishing happens in release.yml, see packaging/README.md"
fi
