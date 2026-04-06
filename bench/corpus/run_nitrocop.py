#!/usr/bin/env python3
"""Run nitrocop on a corpus repo with oracle-identical environment.

Single source of truth for how nitrocop is invoked on corpus repos.
Used by check_cop.py, corpus-oracle.yml, verify_cop_locations.py,
corpus_smoke_test.py, and reduce_mismatch.py.

CLI usage:
    python3 bench/corpus/run_nitrocop.py <repo_dir> [options]

Library usage:
    from bench.corpus.run_nitrocop import run_nitrocop
    result = run_nitrocop("/path/to/repo", cop="Style/NegatedWhile")
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

CORPUS_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = CORPUS_DIR.parent.parent
BASELINE_CONFIG = CORPUS_DIR / "baseline_rubocop.yml"
GEN_REPO_CONFIG = CORPUS_DIR / "gen_repo_config.py"


def resolve_binary(binary: str | None = None) -> str:
    """Find the nitrocop binary."""
    if binary:
        return binary
    from_env = os.environ.get("NITROCOP_BIN")
    if from_env:
        return from_env
    cargo_target = os.environ.get("CARGO_TARGET_DIR", "target")
    for profile in ("release", "debug"):
        candidate = PROJECT_ROOT / cargo_target / profile / "nitrocop"
        if candidate.exists():
            return str(candidate)
    return "nitrocop"


def resolve_repo_config(repo_id: str, repo_dir: str) -> str:
    """Get per-repo config via gen_repo_config.py.

    Returns path to config file — either the baseline or a temporary
    overlay with per-repo file exclusions for repos with known issues.
    """
    try:
        result = subprocess.run(
            [sys.executable, str(GEN_REPO_CONFIG), repo_id,
             str(BASELINE_CONFIG), repo_dir],
            capture_output=True, text=True, timeout=10,
        )
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
    except (subprocess.TimeoutExpired, FileNotFoundError):
        pass
    return str(BASELINE_CONFIG)


def build_env(repo_dir: str | None = None) -> dict[str, str]:
    """Build environment variables matching the corpus oracle exactly."""
    env = os.environ.copy()
    env["BUNDLE_GEMFILE"] = str(CORPUS_DIR / "Gemfile")
    env["BUNDLE_PATH"] = str(CORPUS_DIR / "vendor" / "bundle")
    if repo_dir:
        env["GIT_CEILING_DIRECTORIES"] = str(Path(repo_dir).absolute().parent)
    return env


def deduplicate_offenses(offenses: list[dict]) -> int:
    """Count offenses deduplicated by (path, line, cop_name).

    The corpus oracle uses this deduplication, so we must match it.
    """
    seen: set[tuple[str, int, str]] = set()
    for o in offenses:
        key = (o.get("path", ""), o.get("line", 0), o.get("cop_name", ""))
        seen.add(key)
    return len(seen)


def normalize_offenses(offenses: list[dict]) -> list[dict]:
    """Resolve symlinked offense paths and deduplicate by canonical location.

    The corpus oracle runs resolve_symlink_paths.py before diffing results.
    Local reruns need the same normalization so cops don't appear to regress
    purely because the same file was discovered via both canonical and symlink
    paths.

    """
    seen: set[tuple[str, int, str]] = set()
    deduped: list[dict] = []
    for offense in offenses:
        normalized = offense.copy()
        path = normalized.get("path", "")
        if path and os.path.exists(path):
            normalized["path"] = os.path.realpath(path)
        key = (
            normalized.get("path", ""),
            normalized.get("line", 0),
            normalized.get("cop_name", ""),
        )
        if key in seen:
            continue
        seen.add(key)
        deduped.append(normalized)
    return deduped


def run_nitrocop(
    repo_dir: str,
    *,
    cop: str | None = None,
    binary: str | None = None,
    timeout: int = 120,
    cwd: str | None = None,
    config_override: str | None = None,
) -> dict:
    """Run nitrocop on a corpus repo with oracle-identical settings.

    Returns dict with keys: offenses (list), count (int), error (str|None).

    When config_override is set, it replaces the baseline config. Per-repo
    file exclusions from gen_repo_config.py are still applied on top.
    """
    binary = resolve_binary(binary)
    # Use absolute path but don't resolve symlinks — the caller may pass a
    # symlink outside the git tree to match the oracle's file-discovery context.
    repo_path = Path(repo_dir).absolute()
    repo_dir = str(repo_path)
    repo_id = repo_path.name
    if config_override:
        config = config_override
    else:
        config = resolve_repo_config(repo_id, repo_dir)
    env = build_env(repo_dir)

    cmd = [binary, "--preview", "--format", "json", "--no-cache", "--config", config]
    if cop:
        cmd += ["--only", cop]
    cmd.append(repo_dir)

    # Run from outside any git tree to avoid .gitignore interference.
    # Default to /tmp if no cwd provided — the command uses absolute paths
    # so cwd only affects git/ignore behavior.
    effective_cwd = cwd or "/tmp"
    try:
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, env=env,
            cwd=effective_cwd,
        )
    except subprocess.TimeoutExpired:
        return {"raw": "", "offenses": [], "count": -1, "error": f"timeout after {timeout}s"}

    if result.returncode not in (0, 1):
        return {"raw": result.stdout, "offenses": [], "count": -1,
                "error": f"exit code {result.returncode}"}

    try:
        data = json.loads(result.stdout)
        offenses = normalize_offenses(data.get("offenses", []))
        # --only filters which cops run, but Lint/RedundantCopDisableDirective
        # always runs as post-processing. Filter offenses to the requested cop
        # so per-cop corpus counts are not inflated by unrelated offenses.
        if cop:
            offenses = [o for o in offenses if o.get("cop_name") == cop]
        count = deduplicate_offenses(offenses)
        return {"raw": result.stdout, "offenses": offenses, "count": count, "error": None}
    except json.JSONDecodeError as e:
        return {"raw": result.stdout, "offenses": [], "count": -1,
                "error": f"JSON parse error: {e}"}


def main():
    parser = argparse.ArgumentParser(
        description="Run nitrocop on a corpus repo with oracle-identical environment")
    parser.add_argument("repo_dir", help="Path to corpus repo directory")
    parser.add_argument("--only", dest="cop", help="Filter to one cop (e.g., Style/NegatedWhile)")
    parser.add_argument("--binary", help="Path to nitrocop binary")
    parser.add_argument("--timeout", type=int, default=120, help="Timeout in seconds (default: 120)")
    parser.add_argument("--output", help="Write JSON to file instead of stdout")
    args = parser.parse_args()

    result = run_nitrocop(
        args.repo_dir, cop=args.cop, binary=args.binary, timeout=args.timeout,
    )

    if args.output:
        # Write raw nitrocop JSON — the oracle's diff_results.py expects this format
        Path(args.output).write_text(result["raw"] or "{}\n")
        if result["error"]:
            print(f"WARNING: {result['error']}", file=sys.stderr)
    else:
        # Interactive mode — print parsed summary
        summary = {"count": result["count"], "error": result["error"]}
        print(json.dumps(summary, indent=2))

    sys.exit(0 if result["count"] >= 0 else 1)


if __name__ == "__main__":
    main()
