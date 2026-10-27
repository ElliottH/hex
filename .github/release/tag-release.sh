#!/usr/bin/env bash
# tag-release.sh: pushes the lightweight tag HEX_RELEASE_TAG at the released
# commit (the checked-out HEAD, SHA) with the release deploy key,
# github-actions-hex-release-tag (HEX_RELEASE_TAG_KEY). The "release tags"
# ruleset lets only Dax and deploy keys create, move, or delete v* tags.
# Release runs it last, after every platform published, so a failed release
# leaves no tag. A rerun accepts the tag when it already names SHA. A check run
# (HEX_RELEASE_TAG_TEST=1) pushes a throwaway tag and deletes it again.
# Runs git only, with hooks off.
. "$(dirname "$0")/lib.sh"

tag=$HEX_RELEASE_TAG
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-test-[0-9]+)?$ ]] || die "Unexpected tag $tag."
[ "$(git rev-parse HEAD)" = "$SHA" ] || die "The checkout is not the released commit $SHA."
: "${HEX_RELEASE_TAG_KEY:?Set HEX_RELEASE_TAG_KEY to the release deploy key}"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
(umask 077 && printf '%s\n' "$HEX_RELEASE_TAG_KEY" > "$temporary/key")
# github.com's published ED25519 host key.
echo 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl' > "$temporary/known_hosts"
export GIT_SSH_COMMAND="ssh -i $temporary/key -o IdentitiesOnly=yes -o UserKnownHostsFile=$temporary/known_hosts -o StrictHostKeyChecking=yes"
remote="git@github.com:$GITHUB_REPOSITORY.git"
git() { command git -c core.hooksPath=/dev/null "$@"; }

existing=$(git ls-remote "$remote" "refs/tags/$tag" | cut -f1)
if [ -n "$existing" ]; then
  [ "$existing" = "$SHA" ] || die "Tag $tag already names $existing, not $SHA."
  echo "Tag $tag already names $SHA."
else
  git push -q "$remote" "$SHA:refs/tags/$tag"
  [ "$(git ls-remote "$remote" "refs/tags/$tag" | cut -f1)" = "$SHA" ] || die "Tag $tag did not land."
  echo "Tagged $SHA as $tag."
fi
if [ "${HEX_RELEASE_TAG_TEST:-}" = 1 ]; then
  git push -q "$remote" ":refs/tags/$tag"
  [ -z "$(git ls-remote "$remote" "refs/tags/$tag")" ] || die "Could not delete the test tag $tag."
  echo "Deleted the test tag $tag."
fi
