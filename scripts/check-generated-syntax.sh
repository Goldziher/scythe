#!/usr/bin/env bash
# Syntax-check the harnesses rendered by tools/integration-test-generator.
#
# These files are generated from templates/*.jinja. A formatter once ran over
# those templates and stripped their newlines, re-wrapping them as prose at 120
# columns. That silently produced harnesses with string literals split across
# lines and `//` comments swallowing the code behind them, and it reached CI as
# a pile of confusing compiler errors rather than as a clear signal.
#
# The languages checked here are the ones whose syntax checkers are cheap and
# need no project build. Java, Kotlin and C# are covered by the compile steps in
# the integration workflow instead.
#
# In CI a missing checker must not silently no-op the guard it exists to
# provide, so strict mode turns a missing checker into a failure. Strict mode
# is on automatically when the CI env var is set (as GitHub Actions does), or
# explicitly via --strict. Local runs stay tolerant by default so a
# contributor without, say, PHP installed still gets useful output for the
# languages they do have.
#
# Usage: scripts/check-generated-syntax.sh [--strict]
set -uo pipefail

strict=0
if [ "${CI:-}" = "true" ] || [ "${1:-}" = "--strict" ]; then
	strict=1
fi

cd "$(dirname "$0")/.." || exit 1

failures=0

report() {
	printf '  FAIL %s\n' "$1"
	printf '%s\n' "$2" | sed 's/^/       /'
	failures=$((failures + 1))
}

skipped=0

check() {
	local label=$1 pattern=$2
	shift 2
	if ! command -v "$1" >/dev/null 2>&1; then
		if [ "$strict" -eq 1 ]; then
			printf '== %s -- FAIL, %s not installed (strict mode)\n' "$label" "$1"
			failures=$((failures + 1))
			return
		fi
		printf '== %s -- SKIPPED, %s not installed\n' "$label" "$1"
		skipped=$((skipped + 1))
		return
	fi
	printf '%s\n' "== $label"
	local file output
	while IFS= read -r file; do
		[ -n "$file" ] || continue
		if ! output=$("$@" "$file" 2>&1); then
			report "$file" "$output"
		fi
	done < <(git ls-files "$pattern")
}

check "PHP" 'integration_tests/*/test_integration.php' php -l
check "Ruby" 'integration_tests/*/test_integration.rb' ruby -c
check "Python" 'integration_tests/*/test_integration.py' python3 -m py_compile
check "Go" 'integration_tests/*/main.go' gofmt -e -l

if [ "$failures" -ne 0 ]; then
	printf '\n%s generated harness file(s) failed to parse.\n' "$failures"
	printf 'Fix the template under tools/integration-test-generator/templates/, not the generated file.\n'
	exit 1
fi

if [ "$skipped" -ne 0 ]; then
	printf '\nAll checked harnesses parse (%s language(s) skipped for missing tooling).\n' "$skipped"
else
	printf '\nAll generated harnesses parse.\n'
fi
