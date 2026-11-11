#!/usr/bin/env bash
# Install apt packages on a GitHub-hosted Ubuntu runner, also when the mirrors the
# runner image names do not answer.
#
# The image points apt at a list of mirrors, /etc/apt/apt-mirrors.txt: Azure's first,
# then archive.ubuntu.com and security.ubuntu.com (runner-images,
# images/ubuntu/scripts/build/configure-apt-sources.sh). When one does not answer, apt
# waits out its timeout and retries for every index file before it moves on, which is
# longer than a step may take; and on 2026-11-11 none of the three answered from the
# runners. So:
#
# 1. If every package asked for is installed already, apt is not run at all.
# 2. Otherwise, or with no packages, the first mirror of the list is asked once. If it
#    is silent, the first mirror that answers, of the list and then of OTHERS, becomes
#    the list's only entry. An image without that list is left as it is.
#
# Any mirror is as safe as the image's own: apt checks the archive's signature on every
# InRelease with the keys the image ships, and nothing here changes that (no key, no
# trusted=yes, no --allow-unauthenticated). A mirror can withhold packages, it cannot
# alter them. That is also why http is enough.
#
# PACKAGES: package names separated by single spaces.
# MIRROR_ONLY=true: only the mirror choice, for a job whose later steps run apt
# themselves. The same with no packages.
set -euo pipefail

mirrors=/etc/apt/apt-mirrors.txt
sources=/etc/apt/sources.list.d/ubuntu.sources
# Official Ubuntu archive mirrors outside Canonical's and Azure's networks, from
# https://launchpad.net/ubuntu/+archivemirrors. They carry amd64 and i386 only.
OTHERS=(
  http://mirrors.us.kernel.org/ubuntu/
  http://mirror.math.princeton.edu/pub/ubuntu/
  http://ftp.halifax.rwth-aachen.de/ubuntu/
)

packages=()
if [ "${MIRROR_ONLY:-false}" != true ] && [ -n "${PACKAGES:-}" ]; then
  if ! [[ $PACKAGES =~ ^[a-z0-9][a-z0-9.+-]*(\ [a-z0-9][a-z0-9.+-]*)*$ ]]; then
    echo "::error::packages must be package names separated by single spaces"
    exit 1
  fi
  read -ra packages <<<"$PACKAGES"
  missing=()
  for package in "${packages[@]}"; do
    # shellcheck disable=SC2016 # dpkg-query's format, not the shell's
    status=$(dpkg-query -W -f='${Status}' "$package" 2>/dev/null || true)
    [ "$status" = "install ok installed" ] || missing+=("$package")
  done
  if [ ${#missing[@]} -eq 0 ]; then
    echo "Already installed, apt is not needed: ${packages[*]}"
    exit 0
  fi
  echo "Not installed yet: ${missing[*]}"
fi

# Whether a mirror serves the release and its security updates. At most 10 s.
answers() {
  local suite
  for suite in "$codename" "$codename-security"; do
    curl -fsSL --connect-timeout 3 --max-time 5 -o /dev/null "${1%/}/dists/$suite/InRelease" || return 1
  done
}

if [ -f "$mirrors" ]; then
  # shellcheck source=/dev/null
  codename=$(. /etc/os-release && echo "${VERSION_CODENAME:-}")
  mapfile -t listed < <(awk '$1 ~ /^https?:\/\// { print $1 }' "$mirrors")
  # Without a release name there is nothing to ask a mirror for: the list stays.
  if [ -n "$codename" ] && [ ${#listed[@]} -gt 0 ] && ! answers "${listed[0]}"; then
    candidates=("${listed[@]:1}")
    case $(dpkg --print-architecture) in amd64 | i386) candidates+=("${OTHERS[@]}") ;; esac
    chosen=
    for mirror in "${candidates[@]}"; do
      if answers "$mirror"; then
        chosen=$mirror
        break
      fi
    done
    if [ -z "$chosen" ]; then
      echo "::error::no Ubuntu mirror answers: ${listed[0]} ${candidates[*]}"
      exit 1
    fi
    echo "::warning::${listed[0]} does not answer: apt uses ${chosen}"
    printf '%s\tpriority:1\n' "$chosen" | sudo tee "$mirrors" >/dev/null
    # A stanza that names an ubuntu.com host itself, as security updates may, goes
    # through the list too.
    if [ -f "$sources" ]; then
      sudo sed -i -E "s#^(URIs:[[:space:]]*)https?://[a-z.]+\.ubuntu\.com/ubuntu/?[[:space:]]*\$#\1mirror+file:${mirrors}#" "$sources"
    fi
  fi
fi

[ ${#packages[@]} -gt 0 ] || exit 0
apt=(sudo apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
"${apt[@]}" update
"${apt[@]}" install -y "${packages[@]}"
