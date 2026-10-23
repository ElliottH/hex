#!/bin/sh
# Publish one release built and signed by the Release workflow. <dist> holds
# the signed Linux and Windows feeds with their artifacts and Setup.exe
# installers, and the macOS DMG, Sparkle ZIP, and signed appcast.xml.
#
#   publish-release.sh <dist>
#
# Environment:
#   HEX_VERSION            the release version
#   HEX_RELEASE_COMMIT     the commit it was built from
#   HEX_RELEASE_CHANNEL    stable, or staging for a dry run
#   HEX_RELEASE_BASE_URL   the public origin objects are verified at
#   HEX_RELEASE_PREFIX     their key prefix ('' for stable, staging/<run>/)
#   HEX_RELEASE_SOURCE     a checkout of HEX_RELEASE_COMMIT (installers, notes)
#   HEX_RELEASE_TAG        the GitHub release tag
#   R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, GH_TOKEN
#
# Immutable objects never replace different bytes. Mutable pointers move
# last: the Linux and Windows feeds, then HEX-latest-arm64.dmg, then the
# appcast. A stable release needs every platform; staging publishes what the
# run produced, as a draft GitHub release.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
repo=${HEX_RELEASE_REPO:-anomalyco/hex}
feeds="linux-update.json windows-update.json windows-aarch64-update.json"
[ $# -eq 1 ] || {
  echo "Usage: $0 <dist>" >&2
  exit 1
}
dist=$1
version=${HEX_VERSION:?Set HEX_VERSION}
commit=${HEX_RELEASE_COMMIT:?Set HEX_RELEASE_COMMIT}
channel=${HEX_RELEASE_CHANNEL:?Set HEX_RELEASE_CHANNEL}
base_url=${HEX_RELEASE_BASE_URL:?Set HEX_RELEASE_BASE_URL}
source=${HEX_RELEASE_SOURCE:?Set HEX_RELEASE_SOURCE}
tag=${HEX_RELEASE_TAG:?Set HEX_RELEASE_TAG}
printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
case "$channel:${HEX_RELEASE_PREFIX:-}" in
  stable:) ;;
  staging:staging/*/) ;;
  *)
    echo "Channel $channel does not match prefix '${HEX_RELEASE_PREFIX:-}'." >&2
    exit 1
    ;;
esac
for command in cmp curl gh jq openssl sha256sum; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Publishing requires $command." >&2
    exit 1
  }
done

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

fetch() {
  curl --silent --show-error --proto '=https' --connect-timeout 10 --max-time 600 \
    --retry 3 --header 'Cache-Control: no-cache' --write-out '%{http_code}' \
    --output "$2" "$base_url/$1?check=$(date +%s)"
}

# put_immutable <key> <file> <content-type> [content-disposition]
put_immutable() {
  remote="$temporary/remote"
  rm -f "$remote"
  status=$(fetch "$1" "$remote")
  case "$status" in
    200)
      cmp -s "$2" "$remote" || {
        echo "$1 already exists with different bytes." >&2
        exit 1
      }
      echo "$1 is already published." >&2
      ;;
    404)
      "$root/scripts/r2-put.sh" "$1" "$2" "$3" 'public, max-age=31536000, immutable' ${4:+"$4"}
      test "$(fetch "$1" "$remote")" = 200
      cmp -s "$2" "$remote"
      ;;
    *)
      echo "Could not inspect $1 (HTTP $status)." >&2
      exit 1
      ;;
  esac
}

# put_pointer <key> <file> <content-type>
put_pointer() {
  "$root/scripts/r2-put.sh" "$1" "$2" "$3" no-cache
  test "$(fetch "$1" "$temporary/pointer")" = 200
  cmp -s "$2" "$temporary/pointer"
}

# Everything is checked before anything is uploaded.
# shellcheck disable=SC2086
HEX_VERSION="$version" HEX_RELEASE_SOURCE="$source" \
  "$root/scripts/release-feed.sh" verify "$dist" $feeds
linux_artifact=$(jq -r .payload "$dist/linux-update.json" | openssl base64 -d -A | jq -r .artifact)
cp "$dist/$linux_artifact" "$temporary/HEX-$version-x86_64-linux"
for label in x64 arm64; do
  setup="$dist/HEX-Setup-$version-$label.exe"
  test -s "$setup"
  test "$(head -c 2 "$setup")" = MZ
done
dmg="$dist/HEX-$version-arm64.dmg"
zip="$dist/HEX-$version-arm64.zip"
appcast="$dist/appcast.xml"
macos=0
if [ -f "$dmg" ] || [ -f "$zip" ] || [ -f "$appcast" ]; then
  test -s "$dmg" && test -s "$zip" && test -s "$appcast"
  grep -Fq "$base_url/releases/HEX-$version-arm64.zip" "$appcast"
  grep -Fq 'sparkle:edSignature=' "$appcast"
  macos=1
elif [ "$channel" = stable ]; then
  echo "A stable release needs the macOS DMG, Sparkle ZIP, and appcast." >&2
  exit 1
else
  echo "::warning::No macOS build in this run; staging publishes Linux and Windows only."
fi
notes="$source/docs/releases/$version.md"
if [ "$channel" = stable ]; then
  test -f "$notes" || {
    echo "Add release notes at docs/releases/$version.md." >&2
    exit 1
  }
fi

for label in x64 arm64; do
  name="HEX-Setup-$version-$label.exe"
  put_immutable "releases/$name" "$dist/$name" application/vnd.microsoft.portable-executable \
    "attachment; filename=\"$name\""
done
if [ "$macos" = 1 ]; then
  put_immutable "releases/HEX-$version-arm64.dmg" "$dmg" application/x-apple-diskimage
  put_immutable "releases/HEX-$version-arm64.zip" "$zip" application/zip
fi

# The GitHub release mirrors the downloads byte for byte. Staging stays a
# draft; app releases stay off "Latest" (see AGENTS.md).
assets="$dist/HEX-Setup-$version-x64.exe $dist/HEX-Setup-$version-arm64.exe $temporary/HEX-$version-x86_64-linux"
[ "$macos" = 0 ] || assets="$assets $dmg"
if ! gh release view "$tag" -R "$repo" --json name >/dev/null 2>&1; then
  if [ "$channel" = stable ]; then
    gh release create "$tag" -R "$repo" --target "$commit" --title "Hex $version" \
      --notes-file "$notes" --latest=false
  else
    printf 'Staging build of Hex %s from %s. Downloads: %s\n' "$version" "$commit" "$base_url" > "$temporary/notes.md"
    gh release create "$tag" -R "$repo" --target "$commit" --title "Hex $version (staging)" \
      --notes-file "$temporary/notes.md" --draft --prerelease
  fi
fi
gh release view "$tag" -R "$repo" --json assets > "$temporary/release.json"
mkdir -p "$temporary/mirror"
for path in $assets; do
  name=$(basename "$path")
  if ! jq -e --arg name "$name" '.assets | any(.name == $name)' "$temporary/release.json" >/dev/null; then
    gh release upload "$tag" "$path" -R "$repo"
  fi
  gh release download "$tag" -R "$repo" --pattern "$name" --dir "$temporary/mirror" --clobber
  cmp -s "$path" "$temporary/mirror/$name" || {
    echo "GitHub's $name differs from the release bytes." >&2
    exit 1
  }
  rm -f "$temporary/mirror/$name"
done

# Content-addressed artifacts, installers, then the feeds.
# shellcheck disable=SC2086
HEX_VERSION="$version" HEX_RELEASE_SOURCE="$source" \
  "$root/scripts/release-feed.sh" publish "$dist" $feeds

if [ "$macos" = 1 ]; then
  put_pointer releases/HEX-latest-arm64.dmg "$dmg" application/x-apple-diskimage
  put_pointer appcast.xml "$appcast" application/xml
fi

{
  echo "## Hex $version ($channel)"
  echo
  echo "- Downloads: $base_url"
  echo "- GitHub release: https://github.com/$repo/releases/tag/$tag"
  for label in x64 arm64; do
    echo "- Windows $label: $base_url/releases/HEX-Setup-$version-$label.exe"
  done
  echo "- Linux feed: $base_url/linux-update.json"
  [ "$macos" = 0 ] || echo "- macOS: $base_url/releases/HEX-$version-arm64.dmg, appcast $base_url/appcast.xml"
} | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"
