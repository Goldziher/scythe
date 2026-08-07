#!/usr/bin/env bash
# One-time manual publish of the `scythe-cli` npm package.
#
# WHY THIS EXISTS
#
# The release workflow publishes npm via OIDC trusted publishing, with no
# NPM_TOKEN in the repository. But a trusted publisher can only be configured
# on npmjs.com for a package that ALREADY EXISTS, and `scythe-cli` has never
# been published -- so OIDC cannot create it. The first publish has to come
# from a logged-in machine. This is the same constraint that broke the v0.10.0
# and v0.11.0 crates.io releases; PyPI is the exception, because its "pending
# publisher" feature can bootstrap a name that does not exist yet.
#
# After this script succeeds ONCE, configure the trusted publisher (see the
# instructions it prints) and never run it again -- every later release
# publishes through .github/workflows/publish.yaml.
#
# WHAT VERSION TO PASS
#
# The wrapper downloads `v<version>` release assets at install time, so the
# version published here must already have a GitHub release with its assets
# uploaded. Publishing ahead of the release would put a permanently broken
# version on npm, and npm's unpublish window is 72 hours. The script refuses
# to publish a version whose assets are not all present.
#
# Bootstrapping with the previous release (whose assets exist today) is the
# lower-risk path: it creates the package name, lets you configure trusted
# publishing, and then the next tag publishes through CI like every release
# after it.
#
# Usage:
#   scripts/publish-npm-bootstrap.sh <version>          # e.g. 0.12.0
#   scripts/publish-npm-bootstrap.sh <version> --dry-run

set -euo pipefail

REPO="Goldziher/scythe"
PACKAGE="scythe-cli"

# Must match TRIPLES in release/npm/lib/platform.js. A missing asset here means
# every install on that platform 404s, so all five are required, not just the
# publishing machine's own.
REQUIRED_ASSETS=(
	"scythe-x86_64-unknown-linux-gnu.tar.gz"
	"scythe-aarch64-unknown-linux-gnu.tar.gz"
	"scythe-x86_64-apple-darwin.tar.gz"
	"scythe-aarch64-apple-darwin.tar.gz"
	"scythe-x86_64-pc-windows-gnu.zip"
)

die() {
	printf '\nerror: %s\n' "$1" >&2
	exit 1
}

step() { printf '\n==> %s\n' "$1"; }

VERSION="${1:-}"
DRY_RUN=false
[ "${2:-}" = "--dry-run" ] && DRY_RUN=true

[ -n "$VERSION" ] || die "usage: $0 <version> [--dry-run]   (e.g. $0 0.12.0)"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be bare X.Y.Z, without a leading 'v': got '$VERSION'"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NPM_DIR="$REPO_ROOT/release/npm"
[ -f "$NPM_DIR/package.json" ] || die "$NPM_DIR/package.json not found -- run this from a scythe checkout"

step "Checking prerequisites"

command -v npm >/dev/null || die "npm not found"
command -v gh >/dev/null || die "gh not found -- needed to verify the release assets exist"

NPM_USER="$(npm whoami 2>/dev/null)" || die "not logged in to npm. Run 'npm login' first."
echo "npm user:    $NPM_USER"
echo "npm version: $(npm --version)"

# A package that already exists does not need this script, and re-running it
# would bypass the trusted publisher that should now be doing the work.
if npm view "$PACKAGE" version >/dev/null 2>&1; then
	die "$PACKAGE already exists on npm (latest: $(npm view "$PACKAGE" version)).
       This bootstrap script is for the FIRST publish only. Later releases must go
       through .github/workflows/publish.yaml via trusted publishing. If that
       workflow is failing, fix the trusted publisher configuration on npmjs.com
       rather than publishing by hand."
fi

step "Verifying the v$VERSION release assets exist"

gh release view "v$VERSION" --repo "$REPO" >/dev/null 2>&1 ||
	die "release v$VERSION does not exist in $REPO.
       The wrapper downloads its binary from that release at install time, so
       publishing now would put a permanently broken version on npm.
       Cut the release first, or bootstrap with an earlier version whose assets
       are already published."

published_assets="$(gh release view "v$VERSION" --repo "$REPO" --json assets --jq '.assets[].name')"

missing=()
for asset in "${REQUIRED_ASSETS[@]}" "scythe_${VERSION}_checksums.txt"; do
	grep -qxF "$asset" <<<"$published_assets" || missing+=("$asset")
done

