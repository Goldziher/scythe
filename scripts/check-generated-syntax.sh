#!/usr/bin/env bash
# Syntax/type-check both the hand-written harnesses and the generated query
# code under integration_tests/.
#
# Two different generators feed integration_tests/, and this script now
# checks the output of both:
#
#   - tools/integration-test-generator renders the per-project scaffolding
#     (test_integration.{php,rb,py}, main.go, *.csproj, Cargo.toml, ...) from
#     templates/*.jinja. A formatter once ran over those templates and
#     stripped their newlines, re-wrapping them as prose at 120 columns. That
#     silently produced harnesses with string literals split across lines and
#     `//` comments swallowing the code behind them, and it reached CI as a
#     pile of confusing compiler errors rather than as a clear signal.
#   - scythe itself renders the actual generated/ query code (queries.{ts,py,
#     rb,rbs,php,go,ex,cs}, src/queries.rs, src/main/{java,kotlin}/generated/
#     Queries.{java,kt}) from the SQL fixtures under integration_tests/. That
#     output was committed but checked by nothing: nothing in CI ever asked
#     whether 115 committed query files actually compile.
#
# Two shapes of check follow from that:
#
#   - Most languages check one file at a time, with no project context, using
#     whatever checker verifies syntax (and, where the checker is generous
#     enough to offer it for free, real symbol resolution against the JDK/
#     stdlib) without resolving third-party driver types. That is what `check`
#     below does.
#   - TypeScript, C# and Rust generated code references driver-specific types
#     (`postgres`, `Npgsql`, `sqlx::FromRow`, ...) that only exist once the
#     project's own dependencies are present, so a single-file check would
#     fail 100% of files on "cannot find module/type" -- a signal that is
#     uniform across every file and therefore says nothing about the
#     generator. `check_project` below runs one real build per project
#     directory instead: `pnpm install && tsc --noEmit`, `dotnet build`, or
#     `cargo check`, using the project's own committed lockfile. None of the
#     three needs a live database -- they are pure static checks -- but they
#     are real project builds, so they are slower and need network access to
#     restore packages the first time.
#
# In CI a missing checker must not silently no-op the guard it exists to
# provide, so strict mode turns a missing checker into a failure. Strict mode
# is on automatically when the CI env var is set (as GitHub Actions does), or
# explicitly via --strict. Local runs stay tolerant by default so a
# contributor without, say, PHP installed still gets useful output for the
# languages they do have.
#
# Strict mode also turns a pattern that matches zero files into a failure. A
# `check` with a zero-match glob would otherwise run its loop zero times,
# leave `failures` untouched, and report success identically to "every file
# parsed" -- so a harness or generator-output rename could silently disarm an
# entire language's coverage. Mirrors the `count -eq 0` guard in
# .github/workflows/ci.yml's provenance-header check.
#
# --harness-only skips every "(generated)" query-code check below and runs
# only the four scaffolding checks that predate this script's extension to
# cover generated/ query code. It exists for the `generated-freshness` CI job
# (.github/workflows/ci.yml), which installs only the toolchains those four
# checks need; the query-code checks need javac/kotlinc/dotnet/rbs/pnpm/
# elixirc too, which is what the "Validate: Generated Query Code" job
# installs before running this script without the flag.
#
# Usage: scripts/check-generated-syntax.sh [--strict] [--harness-only]
set -uo pipefail

strict=0
harness_only=0
for arg in "$@"; do
	case "$arg" in
	--strict) strict=1 ;;
	--harness-only) harness_only=1 ;;
	esac
done
if [ "${CI:-}" = "true" ]; then
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

missing_tool() {
	local label=$1 toolname=$2
	if [ "$strict" -eq 1 ]; then
		printf '== %s -- FAIL, %s not installed (strict mode)\n' "$label" "$toolname"
		failures=$((failures + 1))
		return 0
	fi
	printf '== %s -- SKIPPED, %s not installed\n' "$label" "$toolname"
	skipped=$((skipped + 1))
	return 0
}

