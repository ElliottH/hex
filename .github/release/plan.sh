#!/usr/bin/env bash
# plan.sh: decides what this run builds and where it publishes, from the event
# and the ref alone, and writes the plan to $GITHUB_OUTPUT.
#
#   dev    a push to dev (or a dispatch from dev): 0.0.0-dev-<run number> on
#          the release service's dev channel, as OpenCode numbers dev builds.
#   check  a dispatch from dev: the same build, signed and verified, published
#          nowhere.
#   stable a dispatch from production or a pushed app-v<version> tag: the
#          crate version from Cargo.toml on the stable channel.
#
# Environment: EVENT, REF_NAME, REF_TYPE, INPUT_REF, CHANNEL, SHA, and
# Cargo.toml of the commit in the working directory.
. "$(dirname "$0")/lib.sh"

version=$(awk '/^\[package\]/ { package = 1; next } /^\[/ { package = 0 } package && /^version = "/ { gsub(/^version = "|"$/, ""); print; exit }' Cargo.toml)
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Stable releases need an X.Y.Z crate version, not $version."
case "$CHANNEL" in
  stable)
    if [ "$REF_TYPE" != tag ] && [ "$REF_NAME" != production ]; then die "Stable releases run from production or an app-v tag, not $REF_NAME."; fi
    if [ "$EVENT" = workflow_dispatch ] && [ "$INPUT_REF" != production ] && [ "$INPUT_REF" != "app-v$version" ]; then
      die "Stable releases build ref production or app-v$version, not $INPUT_REF."
    fi
    if [ "$REF_TYPE" = tag ] && [ "$REF_NAME" != "app-v$version" ]; then die "Tag $REF_NAME does not match version $version."; fi
    release_channel=stable release_version=$version tag="app-v$version"
    notes="https://github.com/$GITHUB_REPOSITORY/releases/tag/$tag"
    ;;
  dev | check)
    [ "$REF_NAME" = dev ] || die "The $CHANNEL channel runs from dev, not $REF_NAME."
    if [ "$EVENT" = workflow_dispatch ] && [ "$INPUT_REF" != dev ]; then die "The $CHANNEL channel builds ref dev, not $INPUT_REF."; fi
    # Display text only: installs follow the channel's current release.
    release_channel=dev release_version="0.0.0-dev-$GITHUB_RUN_NUMBER" tag=""
    notes="https://github.com/$GITHUB_REPOSITORY/commit/$SHA"
    ;;
  *) die "Unknown channel $CHANNEL." ;;
esac
{
  echo "sha=$SHA"
  echo "version=$version"
  echo "channel=$CHANNEL"
  echo "service=https://anoma.ly/release"
  echo "tag=$tag"
  echo "notes-url=$notes"
  echo "release-channel=$release_channel"
  echo "release-version=$release_version"
} >> "$GITHUB_OUTPUT"
echo "Releasing $version ($CHANNEL) from $SHA as $release_channel $release_version." >> "$GITHUB_STEP_SUMMARY"
