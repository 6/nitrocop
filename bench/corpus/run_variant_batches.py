#!/usr/bin/env python3
"""Run style-variant batch configs on a single corpus repo.

Called from the corpus oracle workflow for each repo when style-variant
testing is enabled. Runs both nitrocop and rubocop with each variant
batch config and writes results to the output directories.

Usage (from workflow):
    python3 bench/corpus/run_variant_batches.py \\
        --repo-dir /path/to/repo \\
        --repo-id my_repo__abc123 \\
        --binary bin/nitrocop \\
        --batches-dir bench/corpus/variant_batches \\
        --results-dir results \\
        --timeout 600

Library usage:
    from run_variant_batches import run_variant_batches
    results = run_variant_batches(
        repo_dir="/path/to/repo",
        repo_id="my_repo__abc123",
        binary="bin/nitrocop",
        batches_dir="bench/corpus/variant_batches",
        results_dir="results",
    )
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

CORPUS_DIR = Path(__file__).resolve().parent


def discover_batches(batches_dir: str | Path) -> list[Path]:
    """Find all variant_batch_*.yml files in the given directory."""
    d = Path(batches_dir)
    if not d.exists():
        return []
    return sorted(d.glob("variant_batch_*.yml"))


def build_env(repo_dir: str) -> dict[str, str]:
    """Build environment matching the corpus oracle."""
    env = os.environ.copy()
    env["BUNDLE_GEMFILE"] = str(CORPUS_DIR / "Gemfile")
    env["BUNDLE_PATH"] = str(CORPUS_DIR / "vendor" / "bundle")
    env["GIT_CEILING_DIRECTORIES"] = str(Path(repo_dir).absolute().parent)
    return env


def _repo_manifest_index(repo_id: str, manifest: str | Path) -> int | None:
    """Return the 0-based index of *repo_id* in the manifest, or None."""
    with open(manifest) as f:
        for i, line in enumerate(f):
            line = line.strip()
            if not line:
                continue
            if json.loads(line).get("id") == repo_id:
                return i
    return None


def run_variant_batches(
    *,
    repo_dir: str,
    repo_id: str,
    binary: str,
    batches_dir: str,
    results_dir: str,
    timeout: int = 600,
    manifest: str | None = None,
    max_variant_repos: int | None = None,
    cache_dir: str | None = None,
    repo_sha: str | None = None,
) -> list[dict]:
    """Run all variant batch configs on a repo.

    If *manifest* and *max_variant_repos* are both provided, variants are
    only run when the repo's position in the manifest is < max_variant_repos.
    This keeps total runtime manageable by limiting variants to the top N
    repos (manifest is sorted by stars).

    If *cache_dir* and *repo_sha* are both provided, variant rubocop results
    are cached as ``{cache_dir}/{repo_id}_{sha}_{batch_name}.json``.
    Cached results are reused on subsequent runs when the repo SHA hasn't
    changed, skipping the expensive rubocop invocation.

    Returns list of {batch_name, nitrocop_ok, rubocop_ok} dicts.
    """
    if manifest and max_variant_repos is not None:
        idx = _repo_manifest_index(repo_id, manifest)
        if idx is None or idx >= max_variant_repos:
            print(
                f"Skipping variants for {repo_id} (manifest index {idx}, limit {max_variant_repos})",
                file=sys.stderr,
            )
            return []

    batches = discover_batches(batches_dir)
    if not batches:
        return []

    # Build cop list per batch so we can pass --only to both tools.
    # This is critical for performance: without --only, variant runs
    # check all 915 cops even though only 160/53/14 are overridden.
    # With --only, batch 1 is ~75% faster, batch 2 ~94%, batch 3 ~98%.
    style_map = parse_batch_style_map(batches_dir)

    abs_dest = str(Path(repo_dir).absolute())
    env = build_env(abs_dest)
    results = []
    cache_path = Path(cache_dir) if cache_dir else None

    for batch_config in batches:
        batch_name = batch_config.stem  # e.g. "variant_batch_1"
        nc_dir = Path(results_dir) / f"variant-nitrocop-{batch_name}"
        rc_dir = Path(results_dir) / f"variant-rubocop-{batch_name}"
        nc_dir.mkdir(parents=True, exist_ok=True)
        rc_dir.mkdir(parents=True, exist_ok=True)

        nc_json = nc_dir / f"{repo_id}.json"
        nc_err = nc_dir / f"{repo_id}.err"
        rc_json = rc_dir / f"{repo_id}.json"
        rc_err = rc_dir / f"{repo_id}.err"

        # Build --only flag from cops overridden in this batch
        batch_cops = sorted(style_map.get(batch_name, {}).keys())
        only_flag = ["--only", ",".join(batch_cops)] if batch_cops else []

        # Layer per-repo vendor exclusions on top of the batch config.
        # This ensures variant runs skip vendor/**/* and other junk dirs
        # just like the default-config oracle does via gen_repo_config.py.
        try:
            gen_result = subprocess.run(
                [sys.executable, str(CORPUS_DIR / "gen_repo_config.py"),
                 repo_id, str(batch_config), abs_dest],
                capture_output=True, text=True, timeout=10,
            )
            effective_config = (
                gen_result.stdout.strip()
                if gen_result.returncode == 0 and gen_result.stdout.strip()
                else str(batch_config)
            )
        except (subprocess.TimeoutExpired, FileNotFoundError):
            effective_config = str(batch_config)

        # nitrocop (always run — it's fast)
        nc_ok = _run_tool(
            cmd=[
                binary, "--preview", "--format", "json", "--no-cache",
                "--config", effective_config, *only_flag, abs_dest,
            ],
            env=env, timeout=timeout,
            stdout_path=nc_json, stderr_path=nc_err,
            label=f"variant-nitrocop ({batch_name}, {len(batch_cops)} cops): {repo_id}",
        )

        # rubocop — use cached result if available
        rc_cache_key = f"{repo_id}_{repo_sha}_{batch_name}.json" if repo_sha else None
        rc_cached = False
        if cache_path and rc_cache_key:
            cached_file = cache_path / rc_cache_key
            if cached_file.exists():
                import shutil
                shutil.copy2(str(cached_file), str(rc_json))
                print(
                    f"=== variant-rubocop ({batch_name}): {repo_id} === (cached)",
                    file=sys.stderr,
                )
                rc_ok = True
                rc_cached = True

        if not rc_cached:
            rescue_file = CORPUS_DIR / "rescue_parser_crashes.rb"
            rc_ok = _run_tool(
                cmd=[
                    "bundle", "exec", "rubocop",
                    "--require", str(rescue_file),
                    "--config", effective_config,
                    *only_flag,
                    "--format", "json", "--force-exclusion", "--cache", "false",
                    abs_dest,
                ],
                env=env, timeout=timeout,
                stdout_path=rc_json, stderr_path=rc_err,
                label=f"variant-rubocop ({batch_name}, {len(batch_cops)} cops): {repo_id}",
            )
            # Save to cache
            if rc_ok and cache_path and rc_cache_key and rc_json.exists():
                cache_path.mkdir(parents=True, exist_ok=True)
                import shutil
                shutil.copy2(str(rc_json), str(cache_path / rc_cache_key))

        results.append({
            "batch_name": batch_name,
            "nitrocop_ok": nc_ok,
            "rubocop_ok": rc_ok,
        })

    return results


def _run_tool(
    *,
    cmd: list[str],
    env: dict[str, str],
    timeout: int,
    stdout_path: Path,
    stderr_path: Path,
    label: str,
) -> bool:
    """Run a tool, capturing stdout/stderr to files. Returns True on success."""
    print(f"=== {label} ===", file=sys.stderr)
    try:
        with open(stdout_path, "w") as out, open(stderr_path, "w") as err:
            result = subprocess.run(
                cmd, stdout=out, stderr=err, timeout=timeout, env=env,
            )
        return result.returncode in (0, 1)
    except subprocess.TimeoutExpired:
        print(f"::warning::{label} timed out after {timeout}s", file=sys.stderr)
        return False
    except FileNotFoundError as e:
        print(f"::warning::{label} command not found: {e}", file=sys.stderr)
        return False


def parse_batch_style_map(batches_dir: str | Path) -> dict[str, dict[str, str]]:
    """Parse variant batch YAML configs to extract cop -> style label per batch.

    Returns {batch_name: {cop_name: "value1, value2, ..."}} mapping.
    Only cops with actual style overrides in that batch are included.
    """
    import yaml

    result: dict[str, dict[str, str]] = {}
    for batch_path in sorted(Path(batches_dir).glob("variant_batch_*.yml")):
        batch_name = batch_path.stem
        try:
            data = yaml.safe_load(batch_path.read_text()) or {}
        except Exception:
            continue

        cop_styles: dict[str, str] = {}
        for key, value in data.items():
            if key == "inherit_from" or not isinstance(value, dict):
                continue
            # key is a cop name like "Style/TrailingCommaInHashLiteral"
            # value is {"EnforcedStyle": "comma", ...}
            style_parts = []
            for param, val in sorted(value.items()):
                if param.startswith("Enforced"):
                    style_parts.append(str(val))
            if style_parts:
                cop_styles[key] = ", ".join(style_parts)
        result[batch_name] = cop_styles
    return result


def merge_variant_results(
    result_files: list[Path],
    batches_dir: str | Path | None = None,
) -> dict:
    """Merge multiple style-variant-*.json files into a single summary.

    Each input file is a diff_results.py output for one variant batch.
    When *batches_dir* is provided, enriches each cop entry with a
    ``style_label`` field (e.g. "comma") parsed from the batch YAML config.
    Only cops with actual style overrides in that batch are included in
    the per-cop list.

    Returns a combined dict with a 'batches' key.
    """
    style_map = parse_batch_style_map(batches_dir) if batches_dir else {}
    merged: dict = {"batches": []}
    for f in sorted(result_files):
        try:
            data = json.loads(f.read_text())
        except (json.JSONDecodeError, OSError):
            continue
        batch_name = f.stem.replace("style-variant-", "")
        entry = {"name": batch_name}
        if "summary" in data:
            entry.update(data["summary"])
        if "by_cop" in data:
            batch_cops = style_map.get(batch_name, {})
            if batch_cops:
                # Filter to only cops that have overrides in this batch,
                # and annotate with style labels
                filtered = []
                for cop_entry in data["by_cop"]:
                    cop_name = cop_entry.get("cop", "")
                    if cop_name in batch_cops:
                        cop_entry = dict(cop_entry)
                        cop_entry["style_label"] = batch_cops[cop_name]
                        filtered.append(cop_entry)
                entry["by_cop"] = filtered
            else:
                entry["by_cop"] = data["by_cop"]
        # Preserve per-repo divergence data for per-repo variant baselines.
        # Only repos with FP+FN > 0 are included (filtered by diff_results.py).
        if "by_repo_cop" in data:
            batch_cops = style_map.get(batch_name, {})
            if batch_cops:
                # Filter to only repos/cops with overrides in this batch
                filtered_repo_cop: dict = {}
                for repo_id, cop_data in data["by_repo_cop"].items():
                    filtered_cops = {
                        cop: stats for cop, stats in cop_data.items()
                        if cop in batch_cops
                    }
                    if filtered_cops:
                        filtered_repo_cop[repo_id] = filtered_cops
                entry["by_repo_cop"] = filtered_repo_cop
            else:
                entry["by_repo_cop"] = data["by_repo_cop"]
        merged["batches"].append(entry)
    return merged


def main():
    parser = argparse.ArgumentParser(
        description="Run style-variant batch configs on a corpus repo")
    parser.add_argument("--repo-dir", required=True, help="Path to corpus repo")
    parser.add_argument("--repo-id", required=True, help="Repo identifier")
    parser.add_argument("--binary", required=True, help="Path to nitrocop binary")
    parser.add_argument("--batches-dir", default="bench/corpus/variant_batches",
                        help="Directory containing variant_batch_*.yml files")
    parser.add_argument("--results-dir", default="results",
                        help="Base directory for result output")
    parser.add_argument("--timeout", type=int, default=600,
                        help="Timeout per tool per batch (default: 600s)")
    parser.add_argument("--manifest",
                        help="Path to manifest.jsonl for repo index lookup")
    parser.add_argument("--max-variant-repos", type=int,
                        help="Only run variants on repos in the first N manifest entries")
    parser.add_argument("--cache-dir",
                        help="Directory for caching variant rubocop results")
    parser.add_argument("--repo-sha",
                        help="Repo SHA for cache key (required with --cache-dir)")
    args = parser.parse_args()

    results = run_variant_batches(
        repo_dir=args.repo_dir,
        repo_id=args.repo_id,
        binary=args.binary,
        batches_dir=args.batches_dir,
        results_dir=args.results_dir,
        timeout=args.timeout,
        manifest=args.manifest,
        max_variant_repos=args.max_variant_repos,
        cache_dir=args.cache_dir,
        repo_sha=args.repo_sha,
    )

    for r in results:
        nc_status = "ok" if r["nitrocop_ok"] else "FAIL"
        rc_status = "ok" if r["rubocop_ok"] else "FAIL"
        print(f"  {r['batch_name']}: nitrocop={nc_status} rubocop={rc_status}",
              file=sys.stderr)

    failures = sum(1 for r in results if not r["nitrocop_ok"] or not r["rubocop_ok"])
    sys.exit(1 if failures > 0 else 0)


if __name__ == "__main__":
    main()
