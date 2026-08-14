#!/usr/bin/env python3
"""Generate fresh code against sql/torture/schema.sql for every committed
postgresql-engine integration project, and real-build-check the result with
each project's own tooling.

Why this exists (issue #146, and the review that found #126): the committed
integration_tests/**/generated/ output is checked against sql/pg/schema.sql,
a 29-line schema with one enum and a handful of scalar columns. It cannot
reach the type/identifier shapes several known codegen defects live in. This
script generates fresh output -- into a scratch copy, never into
integration_tests/ -- against sql/torture/schema.sql instead, which is
deliberately hostile: an array column, an enum inside an array, a composite
type, a quoted mixed-case primary key, columns named for words reserved in
one or more target languages, and a column name that collides with a name
scythe itself might synthesize.

It also closes a narrower gap than check-generated-syntax.sh: that script
only ever looks at what is already committed, so a backend with no
integration project (e.g. typescript-duckdb, #126) commits no output and is
invisible to it. Regenerating fresh here, from the manifest that actually
ships, does not have that blind spot for any backend that has a committed
project to regenerate into -- which is every postgresql-engine backend today.

Real build tooling, not bare compilers: a javac/elixirc run with no project
context reports failures a real build does not have (JSR-305 not on a bare
classpath, Postgrex/MyXQL/Tds not on a bare compile path) -- see
check-generated-syntax.sh's javac_check/elixirc_check for that check, which
this script's real build commands are strictly more accurate than for the
subset of languages where a real build tool is cheap to run.

Exit status is driven by scripts/torture-expected-failures.txt, not by
whether anything failed: a project that fails but is listed there is a known,
already-reported defect and does not fail the build; a project that fails
and is NOT listed is a regression and does; a project that is listed but
PASSES means the list is stale (the defect was fixed) and also fails the
build, so the list is forced to shrink instead of silently rotting. This is
deliberately not a tolerance or a percentage -- every entry is a specific
project with a specific reason, and the check is binary per project.

Usage: scripts/check-generated-backends.py [--keep]
  --keep  Print the scratch workspace path instead of deleting it on exit.
"""

from __future__ import annotations

import glob
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INTEGRATION = os.path.join(ROOT, "integration_tests")
SCYTHE_BIN = os.environ.get("SCYTHE_BIN", os.path.join(ROOT, "target", "release", "scythe"))
TORTURE_SCHEMA = os.path.join(INTEGRATION, "sql", "torture", "schema.sql")
TORTURE_QUERIES = os.path.join(INTEGRATION, "sql", "torture", "queries", "*.sql")
EXPECTED_FAILURES_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "torture-expected-failures.txt")


def load_expected_failures() -> dict[str, str]:
    """Parse "<project> : <reason>" lines into {project: reason}. Blank lines
    and lines starting with # are ignored (see the file's own header)."""
    expected: dict[str, str] = {}
    with open(EXPECTED_FAILURES_FILE, encoding="utf-8") as fh:
        for raw_line in fh:
            line = raw_line.strip()
            if not line or line.startswith("#"):
                continue
            project, _, reason = line.partition(":")
            expected[project.strip()] = reason.strip()
    return expected