if [ ${#missing[@]} -gt 0 ]; then
	printf 'missing from the v%s release:\n' "$VERSION" >&2
	printf '  %s\n' "${missing[@]}" >&2
	die "refusing to publish -- installs would 404 on the affected platforms"
fi
echo "all $((${#REQUIRED_ASSETS[@]} + 1)) required assets present"

cd "$NPM_DIR"

# package.json carries a 0.0.0 placeholder in git; the real version is injected
# at publish time. Always put it back, including on failure, so a botched run
# cannot leave a real version staged by accident.
#
# Restored as raw bytes rather than by running `npm version` back down: npm
# rewrites package.json with its own JSON formatting, expanding the single-line
# arrays this repo's formatter keeps inline. That round-trips the version
# correctly but still leaves a diff, and fails `poly fmt --check`.
ORIGINAL_VERSION="$(node -p "require('./package.json').version")"
MANIFEST_BACKUP="$(mktemp)"
LOCKFILE_BACKUP="$(mktemp)"
cp "$NPM_DIR/package.json" "$MANIFEST_BACKUP"
cp "$NPM_DIR/package-lock.json" "$LOCKFILE_BACKUP" 2>/dev/null || true

restore_manifest() {
	cp "$MANIFEST_BACKUP" "$NPM_DIR/package.json"
	[ -s "$LOCKFILE_BACKUP" ] && cp "$LOCKFILE_BACKUP" "$NPM_DIR/package-lock.json"
	rm -f "$MANIFEST_BACKUP" "$LOCKFILE_BACKUP"
	rm -f "$NPM_DIR"/*.tgz
}
trap restore_manifest EXIT

# --ignore-scripts: the package's own postinstall downloads the binary, and it
# deliberately refuses to run while package.json still carries the 0.0.0
# placeholder. That guard is correct -- it just means a plain `npm install` here
# exits non-zero. The postinstall does get exercised, by the smoke install below,
# once the real version is set.
step "Installing dependencies and running the test suite"
npm install --no-audit --no-fund --silent --ignore-scripts
npm test

step "Setting the package version to $VERSION"
npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null
echo "package.json version: $(node -p "require('./package.json').version")"

step "Packing"
npm pack --silent >/dev/null
TARBALL="$(ls ./*.tgz)"
echo "built $TARBALL"

# Smoke-test the exact artifact that is about to be published. npm's unpublish
# window is 72 hours and republishing the same version is never allowed, so a
# smoke test after publishing would only tell you about a package you are stuck
# with.
#
# Installed into a throwaway prefix rather than the real global root: a global
# install would put a second `scythe` on PATH, shadowing whatever the developer
# already has installed. The install still runs the real postinstall, which is
# the part being tested.
step "Smoke-testing the packed tarball (isolated install, downloads the real binary)"
SMOKE_PREFIX="$(mktemp -d)"
SMOKE_CACHE="$(mktemp -d)"
trap 'restore_manifest; rm -rf "$SMOKE_PREFIX" "$SMOKE_CACHE"' EXIT

# An empty SCYTHE_CACHE_DIR forces a real download. Against the developer's own
# cache the shim would happily run a binary left over from earlier testing, and
# the smoke test would pass without the download path executing at all.
export SCYTHE_CACHE_DIR="$SMOKE_CACHE"

npm install -g --prefix "$SMOKE_PREFIX" "$TARBALL"

SHIM="$SMOKE_PREFIX/lib/node_modules/$PACKAGE/bin/scythe"
[ -f "$SHIM" ] || SHIM="$SMOKE_PREFIX/node_modules/$PACKAGE/bin/scythe"
[ -f "$SHIM" ] || die "expected the installed shim under $SMOKE_PREFIX"

# The shim only ever reads the cache -- it does not download. So a populated
# cache here is proof the postinstall ran and fetched the binary, which is the
# part of the install that can actually fail in the wild. npm 11 warns that
# install scripts are "not yet covered by allowScripts" but still runs them;
# should a future npm or a strict-allow-scripts org policy block them outright,
# this check is what will catch it.
find "$SMOKE_CACHE" -type f -name scythe\* | grep -q . ||
	die "the postinstall did not populate $SMOKE_CACHE -- the binary download did not run.
       If npm reported that install scripts were blocked, this package cannot work as
       published: bin/scythe reads the cache and never downloads."

reported="$(node "$SHIM" --version 2>&1)" || die "the installed wrapper failed to run: $reported"
echo "wrapper reports: $reported"
grep -qF "$VERSION" <<<"$reported" ||
	die "the wrapper reported '$reported', which does not contain $VERSION"

if node "$SHIM" this-is-not-a-real-subcommand >/dev/null 2>&1; then
	die "expected a non-zero exit code from an invalid subcommand -- exit codes are not being forwarded"
fi
echo "exit-code forwarding OK"

if [ "$DRY_RUN" = true ]; then
	step "Dry run -- not publishing"
	npm publish --access public --dry-run
	echo
	echo "Dry run complete. Nothing was published. Re-run without --dry-run to publish."
	exit 0
fi

step "Publishing $PACKAGE@$VERSION to npm"
echo "This cannot be undone after 72 hours, and this exact version can never be republished."
read -r -p "Type the version to confirm: " confirm
[ "$confirm" = "$VERSION" ] || die "confirmation did not match -- nothing was published"

# No --provenance: npm can only generate a provenance attestation from a
# supported CI environment (GitHub Actions, GitLab), and fails outright when
# run locally. CI passes --provenance for every subsequent release.
npm publish --access public

cat <<EOF

==> Published $PACKAGE@$VERSION

Do this now, so no further manual publish is ever needed:

  1. https://www.npmjs.com/package/$PACKAGE/access
  2. Under "Trusted Publisher", add a GitHub Actions publisher:
       Organization or user:  Goldziher
       Repository:            scythe
       Workflow filename:     publish.yaml
       Environment:           (leave empty)
  3. Confirm the next release publishes npm from CI rather than skipping it.

The working tree is unchanged -- package.json is back to its $ORIGINAL_VERSION placeholder.
EOF
