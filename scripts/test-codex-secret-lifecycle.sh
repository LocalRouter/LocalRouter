#!/usr/bin/env bash
# Runs the actual Codex CLI against the local TLS provider and real AppFirewall.
# Provider auth is synthetic and public destinations are rejected by the fixture.
set -euo pipefail

if [[ -z "${LOCALROUTER_TEST_CODEX:-}" ]]; then
  if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
    echo 'Set LOCALROUTER_TEST_CODEX to an installed Codex 0.166.0 executable.' >&2
    exit 1
  fi
  codex_fixture_dir="$(mktemp -d)"
  trap 'rm -rf "$codex_fixture_dir"' EXIT
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
# Hosted Linux has no native desktop credential service. The native OS-store
# variant runs explicitly on the desktop; both variants run the complete CLI.
export LOCALROUTER_TEST_KEYCHAIN="${LOCALROUTER_TEST_KEYCHAIN:-mock}"
cargo test -p localrouter --lib launcher::proxy::lifecycle_tests::installed_codex -- --ignored --nocapture --test-threads=1
