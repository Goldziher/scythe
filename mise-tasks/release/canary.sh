#!/usr/bin/env bash
#MISE description="Build and size-check every release target this host can build natively"
#MISE depends=["release:canary:config"]
set -euo pipefail

# ~keep Each archive is built natively on its own platform by the publish
# ~keep workflow; this aggregate task only has the host's toolchain, so it
# ~keep skips the targets belonging to another operating system rather than
# ~keep failing on a missing cross-linker.
case "$(uname -s)" in
Darwin) host_pattern='*-apple-darwin' ;;
Linux) host_pattern='*-linux-gnu' ;;
*) host_pattern='*' ;;
esac
python3 scripts/release-canary.py targets | while IFS= read -r target; do
  # shellcheck disable=SC2254
  case "$target" in
  $host_pattern) mise run release:canary:target "$target" </dev/null ;;
  *) echo "release-canary: skipping $target (not buildable on $(uname -s))" >&2 ;;
  esac
done
