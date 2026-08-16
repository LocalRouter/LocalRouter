#!/usr/bin/env bash
#
# Submit the rendered WinGet manifests to microsoft/winget-pkgs as a PR.
#
# WinGet has no user-hostable equivalent of a Homebrew tap (third-party REST
# sources exist but require running a server), so the community repo is the
# distribution channel — but the *submission* is fully automatable: this
# script forks microsoft/winget-pkgs under the token's account, pushes a
# one-commit branch with the manifests, and opens the PR. Microsoft's
# validation pipeline plus a moderator take it from there; new versions of an
# already-accepted package are routinely waved through.
#
# Usage:
#   packaging/winget/submit-winget.sh --version 0.0.139 --manifests-dir /tmp/pkg/winget
#
# `--manifests-dir` is the directory scripts/publish-packages.sh rendered,
# i.e. the one containing manifests/l/LocalRouter/LocalRouter/<version>/.
#
# Requires:
#   WINGET_PAT  a CLASSIC PAT with `public_repo` scope. Fine-grained PATs
#               cannot open PRs against repos you don't own, and pushing to a
#               fork of microsoft/winget-pkgs needs the classic scope.
#   gh, git
#
set -euo pipefail

UPSTREAM="microsoft/winget-pkgs"
PKG_PATH="manifests/l/LocalRouter/LocalRouter"

VERSION=""
MANIFESTS_DIR=""

die() { echo "error: $*" >&2; exit 1; }
log() { echo "==> $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version)       VERSION="${2:-}"; shift 2 ;;
    --manifests-dir) MANIFESTS_DIR="${2:-}"; shift 2 ;;
    *)               die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ]              || die "--version is required"
[ -n "$MANIFESTS_DIR" ]        || die "--manifests-dir is required"
[ -n "${WINGET_PAT:-}" ]       || die "WINGET_PAT is required"

SRC_DIR="$MANIFESTS_DIR/$PKG_PATH/$VERSION"
[ -d "$SRC_DIR" ] || die "rendered manifests not found at $SRC_DIR"
ls "$SRC_DIR"/*.yaml >/dev/null 2>&1 || die "no .yaml manifests in $SRC_DIR"

export GH_TOKEN="$WINGET_PAT"

FORK_OWNER="$(gh api user -q .login)" || die "WINGET_PAT is not a valid token"
FORK="$FORK_OWNER/winget-pkgs"

# ---------------------------------------------------------------------------
# Fork (idempotent) and sync it with upstream
# ---------------------------------------------------------------------------

if ! gh repo view "$FORK" --json name >/dev/null 2>&1; then
  log "Forking $UPSTREAM as $FORK"
  gh repo fork "$UPSTREAM" --clone=false
  # Fork creation is asynchronous; give GitHub a moment before cloning.
  for _ in 1 2 3 4 5 6; do
    gh repo view "$FORK" --json name >/dev/null 2>&1 && break
    sleep 5
  done
  gh repo view "$FORK" --json name >/dev/null 2>&1 \
    || die "fork $FORK did not become available"
fi

# A stale fork makes the PR diff include thousands of unrelated commits.
# A just-created fork is already in sync, so failure here is only a warning.
log "Syncing $FORK with $UPSTREAM"
gh repo sync "$FORK" --source "$UPSTREAM" --branch master \
  || echo "warning: fork sync failed; continuing with the fork's master" >&2

# ---------------------------------------------------------------------------
# Clone just our corner of the repo
# ---------------------------------------------------------------------------

# winget-pkgs holds hundreds of thousands of manifests; a blobless+treeless
# partial clone with a cone sparse-checkout of our package directory keeps
# this to a few MB instead of gigabytes.
CLONE="$(mktemp -d)/winget-pkgs"
log "Sparse-cloning $FORK"
git clone --depth 1 --filter=tree:0 --sparse --no-checkout \
  "https://x-access-token:${WINGET_PAT}@github.com/${FORK}.git" "$CLONE" \
  >/dev/null 2>&1 || die "failed to clone $FORK"

git -C "$CLONE" sparse-checkout set --cone "$PKG_PATH"
git -C "$CLONE" checkout master >/dev/null 2>&1

if [ -d "$CLONE/$PKG_PATH/$VERSION" ]; then
  log "$PKG_PATH/$VERSION already exists in $FORK — nothing to submit"
  exit 0
fi

# "New package" vs "New version" matters: winget-pkgs tooling and moderators
# key off the PR title.
if [ -d "$CLONE/$PKG_PATH" ] && ls "$CLONE/$PKG_PATH" >/dev/null 2>&1 \
   && [ -n "$(ls "$CLONE/$PKG_PATH")" ]; then
  TITLE="New version: LocalRouter.LocalRouter version $VERSION"
else
  TITLE="New package: LocalRouter.LocalRouter version $VERSION"
fi

BRANCH="localrouter-$VERSION"
mkdir -p "$CLONE/$PKG_PATH/$VERSION"
cp "$SRC_DIR"/*.yaml "$CLONE/$PKG_PATH/$VERSION/"

git -C "$CLONE" config user.name "Matus Faro"
git -C "$CLONE" config user.email "matus@matus.io"
git -C "$CLONE" checkout -b "$BRANCH" >/dev/null 2>&1
git -C "$CLONE" add "$PKG_PATH/$VERSION"
git -C "$CLONE" commit -m "$TITLE" >/dev/null
# --force: a re-run of the same release overwrites its own earlier branch.
git -C "$CLONE" push --force origin "$BRANCH" >/dev/null 2>&1 \
  || die "failed to push $BRANCH to $FORK"

# ---------------------------------------------------------------------------
# Open the PR (idempotent)
# ---------------------------------------------------------------------------

existing="$(gh pr list --repo "$UPSTREAM" \
  --head "$FORK_OWNER:$BRANCH" --state open --json number -q '.[0].number' || true)"
if [ -n "$existing" ]; then
  log "PR #$existing already open for $BRANCH"
  exit 0
fi

log "Opening PR against $UPSTREAM"
gh pr create --repo "$UPSTREAM" \
  --base master \
  --head "$FORK_OWNER:$BRANCH" \
  --title "$TITLE" \
  --body "Automated submission from LocalRouter's release pipeline.

- Manifests are rendered from [packaging/winget](https://github.com/LocalRouter/LocalRouter/tree/master/packaging/winget) with the SHA256 of the published installer.
- Release: https://github.com/LocalRouter/LocalRouter/releases/tag/v$VERSION"

log "Submitted $TITLE"