# check LABEL PATTERN TOOLNAME CMD...
#
# Runs `CMD... FILE` once per file matched by the `git ls-files` PATTERN.
# TOOLNAME is checked on PATH separately from CMD so a wrapper function (see
# javac_check et al. below) still reports the real underlying tool as
# missing, not the always-present wrapper.
check() {
	local label=$1 pattern=$2 toolname=$3
	shift 3
	if ! command -v "$toolname" >/dev/null 2>&1; then
		missing_tool "$label" "$toolname"
		return
	fi
	printf '%s\n' "== $label"
	local file output matched=0
	while IFS= read -r file; do
		[ -n "$file" ] || continue
		matched=$((matched + 1))
		if ! output=$("$@" "$file" 2>&1); then
			report "$file" "$output"
		fi
	done < <(git ls-files "$pattern")
	if [ "$matched" -eq 0 ]; then
		printf '  FAIL pattern matched no files: %s\n' "$pattern"
		failures=$((failures + 1))
	fi
}

# check_project LABEL PATTERN TOOLNAME DEPTH CMD...
#
# Like `check`, but runs `CMD...` once per unique project *directory* instead
# of once per file, with that directory as cwd. DEPTH is how many path
# components to strip off each matched file to reach its project root (e.g.
# 2 for `project/generated/queries.ts` -> `project`).
check_project() {
	local label=$1 pattern=$2 toolname=$3 depth=$4
	shift 4
	if ! command -v "$toolname" >/dev/null 2>&1; then
		missing_tool "$label" "$toolname"
		return
	fi
	printf '%s\n' "== $label"
	local dirs dir output matched=0
	dirs=$(
		git ls-files "$pattern" | while IFS= read -r f; do
			d=$f
			i=0
			while [ "$i" -lt "$depth" ]; do
				d=$(dirname "$d")
				i=$((i + 1))
			done
			printf '%s\n' "$d"
		done | sort -u
	)
	while IFS= read -r dir; do
		[ -n "$dir" ] || continue
		matched=$((matched + 1))
		if ! output=$(cd "$dir" && "$@" 2>&1); then
			report "$dir" "$output"
		fi
	done <<<"$dirs"
	if [ "$matched" -eq 0 ]; then
		printf '  FAIL pattern matched no files: %s\n' "$pattern"
		failures=$((failures + 1))
	fi
}

# Compiles a single .kt file against the JDK only, discarding class output to
# a scratch directory. Generated Kotlin only ever references java.sql/
# java.math/java.time (JDK-only), so unlike Java's javax.annotation this is
# already a real compile, not just a parse -- there is no third-party
# classpath entry a real Gradle build would add that bare kotlinc would miss.
kotlinc_check() {
	local file=$1 tmp
	tmp=$(mktemp -d) || return 1
	kotlinc -d "$tmp/out.jar" "$file"
	local status=$?
	rm -rf "$tmp"
	return $status
}

# Lock files are committed in this repo by policy (uv.lock, pnpm-lock.yaml,
# go.sum, composer.lock, Gemfile.lock, Cargo.lock alike) -- but 5 of the 10
# rust-* integration projects had no committed Cargo.lock, undetected,
# because integration_tests/.gitignore used to list `**/Cargo.lock` as a
# build artifact: `cargo generate-lockfile` produced a file a plain `git add`
# would silently skip. `cargo check --locked` below already fails on a
# missing lockfile, but with cargo's own wording ("cannot create the lock
# file ... because --locked was passed"), which reads like a flag problem,
# not a policy violation -- so this checks it explicitly, unconditionally
# (not gated by --harness-only: this is a project-hygiene check, not a
# generated-code check), and says what it actually is.
check_lockfiles_committed() {
	printf '%s\n' "== Rust Cargo.lock committed"
	local dir file matched=0
	while IFS= read -r file; do
		[ -n "$file" ] || continue
		dir=$(dirname "$file")
		matched=$((matched + 1))
		if ! git ls-files --error-unmatch "$dir/Cargo.lock" >/dev/null 2>&1; then
			report "$dir" "no committed Cargo.lock (present on disk is not enough -- it must be tracked)"
		fi
	done < <(git ls-files 'integration_tests/rust-*/Cargo.toml')
	if [ "$matched" -eq 0 ]; then
		printf '  FAIL pattern matched no files: integration_tests/rust-*/Cargo.toml\n'
		failures=$((failures + 1))
	fi
}
check_lockfiles_committed