# Per-backend real build command, run with the project directory as cwd.
# `None` means no real build tool applies to this language (PHP and Ruby
# have no compile step at all; a syntax check *is* their real build).
#
# go-pgx builds only the `generated` package, not `./...`: the committed
# go.mod under integration_tests/go-pgx lists no `// indirect` requirements
# (a pre-existing gap in the go.mod.jinja template, not something this
# script introduces -- `go build ./...` fails identically on the untouched
# committed project with "updates to go.mod needed"), so a whole-module
# build needs `-mod=mod` to patch that in memory. Scoping to `./generated`
# also means main.go's own torture-schema mismatch (see HARNESS_STUBS) never
# has to be dealt with for Go specifically.
BUILD_COMMANDS: dict[str, list[str] | None] = {
    "rust-sqlx": ["cargo", "check", "--locked", "--quiet"],
    "rust-tokio-postgres": ["cargo", "check", "--locked", "--quiet"],
    "csharp-npgsql": ["dotnet", "build", "--nologo", "-v", "quiet"],
    "go-pgx": ["go", "build", "-mod=mod", "./generated"],
    "java-jdbc": ["mvn", "-q", "compile"],
    "java-r2dbc": ["mvn", "-q", "compile"],
    # kotlin-jdbc used to be None here, checked by bare kotlinc, on the grounds
    # that its generated code "only ever references java.sql/java.math/
    # java.time (JDK-only) ... there is no third-party classpath entry a real
    # Gradle build would add that bare kotlinc would miss".
    #
    # kotlin-r2dbc falsified that premise for the kotlin family: it emits
    # reactor.core.publisher.Mono, io.r2dbc.spi.Row and kotlinx.coroutines.
    # reactive. Worse, bare kotlinc does not merely miss those -- with no
    # io.r2dbc.spi.Row on the classpath, `row.get("id", Int::class.
    # javaObjectType)` resolves against Kotlin stdlib's MatchGroupCollection.get
    # and reports "actual type is 'MatchGroup?'" on code a real Gradle build
    # compiles cleanly. That is a failure no change to the generated code can
    # clear, the same vacuous shape as `ruby -c` over queries.rbs.
    #
    # Both kotlin backends therefore run a real Gradle build, matching
    # check-generated-syntax.sh, which made the identical change. Keeping the
    # two in step is deliberate: the SYNTAX_BY_EXTENSION comment below already
    # records why this fact must not have two derivations that can disagree.
    "kotlin-jdbc": ["gradle", "compileKotlin", "--quiet"],
    "kotlin-r2dbc": ["gradle", "compileKotlin", "--quiet"],
    "kotlin-exposed": ["gradle", "compileKotlin", "--quiet"],
    "elixir-ecto": ["sh", "-c", "mix deps.get --force && mix compile --force"],
    "elixir-postgrex": ["sh", "-c", "mix deps.get --force && mix compile --force"],
    "python-asyncpg": None,
    "python-psycopg3": None,
    "typescript-kysely": ["sh", "-c", "pnpm install --frozen-lockfile --silent && pnpm run --silent typecheck"],
    "typescript-pg": ["sh", "-c", "pnpm install --frozen-lockfile --silent && pnpm run --silent typecheck"],
    "typescript-postgres": ["sh", "-c", "pnpm install --frozen-lockfile --silent && pnpm run --silent typecheck"],
    "php-amphp": None,
    "php-pdo": None,
    "ruby-pg": None,
}

SYNTAX_ONLY: dict[str, list[str]] = {
    "python-asyncpg": ["python3", "-m", "py_compile"],
    "python-psycopg3": ["python3", "-m", "py_compile"],
    "php-amphp": ["php", "-l"],
    "php-pdo": ["php", "-l"],
    "ruby-pg": ["ruby", "-c"],
}

# The checker is decided by the file's language, not by the backend that wrote
# it: one output directory can hold more than one language. ruby-pg emits both
# `queries.rb` and `queries.rbs`, and RBS is a signature language, not Ruby --
# `ruby -c` cannot parse `ACTIVE: String`. Dispatching on the backend alone ran
# `ruby -c` over the type signatures and produced a failure that no change to
# the generated code could ever clear, which is precisely the class of vacuous
# result this script exists to eliminate. check-generated-syntax.sh:219-222
# already made this distinction; this map stops that fact having two
# derivations that can disagree.
SYNTAX_BY_EXTENSION: dict[str, list[str]] = {
    ".rbs": ["rbs", "parse"],
}

# The extension each SYNTAX_ONLY backend's own checker is able to read. A file
# whose extension is in neither this map nor SYNTAX_BY_EXTENSION is a hard
# error rather than a silent dispatch to the backend's checker.
#
# This is the ruby-pg defect generalised. `ruby -c` was run over `queries.rbs`
# for as long as that entry existed, because dispatch fell back to the backend
# whenever the extension was unrecognised -- and a checker pointed at a file it
# cannot parse produces a failure no change to the generated code can ever
# clear. SYNTAX_BY_EXTENSION fixed the one known instance; this closes the
# fallback that would let the next one happen silently. A backend that starts
# emitting a second file type now fails loudly here until someone decides
# which checker should read it.
PRIMARY_EXTENSION: dict[str, str] = {
    "python-asyncpg": ".py",
    "python-psycopg3": ".py",
    "php-amphp": ".php",
    "php-pdo": ".php",
    "ruby-pg": ".rb",
}

