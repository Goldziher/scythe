#!/usr/bin/env bash
# ~keep Move the major-version tag (v0, v1, ...) onto a release tag's commit.
#
# ~keep It cannot `git push`: GITHUB_TOKEN is never allowed to push or create a ref
# ~keep that introduces .github/workflows changes ("refusing to allow a GitHub App ...
# ~keep without `workflows` permission", or a 403 from the refs API), and every
# ~keep release commit changes them. The tap PAT holds that scope, so it is tried
# ~keep first. A failure warns instead of failing the run: by this point the archives,
# ~keep every registry and the channel gate have already passed.
set -euo pipefail

tag="${1:?usage: move-major-tag.sh <release-tag> <version>}"
version="${2:?usage: move-major-tag.sh <release-tag> <version>}"

major="v${version%%.*}"
sha="$(git rev-list -n1 "$tag")"
repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
ref="refs/tags/${major}"

move() {
  GH_TOKEN="$1" gh api -X PATCH "repos/$repo/git/refs/tags/$major" -f sha="$sha" -F force=true >/dev/null 2>&1 ||
    GH_TOKEN="$1" gh api -X POST "repos/$repo/git/refs" -f ref="$ref" -f sha="$sha" >/dev/null 2>&1
}

for token in "${HOMEBREW_TOKEN:-}" "${GITHUB_TOKEN:-}"; do
  [ -n "$token" ] || continue
  if move "$token"; then
    echo "Moved $major to $tag ($sha)"
    exit 0
  fi
done

echo "::warning::could not move $major to $sha ($tag); move it with a token that has the workflows permission"
