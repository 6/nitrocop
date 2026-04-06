#!/usr/bin/env python3
"""Perf smoke test: time nitrocop on benchmark repos.

Runs nitrocop on vendor/rubocop (small, fast) and optionally on corpus
repos like forem (large, catches scale regressions). Outputs a markdown
summary table and per-repo timing for CI step summaries.

Usage:
    python3 scripts/perf_smoke_test.py --binary target/release/nitrocop
    python3 scripts/perf_smoke_test.py --binary target/release/nitrocop --corpus-repos forem__forem__72d7c44
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def count_rb_files(repo_dir: Path) -> int:
    return sum(1 for _ in repo_dir.rglob("*.rb"))


def resolve_config(repo_id: str, repo_dir: str) -> str | None:
    """Call gen_repo_config.py to get per-repo config with vendor exclusions."""
    gen_script = PROJECT_ROOT / "bench" / "corpus" / "gen_repo_config.py"
    baseline = PROJECT_ROOT / "bench" / "corpus" / "baseline_rubocop.yml"
    try:
        result = subprocess.run(
            [sys.executable, str(gen_script), repo_id, str(baseline), repo_dir],
            capture_output=True, text=True, timeout=10,
        )
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
    except (subprocess.TimeoutExpired, FileNotFoundError) as e:
        print(f"  Warning: gen_repo_config failed: {e}", file=sys.stderr)
    return None


def run_nitrocop(
    binary: str, repo_dir: str, *, config: str | None = None, timeout: int = 120,
) -> dict:
    """Run nitrocop and return timing + offense data."""
    cmd = [binary, "--preview", "--format", "json", "--no-cache"]
    if config:
        cmd += ["--config", config]
    cmd.append(repo_dir)

    start = time.monotonic()
    try:
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout,
        )
        elapsed_ms = int((time.monotonic() - start) * 1000)

        try:
            data = json.loads(result.stdout)
            offense_count = data.get("metadata", {}).get("offense_count", 0)
        except json.JSONDecodeError:
            offense_count = -1

        return {
            "elapsed_ms": elapsed_ms,
            "offense_count": offense_count,
            "timed_out": False,
            "exit_code": result.returncode,
        }
    except subprocess.TimeoutExpired:
        elapsed_ms = int((time.monotonic() - start) * 1000)
        return {
            "elapsed_ms": elapsed_ms,
            "offense_count": -1,
            "timed_out": True,
            "exit_code": -1,
        }


def clone_corpus_repo(repo_id: str) -> Path | None:
    """Clone a corpus repo if not already present."""
    repo_dir = PROJECT_ROOT / "repos" / repo_id
    if repo_dir.exists():
        return repo_dir
    clone_script = PROJECT_ROOT / "bench" / "corpus" / "clone_repos.py"
    manifest = PROJECT_ROOT / "bench" / "corpus" / "manifest.jsonl"
    try:
        subprocess.run(
            [sys.executable, str(clone_script),
             "--dest", str(PROJECT_ROOT),
             "--manifest", str(manifest),
             "--repo-ids", repo_id,
             "--parallel", "1"],
            check=True, timeout=120,
        )
        return repo_dir if repo_dir.exists() else None
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as e:
        print(f"  Warning: clone failed for {repo_id}: {e}", file=sys.stderr)
        return None


def main():
    parser = argparse.ArgumentParser(description="Perf smoke test for nitrocop")
    parser.add_argument("--binary", default="target/release/nitrocop",
                        help="Path to nitrocop binary")
    parser.add_argument("--corpus-repos", nargs="*", default=[],
                        help="Corpus repo IDs to benchmark (cloned if needed)")
    parser.add_argument("--timeout", type=int, default=120,
                        help="Per-repo timeout in seconds (default: 120)")
    args = parser.parse_args()

    binary = str(Path(args.binary).resolve())
    if not Path(binary).exists():
        print(f"Error: binary not found: {binary}", file=sys.stderr)
        sys.exit(1)

    # Build list of repos to test
    repos: list[tuple[str, Path, str | None]] = []  # (label, path, repo_id)

    # Always test vendor/rubocop (already checked out via submodules)
    vendor_rubocop = PROJECT_ROOT / "vendor" / "rubocop"
    if vendor_rubocop.exists():
        repos.append(("vendor/rubocop", vendor_rubocop, None))

    # Clone and test corpus repos
    for repo_id in args.corpus_repos:
        repo_dir = clone_corpus_repo(repo_id)
        if repo_dir:
            repos.append((repo_id, repo_dir, repo_id))
        else:
            print(f"Warning: skipping {repo_id} (clone failed)", file=sys.stderr)

    if not repos:
        print("No repos to test", file=sys.stderr)
        sys.exit(1)

    # Run benchmarks
    results = []
    for label, repo_dir, repo_id in repos:
        file_count = count_rb_files(repo_dir)
        config = None
        if repo_id:
            config = resolve_config(repo_id, str(repo_dir))

        result = run_nitrocop(
            binary, str(repo_dir), config=config, timeout=args.timeout,
        )
        result["label"] = label
        result["file_count"] = file_count
        results.append(result)

        if result["timed_out"]:
            print(f"perf-smoke: {label} files={file_count} time={result['elapsed_ms']}ms TIMEOUT")
        else:
            print(f"perf-smoke: {label} files={file_count} time={result['elapsed_ms']}ms "
                  f"offenses={result['offense_count']}")

    # Write GitHub step summary if available
    summary_file = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_file:
        with open(summary_file, "a") as f:
            f.write("## Perf Smoke Test\n\n")
            f.write("| Repo | Files | Time | Offenses |\n")
            f.write("|------|------:|-----:|---------:|\n")
            for r in results:
                if r["timed_out"]:
                    f.write(f"| {r['label']} | {r['file_count']} | **TIMEOUT** ({args.timeout}s) | - |\n")
                else:
                    secs = f"{r['elapsed_ms'] / 1000:.1f}s"
                    f.write(f"| {r['label']} | {r['file_count']} | {secs} | {r['offense_count']} |\n")

    # Exit 0 always — this is a logging-only check
    sys.exit(0)


if __name__ == "__main__":
    main()
