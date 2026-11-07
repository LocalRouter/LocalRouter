#!/usr/bin/env bash
# Runs the actual Codex CLI against the local TLS provider and real AppFirewall.
# Provider auth is synthetic and public destinations are rejected by the fixture.
set -euo pipefail

case "${1:-}" in
  ''|--preflight-only) ;;
  *) echo 'Usage: test-codex-secret-lifecycle.sh [--preflight-only]' >&2; exit 2 ;;
esac
codex_fixture_dir=''
codex_preflight_dir=''
cleanup() {
  [[ -z "$codex_fixture_dir" ]] || rm -rf -- "$codex_fixture_dir"
  [[ -z "$codex_preflight_dir" ]] || rm -rf -- "$codex_preflight_dir"
}
trap cleanup EXIT

if [[ -z "${LOCALROUTER_TEST_CODEX:-}" ]]; then
  if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
    echo 'Set LOCALROUTER_TEST_CODEX to an installed Codex 0.166.0 executable.' >&2
    exit 1
  fi
  codex_fixture_dir="$(mktemp -d)"
  # The official platform bundle includes Code Mode and sandbox resources.
  curl --fail --location --retry 3 \
    'https://github.com/openai/codex/releases/download/rust-v0.166.0/codex-npm-linux-x64-0.166.0.tgz' \
    --output "$codex_fixture_dir/codex.tgz"
  printf '%s  %s\n' 'cbc00a26d3d0e2ceefc4be8891f8a81ef1e349b3b175bc0ae308762bc001fd93' "$codex_fixture_dir/codex.tgz" | sha256sum --check
  tar -xzf "$codex_fixture_dir/codex.tgz" -C "$codex_fixture_dir"
  codex_platform_dir="$codex_fixture_dir/package/vendor/x86_64-unknown-linux-musl"
  test -x "$codex_platform_dir/bin/codex-code-mode-host"
  test -x "$codex_platform_dir/codex-resources/bwrap"
  export LOCALROUTER_TEST_CODEX="$codex_platform_dir/bin/codex"
fi
codex_fixture_version="$("$LOCALROUTER_TEST_CODEX" --version)"
printf '%s\n' "$codex_fixture_version"
if [[ "$codex_fixture_version" != 'codex-cli 0.166.0' ]]; then
  echo 'This regression fixture pins Codex 0.166.0; update the fixture and digest together.' >&2
  exit 1
fi
# Verify the real client sandbox before compiling the Rust fixture. All
# configuration, files and helper aliases belong to this disposable home.
codex_preflight_parent="${XDG_CACHE_HOME:-$HOME/.cache}"
mkdir -p "$codex_preflight_parent"
codex_preflight_dir="$(mktemp -d "$codex_preflight_parent/localrouter-codex-preflight.XXXXXX")"
mkdir -p "$codex_preflight_dir/home" "$codex_preflight_dir/codex" "$codex_preflight_dir/workspace"
run_sandbox_preflight() {
  env -i PATH="$PATH" HOME="$codex_preflight_dir/home" \
    XDG_CONFIG_HOME="$codex_preflight_dir/home/.config" \
    XDG_DATA_HOME="$codex_preflight_dir/home/.local/share" \
    CODEX_HOME="$codex_preflight_dir/codex" \
    "$LOCALROUTER_TEST_CODEX" sandbox --permission-profile :workspace \
    -C "$codex_preflight_dir/workspace" -- /bin/sh -c "$1"
}
run_sandbox_preflight 'printf "%s" sandbox-ok > sandbox-proof'
test "$(cat "$codex_preflight_dir/workspace/sandbox-proof")" = sandbox-ok
if run_sandbox_preflight 'printf "%s" forbidden > ../outside-proof'; then
  echo 'Codex workspace sandbox allowed a write outside its workspace.' >&2
  exit 1
fi
test ! -e "$codex_preflight_dir/outside-proof"
printf '%s\n' 'Codex sandbox preflight passed: workspace write succeeds; outside write denied.'
if [[ "${1:-}" == --preflight-only ]]; then exit 0; fi

# Hosted Linux has no native desktop credential service. The native OS-store
# variant runs explicitly on the desktop; both variants run the complete CLI.
export LOCALROUTER_TEST_KEYCHAIN="${LOCALROUTER_TEST_KEYCHAIN:-mock}"
cargo test -p localrouter --lib launcher::proxy::lifecycle_tests::installed_codex -- --ignored --nocapture --test-threads=1
