#!/usr/bin/env bash
# Generate release notes for a given tag, grouping commits by
# Conventional-Commits type.
#
# Usage:
#   scripts/generate-release-notes.sh <tag> [--prev <prev-tag>] [--repo <owner/name>]
#
#   tag        The tag this release corresponds to (e.g. v0.0.107). May or
#              may not exist yet — the caller decides. Commit links use
#              this value only in the compare URL.
#   --prev    Previous tag to compare against. If omitted, picked
#              automatically as the highest semver tag strictly lower than
#              <tag> that's an ancestor of the tag's commit (or HEAD if
#              the tag doesn't exist yet).
#   --repo    GitHub `owner/name`. Defaults to the value of
#              GITHUB_REPOSITORY, else parsed from `origin` remote.
#
# Output: markdown to stdout. Exits non-zero on error.
set -euo pipefail

TAG=""
PREV_TAG=""
REPO="${GITHUB_REPOSITORY:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prev) PREV_TAG="$2"; shift 2 ;;
    --repo) REPO="$2"; shift 2 ;;
    -*) echo "Unknown option: $1" >&2; exit 2 ;;
    *) if [[ -z "$TAG" ]]; then TAG="$1"; else echo "Unexpected arg: $1" >&2; exit 2; fi; shift ;;
  esac
done

if [[ -z "$TAG" ]]; then
  echo "Usage: $0 <tag> [--prev <prev-tag>] [--repo <owner/name>]" >&2
  exit 2
fi

if [[ -z "$REPO" ]]; then
  # Parse `git@github.com:owner/name.git` or `https://github.com/owner/name.git`.
  origin_url="$(git config --get remote.origin.url || true)"
  REPO="$(echo "$origin_url" | sed -E 's#.*[:/]([^/:]+/[^/:]+)(\.git)?$#\1#')"
  REPO="${REPO%.git}"
fi

VERSION="${TAG#v}"

# Decide the commit range. Prefer the tag itself when it exists locally;
# otherwise use HEAD (release workflow invokes us before the tag is pushed).
if git rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null; then
  TIP="$TAG"
else
  TIP="HEAD"
fi

# Auto-pick the previous tag: highest semver strictly lower than $TAG
# that is an ancestor of $TIP. We descend the sorted list and take the
# first ancestor we find.
if [[ -z "$PREV_TAG" ]]; then
  while IFS= read -r t; do
    [[ "$t" == "$TAG" ]] && continue
    if git merge-base --is-ancestor "$t" "$TIP" 2>/dev/null; then
      PREV_TAG="$t"
      break
    fi
  done < <(
    git tag --list 'v*.*.*' --sort=-v:refname \
      | awk -v tag="$TAG" '
        # Keep only tags strictly less than $TAG using semver comparison.
        function semver_parts(t, arr,   s) {
          s = t; sub(/^v/, "", s); sub(/-.*/, "", s);
          split(s, arr, ".");
        }
        function semver_lt(a, b,   pa, pb, i) {
          semver_parts(a, pa); semver_parts(b, pb);
          for (i = 1; i <= 3; i++) {
            if ((pa[i]+0) < (pb[i]+0)) return 1;
            if ((pa[i]+0) > (pb[i]+0)) return 0;
          }
          return 0;
        }
        { if (semver_lt($0, tag)) print }
      '
  )
fi

RANGE="${PREV_TAG:+${PREV_TAG}..}${TIP}"
echo "# notes for $TAG (range: $RANGE, repo: $REPO)" >&2

# Load commits. Skip merge commits and the automated version-bump commit
# so the log stays signal-only.
mapfile -t LINES < <(
  git log --no-merges --pretty=format:'%H|%s' "$RANGE" \
    | grep -vE '^[a-f0-9]+\|chore\(release\): bump version' \
    || true
)

declare -A buckets
for k in feat fix perf refactor docs test build_ci style other; do buckets[$k]=""; done
append() { buckets[$1]+="$2"$'\n'; }

for line in "${LINES[@]}"; do
  hash="${line%%|*}"
  subject="${line#*|}"
  short="${hash:0:7}"
  entry="- ${subject} ([\`${short}\`](https://github.com/${REPO}/commit/${hash}))"
  case "$subject" in
    feat\(*|feat:*)                                   append feat "$entry" ;;
    fix\(*|fix:*)                                     append fix "$entry" ;;
    perf\(*|perf:*)                                   append perf "$entry" ;;
    refactor\(*|refactor:*)                           append refactor "$entry" ;;
    docs\(*|docs:*)                                   append docs "$entry" ;;
    test\(*|test:*)                                   append test "$entry" ;;
    build\(*|build:*|ci\(*|ci:*|chore\(*|chore:*)     append build_ci "$entry" ;;
    style\(*|style:*)                                 append style "$entry" ;;
    *)                                                append other "$entry" ;;
  esac
done

section() {
  local heading="$1" key="$2"
  local content="${buckets[$key]}"
  if [[ -n "$content" ]]; then
    echo ""
    echo "#### $heading"
    echo ""
    printf '%s' "$content"
  fi
}

{
  echo "## LocalRouter v${VERSION}"
  echo ""
  echo "### Installation"
  echo ""
  echo "Download the appropriate file for your platform:"
  echo ""
  echo "| Platform | File |"
  echo "|----------|------|"
  echo "| macOS (Intel) | \`LocalRouter_*_x64.dmg\` |"
  echo "| macOS (Apple Silicon) | \`LocalRouter_*_aarch64.dmg\` |"
  echo "| Windows | \`LocalRouter_*_x64-setup.exe\` or \`.msi\` |"
  echo "| Linux | \`LocalRouter_*_amd64.AppImage\` or \`.deb\` |"
  echo ""
  echo "### Auto-Update"
  echo ""
  echo "This release includes cryptographically signed binaries. The app will automatically check for updates weekly (configurable in Preferences → Updates)."
  echo ""
  echo "### Verification"
  echo ""
  echo "All binaries are signed. The updater will verify signatures before installation."
  echo ""
  echo "### Changes"
  if [[ -n "$PREV_TAG" ]]; then
    echo ""
    echo "Since [${PREV_TAG}](https://github.com/${REPO}/releases/tag/${PREV_TAG}):"
  fi

  section "Features"      feat
  section "Fixes"         fix
  section "Performance"   perf
  section "Refactors"     refactor
  section "Documentation" docs
  section "Tests"         test
  section "Build & CI"    build_ci
  section "Style"         style
  section "Other"         other

  echo ""
  if [[ -n "$PREV_TAG" ]]; then
    echo "**Full diff:** https://github.com/${REPO}/compare/${PREV_TAG}...${TAG}"
  else
    echo "**Full commits:** https://github.com/${REPO}/commits/${TAG}"
  fi
}
