#!/usr/bin/env bash
#MISE description="Update all package versions to the given version"
#USAGE arg "<version>" help="Release version, X.Y.Z"
# shellcheck disable=SC2154 # usage_* are set by mise from the #USAGE lines
set -euo pipefail

version="$usage_version"

# Every published crate must appear in both substitutions and in the
# second one's alternation. scythe-inspect was missing from all three,
# so its own version and every pin on it silently kept the old value --
# and because `publish_crates` tolerates an "already exists" error, the
# release succeeded green while shipping five crates instead of six.
# `version:check` below is the guard that makes a recurrence loud.
sed -i '' 's/^version = ".*"/version = "'"$version"'"/' crates/scythe-cli/Cargo.toml crates/scythe-core/Cargo.toml crates/scythe-codegen/Cargo.toml crates/scythe-backend/Cargo.toml crates/scythe-lint/Cargo.toml crates/scythe-inspect/Cargo.toml
# Pins are rewritten across every crate, not an explicit list: cargo
# rejects a path dependency whose `version` requirement does not match
# the path crate's actual version, so a crate missing from the list
# breaks the build outright. That includes crates the first substitution
# deliberately skips -- an unpublished crate still pins published ones.
# An explicit list is exactly how scythe-inspect went unbumped for two
# releases; a glob cannot forget a crate that someone adds later.
sed -i '' -E 's/(scythe-(core|codegen|backend|lint|cli|inspect).*version = ")[^"]*"/\1'"$version"'"/' crates/*/Cargo.toml
# The documented pre-commit `rev:` pins are the one version string a user
# copies verbatim into their own repo, so a stale one silently pins them to
# the previous release. They are prose Markdown, not snippets, so no CI job
# validates them -- they drifted through 0.12.0 unnoticed.
# `[[:space:]]`, not `\s`: BSD sed -E is POSIX ERE and does not know `\s`,
# so that spelling matches nothing and reports success -- the same silent
# no-op that let scythe-inspect go unbumped for two releases.
# Driven off a grep, not a list, for the same reason as the pin
# substitution above: an explicit list silently omits whatever nobody
# remembered to add. It did. `skills/scythe/SKILL.md` sat at v0.6.0 and
# `skills/scythe/references/cli-reference.md` at v0.5.0 -- eight and nine
# releases stale -- purely because `skills/` was never in the list, and
# `version:check` could not catch it because it read the same list.
files=$(grep -rlE '^[[:space:]]*rev: v[0-9]+\.[0-9]+\.[0-9]+' \
  README.md website/src plugin/.ai-rulez/skills 2>/dev/null || true)
if [ -n "$files" ]; then
  echo "$files" | while IFS= read -r f; do
    [ -n "$f" ] || continue
    sed -i '' -E 's/^([[:space:]]*rev: )v[0-9]+\.[0-9]+\.[0-9]+/\1v'"$version"'/' "$f"
  done
fi
# ~keep Keep native Poly install commands and exact-version probes aligned with the release.
sed -i '' -E 's/(scythe )[0-9]+\.[0-9]+\.[0-9]+/\1'"$version"'/g; s/(scythe-cli --version )[0-9]+\.[0-9]+\.[0-9]+/\1'"$version"'/g' poly-hooks.toml
# Every generated-code example in the docs opens with a provenance header,
# and that header embeds the scythe version. Same blind spot as the `rev:`
# pins above: prose Markdown that no CI job validates, so a stale one just
# sits there claiming the release generated output labelled with the
# previous version. Driven off a grep rather than an explicit file list --
# examples get added over time, and a list cannot forget a file nobody has
# written yet. The read loop avoids `xargs -r`, which BSD xargs lacks: a
# plain `xargs sed` on empty input runs sed with no file arguments, and it
# then blocks reading stdin.
#
# The changelog is excluded, and must stay excluded. Its entries quote the
# header as it looked in the release being described, so rewriting them
# would retroactively relabel every historical entry with the version
# currently being cut -- turning a record of what shipped into a claim
# that every past release emitted today's version.
#
# `skills` is in the search path even though no provenance string lives
# there today. It is latent, not hypothetical: the `rev:` sync above
# records that `skills/` was left out of exactly this kind of list once
# before and went eight and nine releases stale as a result. A grep over a
# directory that currently matches nothing costs nothing; a directory
# nobody remembered to add costs a release.
files=$(grep -rlE 'scythe:provenance v=[0-9]+\.[0-9]+\.[0-9]+' \
  website/src README.md skills 2>/dev/null |
  grep -v 'changelog\.md$' || true)
if [ -n "$files" ]; then
  echo "$files" | while IFS= read -r f; do
    [ -n "$f" ] || continue
    sed -i '' -E 's/(scythe:provenance v=)[0-9]+\.[0-9]+\.[0-9]+/\1'"$version"'/g' "$f"
  done
fi
# ~keep The rev: pin rewrite above edits plugin/.ai-rulez/skills, which the plugin
# ~keep root's ai-rulez.lock pins; relock it before regenerating or validation
# ~keep fails with AR752/AR981.
npx -y ai-rulez@latest -C plugin lock
mise run plugin:generate
mise run version:check
# `cargo check --workspace` is not subsumed by the regeneration step
# below: that step only builds scythe-cli in release mode (via
# integration_tests/mise.toml's `build:scythe`), which does not
# type-check crates that aren't in scythe-cli's dependency graph (e.g.
# scythe-lint, tools/*). Keep both.
#
# No `2>/dev/null || true` here: that used to swallow both the output
# and the exit code, so a version bump that broke the build (a crate
# left off the substitution list above, say) would sail through this
# task, regenerate integration_tests/ from a workspace that does not
# compile, and report success. Let it fail loudly and stop the task.
cargo check --workspace
# Every generated artifact's first line embeds CARGO_PKG_VERSION (the
# `v=` field of its `scythe:provenance` header), so the moment the
# substitutions above land, every committed
# integration_tests/**/generated/ file is stale: its header still names
# the old version while a freshly built `scythe` would emit the new
# one. CI's `generated-freshness` job then fails on the release commit
# itself. `generate:all` in integration_tests/mise.toml already
# rebuilds `scythe` from source via its `build:scythe` dependency and
# invokes it by absolute path (see that task's comment, issue #85) so a
# stale `scythe` on $PATH cannot silently revert this step -- reuse it
# here instead of re-deriving that binary-resolution logic.
(cd integration_tests && mise run generate:all)
echo "All crate versions updated to $version"
