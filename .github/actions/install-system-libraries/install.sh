#!/usr/bin/env bash
# Install apt packages on a GitHub-hosted Ubuntu runner, without Azure's Ubuntu mirror
# when it is down.
#
# The runner image points apt at a list of mirrors, /etc/apt/apt-mirrors.txt, with
# Azure's first and archive.ubuntu.com and security.ubuntu.com after it (runner-images,
# images/ubuntu/scripts/build/configure-apt-sources.sh). When Azure's does not answer,
# apt waits out its timeout and retries for every index file before it moves on, which
# is longer than a step may take. So ask the mirror once, and if it is silent take it
# off the list. An image without that list, or without Azure's mirror in it, is left
# as it is.
#
# PACKAGES: package names separated by single spaces. Empty: only the mirror check, for
# a job whose later steps run apt themselves.
set -euo pipefail

mirrors=/etc/apt/apt-mirrors.txt
azure=http://azure.archive.ubuntu.com/ubuntu/

if [ -f "$mirrors" ] && grep -q "^$azure" "$mirrors" &&
  grep -v "^$azure" "$mirrors" | grep -q '[^[:space:]]'; then
  # shellcheck source=/dev/null
  codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
  if ! curl -fsS --max-time 10 -o /dev/null "${azure}dists/${codename}/InRelease"; then
    echo "::warning::${azure} does not answer: apt uses the other mirrors in ${mirrors}"
    sudo sed -i "\|^${azure}|d" "$mirrors"
  fi
fi

[ -n "${PACKAGES:-}" ] || exit 0
if ! [[ $PACKAGES =~ ^[a-z0-9][a-z0-9.+-]*(\ [a-z0-9][a-z0-9.+-]*)*$ ]]; then
  echo "::error::packages must be package names separated by single spaces"
  exit 1
fi
read -ra packages <<<"$PACKAGES"
apt=(sudo apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
"${apt[@]}" update
"${apt[@]}" install -y "${packages[@]}"
