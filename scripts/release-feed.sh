#!/bin/sh
# Sign, verify, and publish the ed25519-signed update feeds that Linux and
# Windows managed installs trust. Platform-neutral so CI can sign and publish
# all targets from one Linux job after the native builds finish.
#
#   release-feed.sh sign    <dist> <feed>...  # needs HEX_RELEASE_SIGNING_KEY
#   release-feed.sh verify  <dist> <feed>...  # signature, artifact, monotonic
#   release-feed.sh publish <dist> <feed>...  # artifacts, installers, feeds last
#
# <feed> is linux-update.json, windows-update.json, or
# windows-aarch64-update.json. `sign` reads the unsigned payload a build left
# at <dist>/<feed>.payload.json. Set HEX_VERSION to pin the expected version.
# `publish` uploads with scripts/r2-put.sh to HEX_RELEASE_BASE_URL, whose
# objects live under HEX_RELEASE_PREFIX in the bucket.
set -eu

script_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
source_root=${HEX_RELEASE_SOURCE:-$script_root}
base_url=${HEX_RELEASE_BASE_URL:-https://downloads.hex.anoma.ly}
expected_public_key=bfad02e62208ff144b5c9d21c7e79c7c16c6904299a437d857303007cd4ff7d8
public_key_prefix=302a300506032b6570032100
max_bytes=536870912

usage() {
  echo "Usage: $0 <sign|verify|publish> <dist> <feed>..." >&2
  exit 1
}
[ $# -ge 3 ] || usage
mode=$1
dist=$2
shift 2
case "$mode" in
  sign | verify | publish) ;;
  *) usage ;;
esac

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Release feeds require $1." >&2
    exit 1
  fi
}
for command in cmp curl jq openssl sort xxd; do
  require "$command"
done

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum < "$1" | awk '{ print $1 }'
  else
    shasum -a 256 < "$1" | awk '{ print $1 }'
  fi
}

target_for() {
  case "$1" in
    linux-update.json)
      target=x86_64-unknown-linux-gnu suffix=x86_64-linux
      installer=install-linux.sh installer_source=scripts/install-linux-release.sh
      installer_type=text/x-shellscript
      ;;
    windows-update.json)
      target=x86_64-pc-windows-msvc suffix=x86_64-windows.exe
      installer=install-windows.ps1 installer_source=scripts/install-windows.ps1
      installer_type=text/plain
      ;;
    windows-aarch64-update.json)
      target=aarch64-pc-windows-msvc suffix=aarch64-windows.exe
      installer=install-windows.ps1 installer_source=scripts/install-windows.ps1
      installer_type=text/plain
      ;;
    *)
      echo "Unknown update feed $1." >&2
      exit 1
      ;;
  esac
}
for feed_name in "$@"; do
  target_for "$feed_name"
done

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
public_pem="$temporary/public.pem"
printf '%s%s' "$public_key_prefix" "$expected_public_key" | xxd -r -p > "$temporary/public.der"
openssl pkey -pubin -inform DER -in "$temporary/public.der" -out "$public_pem"

verify_signed_feed() {
  jq -er .payload "$1" | openssl base64 -d -A > "$2"
  jq -er .signature "$1" | openssl base64 -d -A > "$temporary/signature"
  openssl pkeyutl -verify -rawin -pubin -inkey "$public_pem" \
    -in "$2" -sigfile "$temporary/signature" >/dev/null
}

# Checks a payload against its staged artifact; sets version and artifact.
check_payload() {
  payload_path=$1
  test "$(jq -r .schema_version "$payload_path")" = 1
  test "$(jq -r .channel "$payload_path")" = stable
  test "$(jq -r .target "$payload_path")" = "$target"
  version=$(jq -r .version "$payload_path")
  if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "$feed_name names a non-stable version $version." >&2
    exit 1
  fi
  if [ -n "${HEX_VERSION:-}" ] && [ "$version" != "$HEX_VERSION" ]; then
    echo "$feed_name names $version, expected $HEX_VERSION." >&2
    exit 1
  fi
  artifact=$(jq -r .artifact "$payload_path")
  artifact_path="$dist/$artifact"
  test -f "$artifact_path"
  bytes=$(wc -c < "$artifact_path" | tr -d ' ')
  sha256=$(sha256_of "$artifact_path")
  test "$artifact" = "HEX-$version-$sha256-$suffix"
  test "$(jq -r .bytes "$payload_path")" = "$bytes"
  test "$(jq -r .sha256 "$payload_path")" = "$sha256"
  test "$bytes" -le "$max_bytes"
  case "$suffix" in
    *.exe) test "$(head -c 2 "$artifact_path")" = MZ ;;
    *) test "$(head -c 4 "$artifact_path" | xxd -p)" = 7f454c46 ;;
  esac
}

verify_feed() {
  feed_name=$1
  target_for "$feed_name"
  payload="$temporary/$feed_name.payload"
  verify_signed_feed "$dist/$feed_name" "$payload"
  check_payload "$payload"
}

fetch() {
  curl --fail --silent --show-error --proto '=https' --proto-redir '=https' \
    --connect-timeout 10 --max-time "${3:-60}" --retry 3 \
    --header 'Cache-Control: no-cache' "$1" -o "$2"
}