# The hand-written test harness in each project calls generated functions by
# name against sql/pg/schema.sql's shape (CreateOrder, GetUserById, ...).
# sql/torture/schema.sql defines none of those, so an unmodified harness
# fails a whole-project build on undefined-symbol errors that have nothing
# to do with whether the freshly generated code itself is correct -- see the
# kotlin-jdbc-ext dry run in the review this script responds to, where
# `Unresolved reference 'createUser'` in IntegrationTest.kt masked
# everything else. Overwriting the harness with a no-op keeps the build real
# (same compiler, same project, same dependencies) while making it a build
# of the generated module alone.
HARNESS_STUBS: dict[str, tuple[str, str]] = {
    "rust-sqlx": ("src/main.rs", "#[allow(dead_code, unused_imports)]\nmod queries;\n\nfn main() {}\n"),
    "rust-tokio-postgres": ("src/main.rs", "#[allow(dead_code, unused_imports)]\nmod queries;\n\nfn main() {}\n"),
    # ~keep The stub needs a real entry point, not just a comment: the project is
    # `<OutputType>Exe</OutputType>`, so a Program.cs with no `Main` makes
    # `dotnet build` fail with `CS5001: Program does not contain a static
    # 'Main' method suitable for an entry point` no matter what the generated
    # code says. That made csharp-npgsql's expected-failure entry permanent --
    # it could never go stale, which is the exact rot this file's two-way gate
    # exists to prevent. Mirrors rust-sqlx's `fn main() {}` and java-jdbc's
    # `public static void main`, neither of which had the problem.
    "csharp-npgsql": (
        "Program.cs",
        "// stubbed by check-generated-backends.py: see HARNESS_STUBS\n"
        "public static class Program {\n    public static void Main() { }\n}\n",
    ),
    "java-jdbc": (
        "src/main/java/IntegrationTest.java",
        "public class IntegrationTest {\n    public static void main(String[] args) {}\n}\n",
    ),
    "typescript-kysely": ("test.ts", "export {};\n"),
    "typescript-pg": ("test.ts", "export {};\n"),
    "typescript-postgres": ("test.ts", "export {};\n"),
    "java-r2dbc": (
        "src/main/java/IntegrationTest.java",
        "public class IntegrationTest {\n    public static void main(String[] args) {}\n}\n",
    ),
    # ~keep Both kotlin backends build for real now (see BUILD_COMMANDS), so both need their
    # harness stubbed -- kotlin-jdbc did not while bare kotlinc only ever saw queries.kt.
    "kotlin-jdbc": ("src/main/kotlin/IntegrationTest.kt", "fun main() {}\n"),
    "kotlin-r2dbc": ("src/main/kotlin/IntegrationTest.kt", "fun main() {}\n"),
    "kotlin-exposed": ("src/main/kotlin/IntegrationTest.kt", "fun main() {}\n"),
}


def find_pg_projects() -> list[dict]:
    projects = []
    for scythe_toml in sorted(glob.glob(os.path.join(INTEGRATION, "*", "scythe.toml"))):
        proj_dir = os.path.dirname(scythe_toml)
        with open(scythe_toml, "rb") as fh:
            data = tomllib.load(fh)
        for sql in data.get("sql", []):
            if sql.get("engine") != "postgresql":
                continue
            for gen in sql.get("gen", []):
                projects.append(
                    {
                        "dir": proj_dir,
                        "name": os.path.basename(proj_dir),
                        "backend": gen["backend"],
                        "output": gen["output"],
                    }
                )
    return projects


def patch_scythe_toml(path: str) -> None:
    """Point schema/queries at the torture fixture, in place, on the copy.

    Both substitutions are asserted, because a silent no-op here is the worst
    outcome this script can produce: the project would keep pointing at the
    29-line sql/pg/schema.sql, generate trivially correct code from it, and
    be recorded PASS while appearing to have been torture-tested. The match
    is deliberately narrow -- a stripped line starting `schema = [` -- so a
    multi-line array or a `schema=[` spelling would slip through unnoticed.
    Every pg manifest matches the expected single-line form today; nothing
    but this assertion keeps that true.
    """
    with open(path, encoding="utf-8") as fh:
        text = fh.read()
    lines = text.splitlines()
    out = []
    patched_schema = False
    patched_queries = False
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("schema = ["):
            out.append(f'schema = ["{TORTURE_SCHEMA}"]')
            patched_schema = True
        elif stripped.startswith("queries = ["):
            out.append(f'queries = ["{TORTURE_QUERIES}"]')
            patched_queries = True
        else:
            out.append(line)
    if not patched_schema or not patched_queries:
        missing = []
        if not patched_schema:
            missing.append("schema = [")
        if not patched_queries:
            missing.append("queries = [")
        raise RuntimeError(
            f"{path}: could not repoint {' and '.join(missing)} at the torture fixture. "
            "The project would have been checked against its own schema and recorded PASS "
            "without ever seeing the torture schema."
        )
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("\n".join(out) + "\n")


