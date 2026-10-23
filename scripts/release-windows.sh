#!/bin/sh
# Build, sign, and publish a Windows release, mirroring release-linux.sh:
# a content-addressed hex.exe plus an ed25519-signed update feed that
# managed installs verify before self-updating. Run from Git Bash on Windows,
# once per architecture: HEX_WINDOWS_ARCH=x86_64 (default) or aarch64.
# x86_64 needs the Vulkan SDK; aarch64 needs the Rust aarch64-pc-windows-msvc
# target plus clang-cl and Ninja (Visual Studio's LLVM component), and is
# best built natively on an ARM64 Windows host. Locally, `prepare` builds and
# signs; CI runs `build` here and signs and publishes every platform with
# scripts/release-feed.sh.
set -eu

script_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
root=${HEX_RELEASE_SOURCE:-$script_root}
mode=${1:-prepare}
version=${HEX_VERSION:-$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" | jq -r '.packages[0].version')}
dist=${HEX_RELEASE_DIST:-"$root/dist"}
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
arch=${HEX_WINDOWS_ARCH:-x86_64}
case "$arch" in
  x86_64) feed_name=windows-update.json ;;
  aarch64) feed_name=windows-aarch64-update.json ;;
  *)
    echo "HEX_WINDOWS_ARCH must be x86_64 or aarch64." >&2
    exit 1
    ;;
esac
triple="$arch-pc-windows-msvc"
suffix="$arch-windows.exe"
prepared_commit="$dist/windows-$arch-release-commit"
installer="$root/scripts/install-windows.ps1"
release_feed="$script_root/scripts/release-feed.sh"

case "$mode" in
  build | sign | prepare | publish) ;;
  *)
    echo "Usage: $0 [build|sign|prepare|publish]" >&2
    exit 1
    ;;
esac
require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Windows releases require $1." >&2
    exit 1
  fi
}
for command in cargo curl jq openssl sha256sum sort xxd; do
  require "$command"
done
case "$(uname -s)" in
  MINGW64* | MSYS*) ;;
  *)
    echo "Windows releases must be built from Git Bash on Windows." >&2
    exit 1
    ;;
esac
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "Stable Windows releases require a non-prerelease semantic version." >&2
  exit 1
fi
if [ -n "$(git -C "$root" status --porcelain)" ]; then
  echo "Commit or remove all working-tree changes before releasing." >&2
  exit 1
fi
test -f "$installer"
# PowerShell needs a Windows path; Git Bash converts only bare arguments.
installer_windows=$(cygpath -w "$installer" 2>/dev/null || printf '%s' "$installer")
powershell -NoProfile -Command "[void][ScriptBlock]::Create((Get-Content -Raw '$installer_windows'))"
# CI checks out an exact commit from GitHub, so it is pushed by construction.
if [ "${GITHUB_ACTIONS:-}" != true ]; then
  upstream=$(git -C "$root" rev-parse --abbrev-ref '@{upstream}' 2>/dev/null || true)
  if [ -z "$upstream" ] || [ "$(git -C "$root" rev-parse HEAD)" != "$(git -C "$root" rev-parse "$upstream")" ]; then
    echo "Push the release commit to its upstream branch before releasing." >&2
    exit 1
  fi
fi

mkdir -p "$dist"
# Git Bash has no flock; an atomic mkdir with a PID record serves as the
# release lock, and a lock whose holder died (kill -9, power loss) is
# reclaimed instead of wedging future releases.
lock="$dist/.release-windows-$arch.lock"
acquire_lock() {
  if mkdir "$lock" 2>/dev/null; then
    printf '%s\n' "$$" > "$lock/pid"
    return 0
  fi
  holder=$(cat "$lock/pid" 2>/dev/null || true)
  if [ -z "$holder" ] || ! kill -0 "$holder" 2>/dev/null; then
    rm -rf "$lock"
    if mkdir "$lock" 2>/dev/null; then
      printf '%s\n' "$$" > "$lock/pid"
      return 0
    fi
  fi
  return 1
}
if ! acquire_lock; then
  echo "Another Windows release is in progress on this host." >&2
  exit 1
fi
trap 'rm -rf "$lock" 2>/dev/null' EXIT HUP INT TERM
export HEX_VERSION="$version" HEX_RELEASE_SOURCE="$root"

check_artifact() {
  artifact=$(jq -r .artifact "$1")
  artifact_path="$dist/$artifact"
  test -f "$artifact_path"
  test "$(head -c 2 "$artifact_path")" = MZ
  # An artifact for the other architecture cannot run here; its version is
  # checked by the installer on the target machine.
  if [ "$(uname -m)" = "$arch" ]; then
    reported_version=$("$artifact_path" --version | awk 'NF { value=$NF } END { print value }')
    test "$reported_version" = "$version"
  fi
}

build() {
  # Portable native code: ggml otherwise tunes its CPU kernels to this build
  # machine (for example AVX-512), and the release would crash on older CPUs.
  cmake_args="-DGGML_NATIVE=OFF"
  if [ "$arch" = aarch64 ]; then
    # ggml's ARM CPU backend refuses MSVC; clang-cl uses LLVM codegen, so
    # ggml's own threadpool replaces OpenMP there.
    cmake_args="$cmake_args -DGGML_OPENMP=OFF"
    export CC=clang-cl CXX=clang-cl CMAKE_GENERATOR=Ninja
  fi
  # Cargo reads .cargo/config.toml (the static Windows runtime) from the
  # working directory, not from --manifest-path.
  (cd "$root" && TRANSCRIBE_CMAKE_ARGS="$cmake_args" \
    cargo build --locked --release --target "$triple")
  built="$target_dir/$triple/release/voice-control.exe"
  sha256=$(sha256sum < "$built" | awk '{ print $1 }')
  artifact="HEX-$version-$sha256-$suffix"
  artifact_path="$dist/$artifact"
  cp "$built" "$artifact_path"
  bytes=$(wc -c < "$artifact_path" | tr -d ' ')
  if [ "$bytes" -gt 536870912 ]; then
    echo "The Windows artifact exceeds the updater's 512 MiB limit." >&2
    exit 1
  fi
  jq -cn \
    --arg version "$version" \
    --arg artifact "$artifact" \
    --arg sha256 "$sha256" \
    --argjson bytes "$bytes" \
    --arg target "$triple" \
    '{schema_version:1,channel:"stable",target:$target,version:$version,artifact:$artifact,bytes:$bytes,sha256:$sha256}' \
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
