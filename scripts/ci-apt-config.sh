#!/usr/bin/env bash
# Make apt-get on GitHub's Ubuntu runners fail fast instead of hanging.
#
# The runners resolve packages through a mirror list (/etc/apt/apt-mirrors.txt)
# that puts the Azure mirror first. When that mirror stalls, apt-get waits on
# it until the job times out. Drop it when the list has another mirror to use,
# and bound every fetch so a stalled mirror errors out and is retried.
set -euo pipefail

mirrors=/etc/apt/apt-mirrors.txt
if [ -f "$mirrors" ] && grep -qv 'azure\.archive\.ubuntu\.com' "$mirrors"; then
  sudo sed -i '/azure\.archive\.ubuntu\.com/d' "$mirrors"
fi

printf '%s\n' \
  'Acquire::Retries "3";' \
  'Acquire::http::Timeout "30";' \
  'Acquire::https::Timeout "30";' |
  sudo tee /etc/apt/apt.conf.d/99-ci-timeouts >/dev/null
