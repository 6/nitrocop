#!/usr/bin/env python3
"""Per-line verification of cop FP/FN against CI corpus oracle data.

Unlike check_cop.py (aggregate counts), this checks SPECIFIC offense locations
from the CI corpus oracle. It runs nitrocop on individual corpus files and
verifies whether each known FP/FN has been fixed.

Usage:
    python3 scripts/verify_cop_locations.py Metrics/AbcSize
    python3 scripts/verify_cop_locations.py Metrics/AbcSize --fp-only
    python3 scripts/verify_cop_locations.py Metrics/AbcSize --fn-only
    python3 scripts/verify_cop_locations.py Metrics/BlockLength Metrics/MethodLength
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Optional

from shared.corpus_artifacts import download_corpus_results


def find_project_root() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True, text=True,
    )
    return Path(result.stdout.strip()) if result.returncode == 0 else Path(".")


def rust_build_inputs(project_root: Path) -> list[Path]:
    """Return files whose mtimes determine whether the release binary is stale."""
    paths = [
        project_root / "Cargo.toml",
        project_root / "Cargo.lock",
        project_root / "build.rs",
    ]
    src_dir = project_root / "src"
    if src_dir.is_dir():
        paths.extend(src_dir.rglob("*.rs"))
    return [path for path in paths if path.is_file()]


def stale_binary_reason(project_root: Path, nitrocop_bin: Path) -> Optional[str]:
    """Return why the release binary is stale, or None when it is fresh."""
    if not nitrocop_bin.is_file():
        return f"nitrocop binary not found at {nitrocop_bin}"

    newest_input = max(
        rust_build_inputs(project_root),
        key=lambda path: path.stat().st_mtime_ns,
        default=None,
    )
    if newest_input is None:
        return None

    binary_mtime = nitrocop_bin.stat().st_mtime_ns
    input_mtime = newest_input.stat().st_mtime_ns
    if binary_mtime >= input_mtime:
        return None

    try:
        input_label = newest_input.relative_to(project_root)
    except ValueError:
        input_label = newest_input
    return f"binary is older than {input_label}"


def ensure_fresh_release_binary(project_root: Path, nitrocop_bin: Path):
    """Build or rebuild the default release binary when missing or stale."""
    reason = stale_binary_reason(project_root, nitrocop_bin)
    if reason is None:
        return

    action = "Building" if not nitrocop_bin.exists() else "Detected stale binary"
    if nitrocop_bin.exists():
        print(f"{action} ({reason}); rebuilding with cargo build --release...", file=sys.stderr)
    else:
        print("Building nitrocop (release)...", file=sys.stderr)

    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=project_root,
        check=True,
    )

    rebuilt_reason = stale_binary_reason(project_root, nitrocop_bin)
    if rebuilt_reason is not None:
        sys.exit(f"ERROR: rebuilt binary is still stale: {rebuilt_reason}")


def parse_loc(loc_str: str) -> tuple[str, str, int]:
    """Parse 'repo_id: filepath:line' into (repo_id, filepath, line)."""
    # Format: "repo_id: path/to/file.rb:123"
    repo_id, rest = loc_str.split(": ", 1)
    # Handle paths with colons by splitting from the right
    last_colon = rest.rfind(":")
    filepath = rest[:last_colon]
    line = int(rest[last_colon + 1:])
    return repo_id, filepath, line


def run_nitrocop_on_repo(
    nitrocop_bin: Path, corpus_dir: Path, config_path: Path,
    repo_id: str, filepaths: list[str], cop_name: str,
    *, config_override: str | None = None,
) -> dict[str, set[int]] | None:
    """Run nitrocop once on all files in a repo, return {filepath: set of offense lines}.

    Returns None when the repo directory doesn't exist or none of the
    requested files are on disk.  Callers must distinguish None (not
    checked) from an empty-set result (checked, no offenses found).

    When *config_override* is set, use that config file instead of the
    per-repo resolved config. This supports ``--style`` variant checks.
    """
    # Import shared corpus runner for oracle-identical env/config
    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench" / "corpus"))
    from run_nitrocop import build_env, resolve_repo_config

    repo_dir = corpus_dir / repo_id
    if not repo_dir.is_dir():
        return None

    existing = []
    for fp in filepaths:
        if (repo_dir / fp).exists():
            existing.append(fp)

    if not existing:
        return None

    result_map: dict[str, set[int]] = {fp: set() for fp in filepaths}

    env = build_env(str(repo_dir))
    resolved_config = config_override if config_override else resolve_repo_config(repo_id, str(repo_dir))

    cmd = [
        str(nitrocop_bin),
        "--only", cop_name,
        "--format", "json",
        "--no-cache",
        "--cache", "false",
        "--config", resolved_config,
        "--preview",
    ] + [str(repo_dir / fp) for fp in existing]

    try:
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=120, env=env,
        )
    except subprocess.TimeoutExpired:
        print(f"    TIMEOUT: {repo_id} ({len(existing)} files)", file=sys.stderr)
        return result_map

    try:
        data = json.loads(result.stdout)
        for o in data.get("offenses", []):
            if o.get("cop_name") != cop_name:
                continue
            line_num = o.get("line") or o.get("location", {}).get("start_line")
            offense_path = o.get("path", "")
            if line_num is None:
                continue
            # Match offense path back to the relative filepath
            for fp in existing:
                if offense_path.endswith(fp) or offense_path == str(repo_dir / fp):
                    result_map[fp].add(line_num)
                    break
    except (json.JSONDecodeError, KeyError):
        pass

    return result_map


def main():
    parser = argparse.ArgumentParser(description="Per-line FP/FN verification")
    parser.add_argument("cops", nargs="+", help="Cop names to verify")
    parser.add_argument("--fp-only", action="store_true", help="Only check FPs")
    parser.add_argument("--fn-only", action="store_true", help="Only check FNs")
    parser.add_argument("--input", type=Path, help="Path to corpus-results.json")
    parser.add_argument("--style", type=str, default=None, metavar="PARAM=VALUE",
                        help="Override a single cop config parameter for nitrocop. "
                             "E.g., --style EnforcedStyleForMultiline=comma. "
                             "Generates a temporary config inheriting from the baseline.")
    args = parser.parse_args()

    project_root = find_project_root()
    corpus_dir = project_root / "vendor" / "corpus"
    config_path = project_root / "bench" / "corpus" / "baseline_rubocop.yml"
    nitrocop_bin = Path(os.environ["NITROCOP_BIN"]) if "NITROCOP_BIN" in os.environ else project_root / os.environ.get("CARGO_TARGET_DIR", "target") / "release" / "nitrocop"

    # Parse --style into a temp config override if specified.
    style_config_override: str | None = None
    if args.style:
        if "=" not in args.style:
            print("ERROR: --style must be PARAM=VALUE (e.g., EnforcedStyle=comma)", file=sys.stderr)
            sys.exit(1)
        style_param, style_value = args.style.split("=", 1)
        # All cops in args.cops share the same style override — this matches
        # check_cop.py's behavior where --style applies to the single cop.
        # When multiple cops are given, use the first cop for the config
        # (the override only affects the specified cop's section anyway).
        cop_for_config = args.cops[0]
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".yml", prefix="nitrocop_style_", delete=False,
        ) as f:
            f.write(f"inherit_from: {config_path}\n\n")
            f.write(f"{cop_for_config}:\n")
            f.write(f"  {style_param}: {style_value}\n")
            style_config_override = f.name
        print(f"Style override: {style_param}={style_value} (config: {style_config_override})",
              file=sys.stderr)

    ensure_fresh_release_binary(project_root, nitrocop_bin)

    # Check corpus bundle is installed
    bundle_dir = project_root / "bench" / "corpus" / "vendor" / "bundle"
    if not bundle_dir.exists():
        print(
            "WARNING: corpus bundle not installed. Results will be wrong!\n"
            "  Fix: cd bench/corpus && BUNDLE_PATH=vendor/bundle bundle install\n",
            file=sys.stderr,
        )
    else:
        env = os.environ.copy()
        env["BUNDLE_GEMFILE"] = str(project_root / "bench" / "corpus" / "Gemfile")
        env["BUNDLE_PATH"] = str(bundle_dir)
        try:
            result = subprocess.run(
                ["bundle", "info", "--path", "rubocop"],
                capture_output=True, text=True, timeout=10,
                cwd=str(project_root / "bench" / "corpus"), env=env,
            )
            if result.returncode != 0:
                print(
                    "WARNING: corpus bundle exists but `bundle info rubocop` failed.\n"
                    f"  stderr: {result.stderr.strip()}\n"
                    "  Fix: cd bench/corpus && BUNDLE_PATH=vendor/bundle bundle install\n",
                    file=sys.stderr,
                )
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass

    # Load corpus results
    if args.input:
        input_path = args.input
    else:
        input_path, run_id, _ = download_corpus_results()
        print(f"Using corpus oracle run {run_id}", file=sys.stderr)

    with open(input_path) as f:
        data = json.load(f)

    by_cop = {c["cop"]: c for c in data.get("by_cop", [])}

    overall_fp_fixed = 0
    overall_fp_remain = 0
    overall_fp_skipped = 0
    overall_fn_fixed = 0
    overall_fn_remain = 0
    overall_fn_skipped = 0

    for cop_name in args.cops:
        cop_data = by_cop.get(cop_name)
        if not cop_data:
            print(f"\n{cop_name}: not found in corpus results")
            continue

        fp_examples = cop_data.get("fp_examples", [])
        fn_examples = cop_data.get("fn_examples", [])

        print(f"\n{'='*70}")
        print(f"{cop_name}  (CI: FP={cop_data['fp']}, FN={cop_data['fn']})")
        print(f"{'='*70}")

        # Collect all (repo_id, filepath) pairs we need to check
        all_examples = []
        if not args.fn_only:
            all_examples.extend(fp_examples)
        if not args.fp_only:
            all_examples.extend(fn_examples)

        # Group files by repo
        repo_files: dict[str, set[str]] = {}
        for ex in all_examples:
            loc = ex["loc"] if isinstance(ex, dict) else ex
            try:
                repo_id, filepath, _line = parse_loc(loc)
                repo_files.setdefault(repo_id, set()).add(filepath)
            except (ValueError, IndexError):
                pass

        # Batch run nitrocop per repo
        # nitrocop_cache values: set[int] = checked, None = repo not cloned
        nitrocop_cache: dict[tuple[str, str], set[int] | None] = {}
        skipped_repos: set[str] = set()
        total_repos = len(repo_files)
        for i, (repo_id, files) in enumerate(sorted(repo_files.items()), 1):
            print(f"\r  Scanning repo {i}/{total_repos}: {repo_id[:50]}...  ",
                  end="", file=sys.stderr, flush=True)
            result_map = run_nitrocop_on_repo(
                nitrocop_bin, corpus_dir, config_path,
                repo_id, sorted(files), cop_name,
                config_override=style_config_override,
            )
            if result_map is None:
                skipped_repos.add(repo_id)
                for fp in files:
                    nitrocop_cache[(repo_id, fp)] = None
            else:
                for fp, lines in result_map.items():
                    nitrocop_cache[(repo_id, fp)] = lines
        if total_repos:
            print(file=sys.stderr)  # clear progress line
        if skipped_repos:
            print(
                f"\n  WARNING: {len(skipped_repos)}/{total_repos} repo(s) not "
                f"cloned locally — those locations cannot be verified.",
                file=sys.stderr,
            )

        # Check FPs
        if not args.fn_only and fp_examples:
            fp_fixed = 0
            fp_remain = 0
            fp_skipped = 0
            print(f"\nFalse Positives ({len(fp_examples)} from CI):")
            for ex in fp_examples:
                loc = ex["loc"] if isinstance(ex, dict) else ex
                msg = ex.get("msg", "") if isinstance(ex, dict) else ""
                try:
                    repo_id, filepath, line = parse_loc(loc)
                except (ValueError, IndexError):
                    print(f"  ? SKIP (can't parse): {loc}")
                    fp_skipped += 1
                    continue

                cached = nitrocop_cache.get((repo_id, filepath))
                if cached is None:
                    print(f"  SKIP    {loc}  (repo not cloned)")
                    fp_skipped += 1
                elif line in cached:
                    print(f"  REMAIN  {loc}")
                    if msg:
                        print(f"          {msg}")
                    fp_remain += 1
                else:
                    print(f"  FIXED   {loc}")
                    fp_fixed += 1

            parts = [f"{fp_fixed} fixed", f"{fp_remain} remain"]
            if fp_skipped:
                parts.append(f"{fp_skipped} skipped")
            print(f"\n  FP summary: {', '.join(parts)}")
            overall_fp_fixed += fp_fixed
            overall_fp_remain += fp_remain
            overall_fp_skipped += fp_skipped

        # Check FNs
        if not args.fp_only and fn_examples:
            fn_fixed = 0
            fn_remain = 0
            fn_skipped = 0
            print(f"\nFalse Negatives ({len(fn_examples)} from CI):")
            for ex in fn_examples:
                loc = ex["loc"] if isinstance(ex, dict) else ex
                msg = ex.get("msg", "") if isinstance(ex, dict) else ""
                try:
                    repo_id, filepath, line = parse_loc(loc)
                except (ValueError, IndexError):
                    print(f"  ? SKIP (can't parse): {loc}")
                    fn_skipped += 1
                    continue

                cached = nitrocop_cache.get((repo_id, filepath))
                if cached is None:
                    print(f"  SKIP    {loc}  (repo not cloned)")
                    fn_skipped += 1
                elif line in cached:
                    print(f"  FIXED   {loc}")
                    fn_fixed += 1
                else:
                    print(f"  REMAIN  {loc}")
                    if msg:
                        print(f"          {msg}")
                    fn_remain += 1

            parts = [f"{fn_fixed} fixed", f"{fn_remain} remain"]
            if fn_skipped:
                parts.append(f"{fn_skipped} skipped")
            print(f"\n  FN summary: {', '.join(parts)}")
            overall_fn_fixed += fn_fixed
            overall_fn_remain += fn_remain
            overall_fn_skipped += fn_skipped

    # Overall summary
    total_skipped = overall_fp_skipped + overall_fn_skipped
    total_checked = (overall_fp_fixed + overall_fp_remain
                     + overall_fn_fixed + overall_fn_remain)
    total_remain = overall_fp_remain + overall_fn_remain

    print(f"\n{'='*70}")
    fp_parts = [f"{overall_fp_fixed} fixed", f"{overall_fp_remain} remain"]
    fn_parts = [f"{overall_fn_fixed} fixed", f"{overall_fn_remain} remain"]
    if overall_fp_skipped:
        fp_parts.append(f"{overall_fp_skipped} skipped")
    if overall_fn_skipped:
        fn_parts.append(f"{overall_fn_skipped} skipped")
    print(f"OVERALL: FP {' / '.join(fp_parts)}, FN {' / '.join(fn_parts)}")

    exit_code = 0 if total_remain == 0 and total_checked > 0 else 1

    if total_skipped and total_checked == 0:
        print(
            f"WARNING: All {total_skipped} location(s) skipped — no corpus "
            f"repos are cloned locally. Results are not meaningful."
        )
        if os.environ.get("CI"):
            print(
                "ERROR: Running in CI with no checkable repos. Use "
                "check_cop.py --rerun --clone instead, or clone repos with "
                "scripts/corpus_repo_map.py --clone.",
                file=sys.stderr,
            )
            exit_code = 2
    elif total_skipped:
        print(
            f"NOTE: {total_skipped} location(s) skipped (repos not cloned). "
            f"Only {total_checked} location(s) were actually verified."
        )
    if total_remain == 0 and total_checked > 0:
        print("ALL CHECKED FP/FN VERIFIED FIXED")
    elif total_remain > 0:
        print(f"{total_remain} issues remain")
    print(f"{'='*70}")

    sys.exit(exit_code)


def _run_tests():
    """Self-tests for pure functions. Run with: python3 scripts/verify_cop_locations.py --test"""
    # parse_loc
    assert parse_loc("repo__id__abc123: path/to/file.rb:42") == ("repo__id__abc123", "path/to/file.rb", 42)
    assert parse_loc("r: a.rb:1") == ("r", "a.rb", 1)
    # Path with colon (Windows-style or special chars)
    assert parse_loc("repo: dir/sub:file.rb:99") == ("repo", "dir/sub:file.rb", 99)

    print("All tests passed.")


if __name__ == "__main__":
    if "--test" in sys.argv:
        _run_tests()
    else:
        main()
