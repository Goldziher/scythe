#!/usr/bin/env bash
#MISE description="Fail if any published crate's version or inter-crate pin has drifted"
#MISE quiet=true
set -euo pipefail

# Portable (no BSD sed) so this can gate CI on Linux, unlike version:sync.
# Violations are accumulated in a variable rather than setting a flag
# inside a `... | while read` -- a pipeline runs its loop in a subshell,
# so the flag is lost and the check reports drift while still exiting 0.
# Two vacuous-pass guards below (`expected` non-empty, `revs_checked`
# non-zero). Both loops here accumulate into `violations` and report
# success when it stays empty -- which is also what happens when a loop
# runs zero times, or when every comparison is against the empty string.
# So a file that moves or a pattern that stops recognising its own input
# would turn this task into an unconditional "All crate versions, pins
# and documented rev pins agree" with nothing behind it, while ci.yml's
# Lint job goes on treating it as the release gate. Exactly the failure
# ci.yml's provenance check documents at its own `count -eq 0` guard.
#
# Verified by mutation, both guards: renaming the `version =` key in
# crates/scythe-cli/Cargo.toml, and renaming every `rev:` key under
# README.md/website/src/skills. Each exits 1 here; against the same
# inputs the pre-guard task printed "All crate versions, pins and
# documented rev pins agree on 0.14.0" and exited 0.
#
# `crates_checked` deliberately has no zero-guard: `expected` is read
# from crates/scythe-cli/Cargo.toml, so the glob below cannot match zero
# files without the guard above having already fired. A branch that
# cannot be reached is not a check. It is counted only for the summary
# line, so the output states how much was actually inspected.
expected=$(awk -F'"' '/^version = /{print $2; exit}' crates/scythe-cli/Cargo.toml)
if [ -z "$expected" ]; then
  echo "version:check: could not read a version from crates/scythe-cli/Cargo.toml -- that file is this task's only source of truth, so every comparison below would be against the empty string" >&2
  exit 1
fi
violations=""
crates_checked=0
revs_checked=0
hook_pins_checked=0
hook_probes_checked=0
for f in crates/*/Cargo.toml; do
  crates_checked=$((crates_checked + 1))
  # An unpublished crate never reaches crates.io, so its own version is
  # meaningless and version:sync deliberately leaves it alone --
  # scythe-conformance sits at 0.1.0 forever. Asserting lockstep on it
  # would make `mise run version:sync` abort on its own version:check step,
  # with no spelling of the version that could satisfy both.
  # Its *pins* are checked below regardless: cargo rejects a path
  # dependency whose `version` requirement does not match the path
  # crate's actual version, published or not.
  if ! grep -qE '^publish[[:space:]]*=[[:space:]]*false' "$f"; then
    own=$(awk -F'"' '/^version = /{print $2; exit}' "$f")
    if [ "$own" != "$expected" ]; then
      violations="${violations}version drift: $f declares $own, expected $expected\n"
    fi
  fi
  while IFS= read -r v; do
    [ -z "$v" ] && continue
    if [ "$v" != "$expected" ]; then
      violations="${violations}pin drift: $f pins a scythe crate at $v, expected $expected\n"
    fi
  done <<PINS
$(grep -oE 'scythe-[a-z]+ = \{[^}]*version = "[^"]*"' "$f" | sed -E 's/.*version = "([^"]*)"/\1/')
PINS
done
# The documented pre-commit `rev:` pins are rewritten by version:sync but
# validated by no other job, so without this they drift silently -- and a
# stale one pins every user who copies the snippet to the old release.
for f in $(grep -rlE '^[[:space:]]*rev: v[0-9]+\.[0-9]+\.[0-9]+' README.md website/src plugin/.ai-rulez/skills 2>/dev/null || true); do
  while IFS= read -r r; do
    [ -z "$r" ] && continue
    revs_checked=$((revs_checked + 1))
    if [ "$r" != "v$expected" ]; then
      violations="${violations}rev drift: $f documents $r, expected v$expected\n"
    fi
  done <<REVS
$(grep -oE '^[[:space:]]*rev: v[0-9]+\.[0-9]+\.[0-9]+' "$f" | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+')
REVS
done
if [ "$revs_checked" -eq 0 ]; then
  echo "version:check: found no documented 'rev: vX.Y.Z' pin under README.md, website/src or skills. version:sync rewrites those pins and nothing else validates them, so zero matches means this check silently stopped covering them -- either the pins moved (add their location to both this task and version:sync) or the grep pattern no longer recognises them." >&2
  exit 1
fi
while IFS= read -r v; do
  [ -z "$v" ] && continue
  hook_pins_checked=$((hook_pins_checked + 1))
  if [ "$v" != "$expected" ]; then
    violations="${violations}Poly hook pin drift: poly-hooks.toml pins scythe-cli at $v, expected $expected\n"
  fi
done <<HOOK_PINS
$(grep -oE 'scythe-cli --version [0-9]+\.[0-9]+\.[0-9]+' poly-hooks.toml | awk '{print $NF}')
HOOK_PINS
cargo_channels=$(grep -c '^channel = "cargo"$' poly-hooks.toml || true)
if [ "$hook_pins_checked" -eq 0 ] || [ "$hook_pins_checked" -ne "$cargo_channels" ]; then
  violations="${violations}Poly hook pin coverage: found $hook_pins_checked pinned install command(s) for $cargo_channels cargo channel(s)\n"
fi
while IFS= read -r v; do
  [ -z "$v" ] && continue
  hook_probes_checked=$((hook_probes_checked + 1))
  if [ "$v" != "$expected" ]; then
    violations="${violations}Poly hook probe drift: poly-hooks.toml checks for scythe $v, expected $expected\n"
  fi
done <<HOOK_PROBES
$(grep -oE 'scythe [0-9]+\.[0-9]+\.[0-9]+' poly-hooks.toml | awk '{print $NF}')
HOOK_PROBES
if [ "$hook_probes_checked" -eq 0 ] || [ "$hook_probes_checked" -ne "$cargo_channels" ]; then
  violations="${violations}Poly hook probe coverage: found $hook_probes_checked exact-version probe(s) for $cargo_channels cargo channel(s)\n"
fi
if [ -n "$violations" ]; then
  printf "%b" "$violations" >&2
  exit 1
fi
echo "All $crates_checked crate version(s)/pin(s), $revs_checked documented rev pin(s), and $hook_pins_checked Poly hook pin(s) agree on $expected"