def run(cmd: list[str], cwd: str, timeout: int = 300) -> tuple[bool, str]:
    try:
        proc = subprocess.run(
            cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, text=True
        )
        return proc.returncode == 0, proc.stdout
    except subprocess.TimeoutExpired as e:
        return False, f"timed out after {timeout}s: {e.output or ''}"
    except FileNotFoundError as e:
        return False, f"tool not found: {e}"


def run_syntax_check(backend: str, file_path: str, cwd: str) -> tuple[bool, str]:
    """Dispatch SYNTAX_BY_EXTENSION first, then SYNTAX_ONLY[backend].

    The extension map wins because a file's language is a property of the file,
    not of the backend that emitted it."""
    ext = os.path.splitext(file_path)[1]
    ext_cmd = SYNTAX_BY_EXTENSION.get(ext)
    if ext_cmd is not None:
        return run([*ext_cmd, file_path], cwd=cwd, timeout=30)
    expected = PRIMARY_EXTENSION.get(backend)
    if expected is not None and ext != expected:
        return False, (
            f"{backend} emitted a {ext or '(no extension)'} file, but its checker "
            f"{SYNTAX_ONLY.get(backend)} reads {expected}. Dispatching it anyway is how "
            "`ruby -c` came to be run over queries.rbs. Register the right checker in "
            "SYNTAX_BY_EXTENSION, or update PRIMARY_EXTENSION if the backend legitimately "
            "changed what it emits."
        )
    syntax_cmd = SYNTAX_ONLY.get(backend)
    if syntax_cmd is None:
        return False, "no syntax command registered"
    return run([*syntax_cmd, file_path], cwd=cwd, timeout=30)


