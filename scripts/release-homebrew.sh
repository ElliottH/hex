#!/bin/sh
# Renders the Homebrew cask anomalyco/tap/hex (anomalyco/homebrew-tap,
# Casks/hex.rb) from packaging/homebrew/hex.rb, without secrets. The Release
# workflow's Homebrew job pushes it with .github/release/push-homebrew.sh.
#
#   release-homebrew.sh render <version> <dmg-url> <sha256> <out>
#
# The DMG URL is the release service's macos-arm64-dmg file.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
usage() {
  echo "Usage: $0 render <version> <dmg-url> <sha256> <out>" >&2
  exit 1
}

case "${1:-}" in
  render)
    [ $# -eq 5 ] || usage
    version=$2 url=$3 sha256=$4 out=$5
    printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(beta|dev)\.[0-9]+)?$'
    printf '%s\n' "$sha256" | grep -Eq '^[0-9a-f]{64}$'
    printf '%s\n' "$url" | grep -Eq '^https://[A-Za-z0-9./_-]+\.dmg$'
    case "$url" in
      *"$version"*) ;;
      *)
        echo "The DMG URL must name version $version." >&2
        exit 1
        ;;
    esac
    # Homebrew wants the URL to interpolate the version.
    # shellcheck disable=SC2016
    versioned=$(printf '%s' "$url" | sed "s|$(printf '%s' "$version" | sed 's/[.]/\\./g')|#{version}|g")
    sed -e "s|@VERSION@|$version|" -e "s|@SHA256@|$sha256|" -e "s|@URL@|$versioned|" \
      "$root/packaging/homebrew/hex.rb" > "$out"
    if grep -q '@[A-Z0-9_]*@' "$out"; then
      echo "The cask has an unfilled placeholder." >&2
      exit 1
    fi
    echo "$out"
    ;;
  *) usage ;;
esac
