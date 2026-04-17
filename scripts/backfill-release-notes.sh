#!/usr/bin/env bash
# Regenerate release notes for every existing GitHub release tag and
# update the release body via `gh release edit`.
#
# Usage:
#   scripts/backfill-release-notes.sh              # all tags
#   scripts/backfill-release-notes.sh v0.0.90      # just one
#   scripts/backfill-release-notes.sh --dry-run    # print only, don't edit
#   scripts/backfill-release-notes.sh --since v0.0.90   # tags >= v0.0.90
#
# Requires: gh CLI authenticated, write access to the repo.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
GEN="$here/generate-release-notes.sh"

DRY_RUN=0
SINCE=""
ONLY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --since) SINCE="$2"; shift 2 ;;
    -*) echo "Unknown option: $1" >&2; exit 2 ;;
    *) ONLY="$1"; shift ;;
  esac
done

# Build list of release tags to process. Use gh so we only touch tags
# that actually have a GitHub release (don't want to create new ones).
mapfile -t ALL_TAGS < <(gh release list --limit 200 --json tagName -q '.[].tagName')

TAGS=()
if [[ -n "$ONLY" ]]; then
  TAGS=("$ONLY")
else
  TAGS=("${ALL_TAGS[@]}")
fi

# Filter by --since if provided. Uses semver compare (ignoring pre-release).
semver_ge() {
  # returns 0 if $1 >= $2
  local a="${1#v}" b="${2#v}"
  a="${a%%-*}"; b="${b%%-*}"
  IFS=. read -r a1 a2 a3 <<< "$a"
  IFS=. read -r b1 b2 b3 <<< "$b"
  for i in 1 2 3; do
    local av bv
    case $i in 1) av=$a1; bv=$b1 ;; 2) av=$a2; bv=$b2 ;; 3) av=$a3; bv=$b3 ;; esac
    (( av > bv )) && return 0
    (( av < bv )) && return 1
  done
  return 0
}

if [[ -n "$SINCE" ]]; then
  FILTERED=()
  for t in "${TAGS[@]}"; do
    if semver_ge "$t" "$SINCE"; then FILTERED+=("$t"); fi
  done
  TAGS=("${FILTERED[@]}")
fi

if [[ ${#TAGS[@]} -eq 0 ]]; then
  echo "No tags to process." >&2
  exit 0
fi

echo "Will process ${#TAGS[@]} tag(s): ${TAGS[*]}"

fail=0
for tag in "${TAGS[@]}"; do
  echo ""
  echo "=== $tag ==="
  tmp="$(mktemp)"
  # The script logs its range detection to stderr; keep that visible.
  if ! bash "$GEN" "$tag" > "$tmp"; then
    echo "  FAILED to generate notes for $tag" >&2
    rm -f "$tmp"
    fail=$((fail+1))
    continue
  fi

  if [[ $DRY_RUN -eq 1 ]]; then
    echo "--- $tag (dry-run, not writing) ---"
    cat "$tmp"
    rm -f "$tmp"
    continue
  fi

  if gh release edit "$tag" --notes-file "$tmp" >/dev/null; then
    echo "  updated"
  else
    echo "  FAILED to update $tag" >&2
    fail=$((fail+1))
  fi
  rm -f "$tmp"
done

echo ""
if [[ $fail -gt 0 ]]; then
  echo "$fail tag(s) failed." >&2
  exit 1
fi
echo "Done."
