#!/bin/sh
# Build, sign, and publish the Linux release: a content-addressed executable
# plus the ed25519-signed linux-update.json. Locally, `prepare` builds and
# signs; `publish` uploads after validation. CI runs `build` here and signs
# and publishes every platform with scripts/release-feed.sh.
set -eu

script_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
root=${HEX_RELEASE_SOURCE:-$script_root}
mode=${1:-prepare}
version=${HEX_VERSION:-$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" | jq -r '.packages[0].version')}
dist=${HEX_RELEASE_DIST:-"$root/dist"}
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
feed_name=linux-update.json
prepared_commit="$dist/linux-release-commit"
installer="$root/scripts/install-linux-release.sh"
release_feed="$script_root/scripts/release-feed.sh"
expected_public_key=bfad02e62208ff144b5c9d21c7e79c7c16c6904299a437d857303007cd4ff7d8

case "$mode" in
  build | sign | prepare | publish) ;;
  *)
    echo "Usage: $0 [build|sign|prepare|publish]" >&2
    exit 1
    ;;
esac
require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Linux releases require $1." >&2
    exit 1
  fi
}
for command in cargo curl file flock jq openssl sha256sum sort xxd; do
  require "$command"
done
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
  echo "Linux releases must be built on x86_64 Linux." >&2
  exit 1
fi
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "Stable Linux releases require a non-prerelease semantic version." >&2
  exit 1
fi
if [ -n "$(git -C "$root" status --porcelain)" ]; then
  echo "Commit or remove all working-tree changes before releasing." >&2
  exit 1
fi
# CI checks out an exact commit from GitHub, so it is pushed by construction.
if [ "${GITHUB_ACTIONS:-}" != true ]; then
  upstream=$(git -C "$root" rev-parse --abbrev-ref '@{upstream}' 2>/dev/null || true)
  if [ -z "$upstream" ] || [ "$(git -C "$root" rev-parse HEAD)" != "$(git -C "$root" rev-parse "$upstream")" ]; then
    echo "Push the release commit to its upstream branch before releasing." >&2
    exit 1
  fi
fi
test -f "$installer"
sh -n "$installer"
grep -Fq "$expected_public_key" "$installer"

mkdir -p "$dist"
exec 9> "$dist/.release-linux.lock"
if ! flock -n 9; then
  echo "Another Linux release is in progress on this host." >&2
  exit 1
fi
export HEX_VERSION="$version" HEX_RELEASE_SOURCE="$root"

check_artifact() {
  artifact=$(jq -r .artifact "$1")
  artifact_path="$dist/$artifact"
  test -x "$artifact_path"
  file "$artifact_path" | grep -E 'ELF 64-bit.*x86-64' >/dev/null
  reported_version=$("$artifact_path" --version | awk 'NF { value=$NF } END { print value }')
  test "$reported_version" = "$version"
}

build() {
  require python3
  "$root/scripts/test-install-linux-release.sh"
  # Portable native code: ggml otherwise tunes its CPU kernels to the build
  # machine (for example AVX-512), and the release would crash on older CPUs.
  (cd "$root" && TRANSCRIBE_CMAKE_ARGS="-DGGML_NATIVE=OFF${TRANSCRIBE_CMAKE_ARGS:+ $TRANSCRIBE_CMAKE_ARGS}" \
    cargo build --locked --release)
  built="$target_dir/release/voice-control"
  sha256=$(sha256sum < "$built" | awk '{ print $1 }')
  artifact="HEX-$version-$sha256-x86_64-linux"
  artifact_path="$dist/$artifact"
  install -m755 "$built" "$artifact_path"
  bytes=$(wc -c < "$artifact_path" | tr -d ' ')
  if [ "$bytes" -gt 536870912 ]; then
    echo "The Linux artifact exceeds the updater's 512 MiB limit." >&2
    exit 1
  fi
  jq -cn \
    --arg version "$version" \
    --arg artifact "$artifact" \
    --arg sha256 "$sha256" \
    --argjson bytes "$bytes" \
    '{schema_version:1,channel:"stable",target:"x86_64-unknown-linux-gnu",version:$version,artifact:$artifact,bytes:$bytes,sha256:$sha256}' \
    > "$dist/$feed_name.payload.json"
  check_artifact "$dist/$feed_name.payload.json"
  echo "$artifact_path"
}

case "$mode" in
  build)
    build
    ;;
  sign)
    "$release_feed" sign "$dist" "$feed_name"
    check_artifact "$dist/$feed_name.payload.json"
    ;;
  prepare)
    build
    "$release_feed" sign "$dist" "$feed_name"
    "$release_feed" verify "$dist" "$feed_name"
    git -C "$root" rev-parse HEAD > "$prepared_commit"
    echo "Prepared but not published. Run $0 publish after validation." >&2
    ;;
  publish)
    if [ ! -f "$dist/$feed_name" ] || [ ! -f "$prepared_commit" ]; then
      echo "Prepare and validate version $version before publishing it." >&2
      exit 1
    fi
    test "$(cat "$prepared_commit")" = "$(git -C "$root" rev-parse HEAD)"
    "$release_feed" publish "$dist" "$feed_name"
    ;;
esac