published_version() {
  remote_feed="$temporary/remote-$feed_name"
  status=$(curl --silent --show-error --proto '=https' --proto-redir '=https' \
    --connect-timeout 10 --max-time 60 --retry 3 \
    --header 'Cache-Control: no-cache' --write-out '%{http_code}' \
    --output "$remote_feed" "$base_url/$feed_name?check=$(date +%s)")
  case "$status" in
    200)
      remote_payload="$temporary/remote-$feed_name.payload"
      verify_signed_feed "$remote_feed" "$remote_payload"
      test "$(jq -r .schema_version "$remote_payload")" = 1
      test "$(jq -r .channel "$remote_payload")" = stable
      test "$(jq -r .target "$remote_payload")" = "$target"
      remote_version=$(jq -r .version "$remote_payload")
      printf '%s\n' "$remote_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
      printf '%s\n' "$remote_version"
      ;;
    404) ;;
    *)
      echo "Could not inspect the published $feed_name (HTTP $status)." >&2
      exit 1
      ;;
  esac
}

# A feed already serving these exact bytes is accepted so an interrupted
# publish can be retried; ed25519 signatures are deterministic.
require_newer_than_published() {
  current=$(published_version)
  if [ -n "$current" ] && cmp -s "$dist/$feed_name" "$temporary/remote-$feed_name"; then
    already_published=1
    return 0
  fi
  already_published=0
  if [ -n "$current" ]; then
    newest=$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -1)
    if [ "$current" = "$version" ] || [ "$newest" != "$version" ]; then
      echo "Refusing to replace $feed_name $current with $version." >&2
      exit 1
    fi
  fi
}

if [ "$mode" = sign ]; then
  signing_key=${HEX_RELEASE_SIGNING_KEY:-${HEX_LINUX_SIGNING_KEY:-}}
  if [ -z "$signing_key" ]; then
    echo "Set HEX_RELEASE_SIGNING_KEY to the Ed25519 private PEM." >&2
    exit 1
  fi
  key="$temporary/private.pem"
  (umask 077 && printf '%s\n' "$signing_key" > "$key")
  actual_public_key=$(openssl pkey -in "$key" -pubout -outform DER | tail -c 32 | xxd -p -c 64)
  if [ "$actual_public_key" != "$expected_public_key" ]; then
    echo "The signing key does not match the public key embedded in HEX." >&2
    exit 1
  fi
  for feed_name in "$@"; do
    target_for "$feed_name"
    unsigned="$dist/$feed_name.payload.json"
    check_payload "$unsigned"
    openssl pkeyutl -sign -rawin -inkey "$key" -in "$unsigned" -out "$temporary/signature"
    jq -n \
      --arg payload "$(openssl base64 -A -in "$unsigned")" \
      --arg signature "$(openssl base64 -A -in "$temporary/signature")" \
      '{payload:$payload,signature:$signature}' > "$dist/$feed_name"
    verify_feed "$feed_name"
    echo "$dist/$feed_name"
  done
  exit 0
fi

for feed_name in "$@"; do
  verify_feed "$feed_name"
  require_newer_than_published
  echo "Verified $feed_name: $version, $artifact ($bytes bytes)." >&2
done
[ "$mode" = publish ] || exit 0

put() {
  "$script_root/scripts/r2-put.sh" "$@"
}

# Artifacts are content-addressed and immutable: an existing object must hold
# the same bytes, and is never replaced.
for feed_name in "$@"; do
  verify_feed "$feed_name"
  remote_artifact="$temporary/remote-$artifact"
  status=$(curl --silent --show-error --proto '=https' --proto-redir '=https' \
    --connect-timeout 10 --max-time 600 --retry 3 --write-out '%{http_code}' \
    --output "$remote_artifact" "$base_url/releases/$artifact?check=$(date +%s)")
  case "$status" in
    200)
      if ! cmp -s "$artifact_path" "$remote_artifact"; then
        echo "releases/$artifact already exists with different bytes." >&2
        exit 1
      fi
      echo "releases/$artifact is already published." >&2
      ;;
    404)
      put "releases/$artifact" "$artifact_path" application/octet-stream \
        'public, max-age=31536000, immutable'
      fetch "$base_url/releases/$artifact" "$remote_artifact" 600
      cmp -s "$artifact_path" "$remote_artifact"
      ;;
    *)
      echo "Could not inspect releases/$artifact (HTTP $status)." >&2
      exit 1
      ;;
  esac
done

published_installers=
for feed_name in "$@"; do
  target_for "$feed_name"
  case " $published_installers " in
    *" $installer "*) continue ;;
  esac
  installer_path="$source_root/$installer_source"
  put "$installer" "$installer_path" "$installer_type" 'public, max-age=300' \
    "attachment; filename=\"$installer\""
  fetch "$base_url/$installer?check=$(date +%s)" "$temporary/remote-$installer"
  cmp -s "$installer_path" "$temporary/remote-$installer"
  published_installers="$published_installers $installer"
done

# Feeds are the only mutable trust roots; publish them last, after rechecking.
for feed_name in "$@"; do
  verify_feed "$feed_name"
  require_newer_than_published
  if [ "$already_published" = 0 ]; then
    put "$feed_name" "$dist/$feed_name" application/json no-cache
  fi
  fetch "$base_url/$feed_name?check=$(date +%s)" "$temporary/published-$feed_name"
  cmp -s "$dist/$feed_name" "$temporary/published-$feed_name"
  echo "$base_url/$feed_name -> $version"
  echo "$base_url/releases/$artifact"
done
