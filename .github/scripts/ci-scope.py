#!/usr/bin/env python3
"""Choose which ci.yml check jobs a pull request needs, from its changed files.

Reads the changed paths, one per line, on stdin. Prints one JSON object mapping
each job id in JOBS to true (run it) or false (skip it), and writes a Markdown
table of the choices and their reasons to the file named by
$GITHUB_STEP_SUMMARY when that is set.

A path no rule below recognises selects every job: a file this script does not
know about is one it cannot prove harmless. `--all REASON` selects every job
without reading stdin (pushes, tags, scheduled and manual runs).
`--self-test` checks the rules against fixed path lists.
"""

import json
import os
import subprocess
import sys
import unittest
from fnmatch import fnmatch
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]

# Every check job in ci.yml, in its order there. ci-result fails the run when
# its needs and this list differ, so a new job must be added to both; one
# gated on a name missing here would read null and be skipped on every event.
JOBS = [
    "fmt",
    "clippy",
    "machete",
    "deny",
    "audit",
    "test",
    "fuzz-replay",
    "leak-guard",
    "test-conpty",
    "smoke",
    "pipe-acl",
    "socket-acl",
    "handoff",
    "oracle",
]

# Jobs that run on the whole workspace: any workspace crate selects them.
WORKSPACE_JOBS = ["fmt", "clippy", "machete", "test", "leak-guard", "test-conpty"]
# Jobs that build and run the tmxr binary: a crate it is built from selects them.
BINARY_JOBS = ["smoke", "pipe-acl", "socket-acl", "handoff", "oracle"]
# Dependency policy: a crate manifest selects these. The lockfile, which
# audit reads, selects everything.
DEPENDENCY_JOBS = ["deny", "machete"]

# Paths that change what every job builds or how it runs.
EVERYTHING = [
    ".github/*",
    ".config/*",
    ".gitattributes",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rustfmt.toml",
    "clippy.toml",
]

# Paths no check job reads. Markdown inside a crate is not here: a crate can
# include it (as tmxr-config does its defaults.toml), so it counts as the crate.
NOTHING = [
    "docs/*",
    "*.md",
    "LICENSE",
    ".editorconfig",
    ".gitignore",
    # Release packaging: read only by the build and publish jobs, which never
    # run on a pull request.
    "pkg/alpine/*",
    "pkg/aur/*",
    "pkg/homebrew/*",
    "pkg/scoop/*",
]

# Paths outside the crates that one job reads.
EXTRA = {
    "pkg/windows/*": ["test-conpty"],
    "deny.toml": ["deny"],
}

BINARY = "tmxr-cli"
ORACLE = "tmxr-compat-oracle"


def matches(path, patterns):
    """`dir/*` covers everything under dir; other patterns match the whole
    path, and a pattern without a slash matches a root-level file only."""
    return any(
        path.startswith(p[:-1]) if p.endswith("/*") else fnmatch(path, p)
        for p in patterns
        if "/" in p or "/" not in path
    )


def metadata(manifest):
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1",
         "--manifest-path", str(manifest)],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def closure(name, deps):
    """`name` and every workspace crate it depends on, directly or not."""
    seen, todo = set(), [name]
    while todo:
        n = todo.pop()
        if n not in seen:
            seen.add(n)
            todo.extend(deps.get(n, ()))
    return seen


def load_workspace():
    """(crate dirs relative to the root, path deps per crate, fuzz's deps)."""
    meta = metadata(ROOT / "Cargo.toml")
    dirs, deps = {}, {}
    for p in meta["packages"]:
        rel = PurePosixPath(Path(p["manifest_path"]).parent.relative_to(ROOT))
        dirs[p["name"]] = str(rel)
        deps[p["name"]] = [d["name"] for d in p["dependencies"] if d.get("path")]
    fuzz = metadata(ROOT / "fuzz" / "Cargo.toml")
    fuzz_deps = set()
    for p in fuzz["packages"]:
        for d in p["dependencies"]:
            if d.get("path"):
                fuzz_deps |= closure(d["name"], deps)
    return dirs, deps, fuzz_deps


def choose(paths, dirs, deps, fuzz_deps):
    """{job: [reasons]}; a job with no reason is skipped."""
    reasons = {job: [] for job in JOBS}

    def select(jobs, why):
        for job in jobs:
            if why not in reasons[job]:
                reasons[job].append(why)

    binary = closure(BINARY, deps)
    oracle = binary | closure(ORACLE, deps)
    for path in paths:
        crate = next(
            (c for c, d in sorted(dirs.items(), key=lambda i: -len(i[1]))
             if path.startswith(d + "/")),
            None,
        )
        if matches(path, EVERYTHING):
            select(JOBS, f"`{path}` affects every job")
        elif crate:
            why = f"crate `{crate}` changed"
            select(WORKSPACE_JOBS, why)
            if path == f"{dirs[crate]}/Cargo.toml":
                select(DEPENDENCY_JOBS, why)
            if crate in binary:
                select(BINARY_JOBS, why)
            elif crate in oracle:
                select(["oracle"], why)
            if crate in fuzz_deps:
                select(["fuzz-replay"], why)
        elif path.startswith("fuzz/"):
            # machete walks the directory tree, so it checks fuzz/ too.
            select(["fuzz-replay", "machete"], "`fuzz/` changed")
        elif extra := next((j for p, j in EXTRA.items() if matches(path, [p])), None):
            select(extra, f"`{path}` changed")
        elif matches(path, NOTHING):
            continue
        else:
            select(JOBS, f"`{path}` matches no rule")
    return reasons