def main() -> int:
    keep = "--keep" in sys.argv

    if not os.path.exists(SCYTHE_BIN):
        print(f"FAIL: {SCYTHE_BIN} not built. Run: cargo build --release -p scythe-cli", file=sys.stderr)
        return 1

    projects = find_pg_projects()
    scratch = tempfile.mkdtemp(prefix="scythe-torture-")
    print(f"scratch workspace: {scratch}")

    # sql/ is copied once and shared by every project below via the same
    # `../sql/...` relative paths their committed scythe.toml already uses.
    shutil.copytree(os.path.join(INTEGRATION, "sql"), os.path.join(scratch, "sql"))

    results = []
    for proj in projects:
        dst = os.path.join(scratch, proj["name"])
        shutil.copytree(
            proj["dir"],
            dst,
            ignore=shutil.ignore_patterns(
                "node_modules", "bin", "obj", "target", "__pycache__", "*.egg-info", "_build", "deps"
            ),
        )
        # No rmtree of the output dir first: `scythe generate` overwrites the
        # files it owns in place, and for rust-sqlx-nested-json `output` is
        # "src" -- the same directory main.rs (a HARNESS_STUBS target) lives
        # in, so removing the directory would also remove the just-written
        # stub (or race with writing it, depending on order).
        patch_scythe_toml(os.path.join(dst, "scythe.toml"))

        stub = HARNESS_STUBS.get(proj["backend"])
        if stub is not None:
            rel_path, content = stub
            stub_path = os.path.join(dst, rel_path)
            # `open(..., "w")` creates a missing file rather than failing, so a
            # stub aimed at a path the project no longer has would write a stray
            # file AND leave the real harness in place. In a compiled language
            # that is a permanent undefined-symbol failure, which then gets
            # allowlisted with a reason describing the generated code -- the
            # exact rot that made csharp-npgsql's entry unfalsifiable for its
            # whole life. In TypeScript it is worse: a stray `test.ts` would
            # itself typecheck clean, so the project fails silently in the PASS
            # direction. One directory rename is all it takes either way.
            if not os.path.exists(stub_path):
                raise RuntimeError(
                    f"{proj['name']}: HARNESS_STUBS targets {rel_path}, which does not exist in "
                    "this project. Writing it would leave the real harness in place and make this "
                    "project's result meaningless. Fix the stub path in HARNESS_STUBS."
                )
            with open(stub_path, "w", encoding="utf-8") as fh:
                fh.write(content)

        ok, out = run([SCYTHE_BIN, "generate"], cwd=dst, timeout=60)
        if not ok:
            results.append((proj["name"], proj["backend"], "generate", out.strip()))
            continue

        cmd = BUILD_COMMANDS.get(proj["backend"])
        if cmd is None:
            out_dir = os.path.join(dst, proj["output"])
            files = [f for f in glob.glob(os.path.join(out_dir, "*")) if os.path.isfile(f)]
            # An empty output directory used to fall straight through to PASS:
            # `failed` stays empty when there is nothing to iterate, so a project
            # whose generator wrote no file at all reported exactly the same
            # result as one whose every file checked out. A gate that passes when
            # it inspected nothing is indistinguishable from one that cannot fail.
            if not files:
                results.append(
                    (
                        proj["name"],
                        proj["backend"],
                        "syntax",
                        f"no generated files found in {proj['output']}/ -- nothing was checked, "
                        "so this project's PASS would have been vacuous",
                    )
                )
                continue
            failed = []
            for f in files:
                ok2, out2 = run_syntax_check(proj["backend"], f, cwd=dst)
                if not ok2:
                    failed.append(f"{f}:\n{out2}")
            if failed:
                results.append((proj["name"], proj["backend"], "syntax", "\n".join(failed)))
            else:
                results.append((proj["name"], proj["backend"], "PASS", ""))
            continue

        ok3, out3 = run(cmd, cwd=dst, timeout=300)
        if ok3:
            results.append((proj["name"], proj["backend"], "PASS", ""))
        else:
            results.append((proj["name"], proj["backend"], "build", out3.strip()))

    expected = load_expected_failures()
    project_names = {p["name"] for p in projects}
    unknown_entries = sorted(set(expected) - project_names)

    print()
    print(f"{len(projects)} postgresql-engine project(s) regenerated against sql/torture/schema.sql:")
    fail_count = 0
    regressions: list[str] = []
    stale_entries: list[str] = []
    for name, backend, stage, detail in results:
        actually_failed = stage != "PASS" and stage != "SKIP"
        is_expected = name in expected
        if actually_failed:
            fail_count += 1
            status = "FAIL(expected)" if is_expected else "FAIL(NEW)"
            if not is_expected:
                regressions.append(name)
        else:
            status = "PASS(stale-allowlist)" if is_expected else "PASS"
            if is_expected:
                stale_entries.append(name)
        print(f"  {status:22s} {name:38s} backend={backend:20s} stage={stage}")
        if actually_failed:
            # Tail, not head: dependency-fetch/compile noise (elixir's Hex
            # resolution, maven's plugin banners) comes first and the actual
            # error is what the tool reported last.
            lines = detail.splitlines()
            for ln in lines[-40:]:
                print(f"        {ln}")
    print()
    print(f"{fail_count}/{len(projects)} failed ({len(expected)} expected by {EXPECTED_FAILURES_FILE}).")

    ok = True
    if regressions:
        ok = False
        print()
        print(f"REGRESSION: {len(regressions)} project(s) failed but are not in {EXPECTED_FAILURES_FILE}:")
        for name in regressions:
            print(f"  - {name}")
        print("If this is a new, real defect: add a line for it to the allowlist file (with a")
        print("reason and issue number) so this is a known baseline, not a silent regression.")
        print("If this is a driver bug in check-generated-backends.py itself: fix the driver.")

    if stale_entries:
        ok = False
        print()
        print(f"STALE ALLOWLIST: {len(stale_entries)} project(s) are listed in {EXPECTED_FAILURES_FILE} but now PASS:")
        for name in stale_entries:
            print(f"  - {name}")
        print("The underlying defect appears fixed. Delete these lines from the allowlist file.")

    if unknown_entries:
        ok = False
        print()
        print(f"STALE ALLOWLIST: {len(unknown_entries)} entry/entries name a project that no longer exists:")
        for name in unknown_entries:
            print(f"  - {name}")
        print("Delete these lines, or the project was renamed and the entry needs to follow it.")

    if keep:
        print(f"scratch workspace kept at: {scratch}")
    else:
        shutil.rmtree(scratch, ignore_errors=True)

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
