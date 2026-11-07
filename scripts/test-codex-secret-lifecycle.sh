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
  download_component() {
    local component="$1" digest="$2"
    curl --fail --location --retry 3 \
      "https://github.com/openai/codex/releases/download/rust-v0.166.0/${component}-x86_64-unknown-linux-musl.tar.gz" \
      --output "$codex_fixture_dir/$component.tar.gz"
    # GitHub's digest for each pinned upstream release asset.
    printf '%s  %s\n' "$digest" "$codex_fixture_dir/$component.tar.gz" | sha256sum --check
    tar -xzf "$codex_fixture_dir/$component.tar.gz" -C "$codex_fixture_dir"
    mv "$codex_fixture_dir/${component}-x86_64-unknown-linux-musl" "$codex_fixture_dir/$component"
  }
  download_component codex a0bf572d2f93f056c6712a6da348e96078dc84f3885fd7be67807ef389cb3978
  download_component codex-code-mode-host 7588334fd47bc4003d970b70dfd4d1e4578491b1e9baa72976a4c2d7e97590da
  download_component codex-exec-server 16f698d576fcd1d94e5e38b5c6ce48168afad1d95737b1eb2d84b3f0219ea6e3
  export LOCALROUTER_TEST_CODEX="$codex_fixture_dir/codex"
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