def summary(reasons, header):
    lines = [f"### PR job scope\n\n{header}\n", "| job | runs | why |", "| --- | --- | --- |"]
    for job in JOBS:
        why = reasons[job]
        lines.append(
            f"| `{job}` | {'yes' if why else 'skipped'} | "
            f"{'; '.join(why) if why else 'no changed file is an input of this job'} |"
        )
    return "\n".join(lines) + "\n"


class Rules(unittest.TestCase):
    dirs = {
        "tmxr-cli": "apps/tmxr",
        "tmxr-client": "crates/tmxr-client",
        "tmxr-command": "crates/tmxr-command",
        "tmxr-compat-oracle": "crates/tmxr-compat-oracle",
    }
    deps = {
        "tmxr-cli": ["tmxr-client"],
        "tmxr-client": ["tmxr-command"],
        "tmxr-compat-oracle": ["tmxr-command"],
    }
    fuzz = {"tmxr-command"}

    def run_for(self, *paths):
        r = choose(paths, self.dirs, self.deps, self.fuzz)
        return {j for j, why in r.items() if why}

    def test_docs_only_runs_nothing(self):
        self.assertEqual(self.run_for("docs/backlog.md", "README.md", "CHANGELOG.md"), set())

    def test_unknown_path_runs_everything(self):
        self.assertEqual(self.run_for("docs/a.md", "scripts/new.sh"), set(JOBS))

    def test_workflow_and_lockfile_run_everything(self):
        for p in (".github/workflows/ci.yml", "Cargo.lock", "rust-toolchain.toml"):
            self.assertEqual(self.run_for(p), set(JOBS), p)

    def test_shared_crate_runs_its_dependents(self):
        self.assertEqual(
            self.run_for("crates/tmxr-command/src/lib.rs"),
            set(WORKSPACE_JOBS) | set(BINARY_JOBS) | {"fuzz-replay"},
        )

    def test_crate_markdown_counts_as_the_crate(self):
        self.assertIn("test", self.run_for("crates/tmxr-command/README.md"))

    def test_oracle_crate_skips_the_binary_jobs(self):
        self.assertEqual(
            self.run_for("crates/tmxr-compat-oracle/tests/corpus.rs"),
            set(WORKSPACE_JOBS) | {"oracle"},
        )

    def test_binary_only_crate_skips_fuzz(self):
        self.assertNotIn("fuzz-replay", self.run_for("crates/tmxr-client/src/lib.rs"))

    def test_manifest_runs_dependency_policy(self):
        self.assertTrue({"deny", "machete"} <= self.run_for("crates/tmxr-client/Cargo.toml"))
        self.assertNotIn("audit", self.run_for("crates/tmxr-client/Cargo.toml"))
        self.assertNotIn("deny", self.run_for("crates/tmxr-client/src/lib.rs"))

    def test_fuzz_runs_replay_and_machete(self):
        self.assertEqual(
            self.run_for("fuzz/src/lib.rs", "fuzz/Cargo.lock"), {"fuzz-replay", "machete"}
        )

    def test_conpty_script_runs_its_job(self):
        self.assertEqual(self.run_for("pkg/windows/fetch-conpty.sh"), {"test-conpty"})


class Workspace(unittest.TestCase):
    """The real tree, read through cargo metadata as CI reads it."""

    def test_reads_the_crates_and_their_dependencies(self):
        dirs, deps, fuzz_deps = load_workspace()
        self.assertEqual(dirs[BINARY], "apps/tmxr")
        self.assertIn(ORACLE, dirs)
        self.assertIn("tmxr-command", closure(BINARY, deps))
        self.assertIn("tmxr-command", fuzz_deps)
        self.assertNotIn(BINARY, fuzz_deps)


def main():
    args = sys.argv[1:]
    if args == ["--self-test"]:
        result = unittest.main(argv=[sys.argv[0]], exit=False).result
        sys.exit(0 if result.wasSuccessful() else 1)
    if args[:1] == ["--all"] and len(args) == 2:
        reasons = {job: [args[1]] for job in JOBS}
        header = f"Every job runs: {args[1]}."
    elif not args:
        paths = [line.strip() for line in sys.stdin if line.strip()]
        reasons = choose(paths, *load_workspace())
        header = f"Chosen from the {len(paths)} files this pull request changes."
    else:
        sys.exit("usage: ci-scope.py [--all REASON | --self-test] < changed-paths")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(summary(reasons, header))
    print(json.dumps({job: bool(why) for job, why in reasons.items()}))


if __name__ == "__main__":
    main()
