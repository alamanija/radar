#!/usr/bin/env bash
# Prints the updater manifest (`latest.json`) for a release to stdout.
#
#   make-latest-json.sh <version> <owner/repo> <artifact-dir>
#
# Env: RELEASE_NOTES (default "See the assets below to download."),
#      PUB_DATE (RFC 3339; default: now).
#
# <artifact-dir> holds the cargo-packager outputs and their `.sig` files:
#   *-setup.exe(.sig)      -> windows-x86_64   (format nsis)
#   *.app.tar.gz(.sig)     -> darwin-aarch64   (format app)
#
# The shape (version, notes, pub_date, platforms.<os>-<arch>.{signature,url})
# stays readable by the updater in already-installed older builds;
# cargo-packager-updater additionally needs `format`, which older builds
# ignore. Needs `jq`. Fails if either platform is missing, so a
# half-built release is never published.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <version> <owner/repo> <artifact-dir>" >&2
  exit 2
fi
version=$1
repo=$2
dir=$3
notes=${RELEASE_NOTES:-"See the assets below to download."}
pub_date=${PUB_DATE:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}
base="https://github.com/${repo}/releases/download/v${version}"

# Prints the single file in $dir matching the glob $1, or fails.
find_one() {
  local matches=()
  shopt -s nullglob
  matches=("$dir"/$1)
  shopt -u nullglob
  if [ "${#matches[@]}" -ne 1 ]; then
    echo "expected exactly one '$1' in $dir, found ${#matches[@]}" >&2
    return 1
  fi
  basename "${matches[0]}"
}

# entry <glob> <format>: one platform object as JSON.
entry() {
  local file sig
  file=$(find_one "$1")
  [ -f "$dir/$file.sig" ] || { echo "missing $file.sig" >&2; return 1; }
  sig=$(tr -d '\r\n' < "$dir/$file.sig")
  jq -n --arg signature "$sig" --arg url "$base/$file" --arg format "$2" \
    '{signature: $signature, url: $url, format: $format}'
}

win=$(entry '*-setup.exe' nsis)
mac=$(entry '*.app.tar.gz' app)

jq -n --arg version "$version" --arg notes "$notes" --arg pub_date "$pub_date" \
  --argjson win "$win" --argjson mac "$mac" \
  '{version: $version, notes: $notes, pub_date: $pub_date,
    platforms: {"windows-x86_64": $win, "darwin-aarch64": $mac}}'