check "PHP" 'integration_tests/*/test_integration.php' php php -l
check "Ruby" 'integration_tests/*/test_integration.rb' ruby ruby -c
check "Python" 'integration_tests/*/test_integration.py' python3 python3 -m py_compile
check "Go" 'integration_tests/*/main.go' gofmt gofmt -e -l

if [ "$harness_only" -eq 0 ]; then
	check "PHP (generated)" 'integration_tests/*/generated/*.php' php php -l
	check "Ruby (generated)" 'integration_tests/*/generated/*.rb' ruby ruby -c
	# .rbs files are RBS type signatures, not Ruby source -- `ruby -c` cannot
	# parse them. `rbs parse` is the signature-language equivalent: a pure
	# syntax check that needs no `-r`/`-I` library environment.
	check "RBS (generated)" 'integration_tests/*/generated/*.rbs' rbs rbs parse
	check "Python (generated)" 'integration_tests/*/generated/*.py' python3 python3 -m py_compile
	check "Go (generated)" 'integration_tests/*/generated/*.go' gofmt gofmt -e -l
	check "Kotlin (generated)" 'integration_tests/*/src/main/kotlin/generated/queries.kt' kotlinc kotlinc_check

	# TypeScript, C#, Rust, Java and Elixir generated code needs its
	# project's own dependencies resolved to mean anything, so these run one
	# real project build each instead of a per-file check.
	#
	# Java and Elixir specifically used to run through a bare javac/elixirc
	# per-file check here instead (no project, no dependencies). That
	# reported 11 failures that were not defects: javac has no classpath
	# entry for JSR-305 (`javax.annotation.{Nonnull,Nullable}`), which
	# tools/integration-test-generator/templates/pom.xml.jinja adds to every
	# committed java-jdbc-* project's pom.xml, and elixirc has no load path
	# for MyXQL/Tds, which is exactly what every committed elixir-*
	# project's `mix deps.get` provides. A real build tool run against the
	# real project reports what CI's `task test:*` would actually hit.
	check_project "TypeScript (generated)" 'integration_tests/*/generated/*.ts' pnpm 2 \
		sh -c 'pnpm install --frozen-lockfile --silent && pnpm run --silent typecheck'
	check_project "C# (generated)" 'integration_tests/*/generated/*.cs' dotnet 2 \
		dotnet build --nologo -v quiet
	check_project "Rust (generated)" 'integration_tests/*/src/queries.rs' cargo 2 \
		cargo check --locked --quiet
	check_project "Java (generated)" 'integration_tests/*/src/main/java/generated/Queries.java' mvn 5 \
		mvn -q compile
	check_project "Elixir (generated)" 'integration_tests/*/generated/*.ex' mix 2 \
		sh -c 'mix deps.get --force && mix compile --force'
fi

if [ "$failures" -ne 0 ]; then
	printf '\n%s generated file(s) failed to compile/parse.\n' "$failures"
	printf 'Fix the template under tools/integration-test-generator/templates/ for harness\n'
	printf 'files, or the scythe backend under crates/scythe-codegen/src/backends/ for\n'
	printf 'query files -- never the generated file itself.\n'
	exit 1
fi

if [ "$skipped" -ne 0 ]; then
	printf '\nAll checked files compile/parse (%s language(s) skipped for missing tooling).\n' "$skipped"
else
	printf '\nAll generated files compile/parse.\n'
fi
