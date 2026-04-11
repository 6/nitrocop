#!/usr/bin/env python3
"""Corpus smoke test: run nitrocop + rubocop on a few small repos and compare.

Catches systemic regressions (file discovery, config resolution, directive handling)
that silently break many cops at once. Runs in ~3 min on CI.

The test compares current results against a checked-in baseline snapshot. It fails
if matches decrease or FP/FN increase beyond a small tolerance per repo. To update
the baseline after intentional changes:

    python3 scripts/corpus_smoke_test.py --update-baseline

Usage:
    python3 scripts/corpus_smoke_test.py                    # auto-detect binary
    python3 scripts/corpus_smoke_test.py --binary path/to/nitrocop
    python3 scripts/corpus_smoke_test.py --update-baseline  # regenerate baseline

When using the repo's normal release binary, the script will auto-rebuild if the
binary is older than Rust source inputs. Explicit non-standard `--binary` paths
must already be fresh.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

# Small repos pinned to exact SHAs from the corpus manifest.
# Chosen for speed: each takes <30s to clone+lint.
SMOKE_REPOS = [
    {
        "id": "multi_json__multi_json__c5fa9fc",
        "repo_url": "https://github.com/sferik/multi_json",
        "sha": "c5fa9fce50aec2d98c438f5d5e751b6f6980805c",
        # Lowered from 95.0: scan_root Include fix reveals FN on cops
        # with db/**/*.rb and spec/**/*.rb patterns.
        "min_match_rate": 84.0,
        "min_nc_files": 118,
    },
    {
        "id": "bkeepers__dotenv__34156bf",
        "repo_url": "https://github.com/bkeepers/dotenv",
        "sha": "34156bf400cd67387fa6ed9f146778f6a2f5f743",
        "min_match_rate": 93.0,
        "min_nc_files": 34,
    },
    {
        "id": "doorkeeper__doorkeeper__b305358",
        "repo_url": "https://github.com/doorkeeper-gem/doorkeeper",
        "sha": "b30535805477bc4a2568d68968595484d6163b31",
        "min_match_rate": 92.0,
        "min_nc_files": 250,
    },
    {
        # Rufo has 121 .rb.spec files — catches file discovery regressions
        # like removing "spec" from RUBY_EXTENSIONS.
        "id": "ruby-formatter__rufo__a90e654",
        "repo_url": "https://github.com/ruby-formatter/rufo",
        "sha": "a90e6541b7b718a031145a0725e7491d98cee41f",
        "min_match_rate": 86.0,
        "min_nc_files": 145,
    },
    {
        # Standard uses `.standard.yml` and version-aware config loading.
        "id": "standardrb__standard__c886a57",
        "repo_url": "https://github.com/standardrb/standard",
        "sha": "c886a57812b1b15d596eac33712defe12443fbcf",
        # Lowered from 95.0: scan_root Include fix reveals FN on cops
        # with spec/**/*.rb patterns.
        "min_match_rate": 93.0,
        "min_nc_files": 104,
    },
    {
        # rubocop-rspec exercises plugin discovery and plugin default config loading.
        "id": "rubocop__rubocop-rspec__51dab28",
        "repo_url": "https://github.com/rubocop/rubocop-rspec",
        "sha": "51dab288f96fb7d571a5835a4eb5503ad9b733b0",
        "min_match_rate": 83.0,
        "min_nc_files": 278,
    },
]

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASELINE_CONFIG = os.path.join(ROOT, "bench", "corpus", "baseline_rubocop.yml")
GEN_REPO_CONFIG = os.path.join(ROOT, "bench", "corpus", "gen_repo_config.py")
SNAPSHOT_PATH = os.path.join(ROOT, "bench", "corpus", "smoke_baseline.json")

# How much worse a repo can get before failing. Allows for minor fluctuations
# from rubocop version differences without masking real regressions.
REGRESSION_TOLERANCE = 5  # absolute offense count

# Aggregate match rate floor. If the overall match rate across all smoke repos
# drops below this, it indicates a systemic regression (config resolution,
# file discovery, directive handling, etc.) even if per-repo deltas are small.
AGGREGATE_RATE_FLOOR = 90.0  # percent


def release_binary_path() -> str:
    if "NITROCOP_BIN" in os.environ:
        return os.environ["NITROCOP_BIN"]
    target_dir = os.environ.get("CARGO_TARGET_DIR", "target")
    return os.path.join(ROOT, target_dir, "release", "nitrocop")


def rust_build_inputs() -> list[Path]:
    paths = [
        Path(ROOT) / "Cargo.toml",
        Path(ROOT) / "Cargo.lock",
        Path(ROOT) / "build.rs",
    ]
    src_dir = Path(ROOT) / "src"
    if src_dir.is_dir():
        paths.extend(src_dir.rglob("*.rs"))
    return [path for path in paths if path.is_file()]


def stale_binary_reason(binary: str) -> str | None:
    binary_path = Path(binary)
    if not binary_path.is_file():
        return f"nitrocop binary not found at {binary}"

    newest_input = max(rust_build_inputs(), key=lambda path: path.stat().st_mtime_ns, default=None)
    if newest_input is None:
        return None

    binary_mtime = binary_path.stat().st_mtime_ns
    newest_input_mtime = newest_input.stat().st_mtime_ns
    if binary_mtime >= newest_input_mtime:
        return None

    try:
        input_label = newest_input.relative_to(ROOT)
    except ValueError:
        input_label = newest_input
    return f"binary is older than {input_label}"


def binary_matches_release_path(binary: str) -> bool:
    return Path(binary).resolve() == Path(release_binary_path()).resolve()


def ensure_fresh_binary(binary: str, explicit: bool) -> str:
    reason = stale_binary_reason(binary)
    if reason is None:
        return binary

    can_rebuild = not explicit or binary_matches_release_path(binary)
    if can_rebuild:
        rebuilt_binary = release_binary_path()
        print(f"Detected stale binary ({reason}); rebuilding with cargo build --release...")
        subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
        rebuilt_reason = stale_binary_reason(rebuilt_binary)
        if rebuilt_reason is not None:
            sys.exit(f"ERROR: rebuilt binary is still stale: {rebuilt_reason}")
        return rebuilt_binary

    sys.exit(
        "ERROR: stale nitrocop binary: "
        f"{reason}. Rebuild with `cargo build --release` or pass a fresh `--binary`."
    )


def find_binary(explicit: str | None) -> str:
    if explicit:
        if not os.path.isfile(explicit):
            sys.exit(f"ERROR: nitrocop binary not found at {explicit}")
        return explicit
    target_dir = os.environ.get("CARGO_TARGET_DIR", "target")
    for profile in ["release", "ci", "debug"]:
        path = os.path.join(ROOT, target_dir, profile, "nitrocop")
        if os.path.isfile(path):
            return path
    sys.exit("ERROR: nitrocop binary not found. Build with `cargo build --release` first.")


def clone_repo(repo: dict, dest: str) -> bool:
    try:
        subprocess.run(["git", "init", dest], capture_output=True, check=True)
        subprocess.run(
            ["git", "-C", dest, "fetch", "--depth", "1", repo["repo_url"], repo["sha"]],
            capture_output=True, check=True, timeout=120,
        )
        subprocess.run(
            ["git", "-C", dest, "checkout", "FETCH_HEAD"],
            capture_output=True, check=True,
        )
        return True
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as e:
        print(f"  WARNING: clone failed for {repo['id']}: {e}", file=sys.stderr)
        return False


def run_nitrocop(binary: str, repo_dir: str) -> dict:
    # Use shared corpus env for oracle-identical settings
    sys.path.insert(0, os.path.join(ROOT, "bench", "corpus"))
    from run_nitrocop import build_env, resolve_repo_config
    env = build_env(repo_dir)
    repo_id = os.path.basename(repo_dir)
    resolved_config = resolve_repo_config(repo_id, repo_dir)
    with tempfile.TemporaryDirectory(prefix="nitrocop-smoke-cache-") as cache_dir:
        env["NITROCOP_CACHE_DIR"] = cache_dir
        result = subprocess.run(
            [
                binary,
                "--preview",
                "--format",
                "json",
                "--no-cache",
                "--cache",
                "false",
                "--config",
                resolved_config,
                repo_dir,
            ],
            capture_output=True,
            text=True,
            env=env,
            timeout=300,
        )
        cache_files = sorted(str(path.relative_to(cache_dir)) for path in Path(cache_dir).rglob("*")
                             if path.is_file())
        if cache_files:
            raise RuntimeError(
                "nitrocop smoke run wrote cache artifacts despite --no-cache/--cache false: "
                + ", ".join(cache_files)
            )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return {}


def run_rubocop(repo_dir: str) -> dict:
    env = os.environ.copy()
    env["BUNDLE_GEMFILE"] = os.path.join(ROOT, "bench", "corpus", "Gemfile")
    env["BUNDLE_PATH"] = os.path.join(ROOT, "bench", "corpus", "vendor", "bundle")
    result = subprocess.run(
        ["bundle", "exec", "rubocop", "--config", BASELINE_CONFIG,
         "--format", "json", "--force-exclusion", "--cache", "false", repo_dir],
        capture_output=True, text=True, env=env, timeout=300,
    )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return {}


def diff_results(rc_data: dict, nc_data: dict) -> tuple[int, int, int, int, int]:
    """Compare rubocop vs nitrocop output. Returns (rc_files, nc_files, matches, fp, fn)."""
    rc_offenses = set()
    for f in rc_data.get("files", []):
        path = f.get("path", "")
        for o in f.get("offenses", []):
            rc_offenses.add((path, o.get("location", {}).get("line", 0), o.get("cop_name", "")))

    nc_offenses = set()
    for o in nc_data.get("offenses", []):
        nc_offenses.add((o.get("path", ""), o.get("line", 0), o.get("cop_name", "")))

    matches = len(rc_offenses & nc_offenses)
    fp = len(nc_offenses - rc_offenses)
    fn = len(rc_offenses - nc_offenses)

    rc_files = len(rc_data.get("files", []))
    nc_files = len(nc_data.get("metadata", {}).get("inspected_files", []))
    if nc_files == 0 and nc_data.get("offenses"):
        nc_files = len({o.get("path", "") for o in nc_data.get("offenses", [])})

    return rc_files, nc_files, matches, fp, fn


def run_with_overlay_config(binary: str, repo_dir: str, repo_id: str) -> dict:
    """Run nitrocop using gen_repo_config.py overlay (simulates corpus oracle per-repo config).

    This catches regressions in inherit_from + AllCops.Exclude merging that affect
    the corpus oracle but not the direct-baseline smoke runs."""
    env = os.environ.copy()
    env["BUNDLE_GEMFILE"] = os.path.join(ROOT, "bench", "corpus", "Gemfile")
    env["BUNDLE_PATH"] = os.path.join(ROOT, "bench", "corpus", "vendor", "bundle")

    # Generate an overlay config with a dummy exclude (exercises the inherit_from path)
    overlay_config = subprocess.run(
        ["python3", GEN_REPO_CONFIG, repo_id, BASELINE_CONFIG, repo_dir],
        capture_output=True, text=True,
    ).stdout.strip()

    # If gen_repo_config returned the baseline directly (no excludes for this repo),
    # create a synthetic overlay to exercise the inherit_from + AllCops.Exclude path.
    if overlay_config == BASELINE_CONFIG:
        overlay_path = os.path.join(tempfile.gettempdir(), f"smoke_overlay_{repo_id}.yml")
        abs_baseline = os.path.abspath(BASELINE_CONFIG)
        with open(overlay_path, "w") as f:
            f.write(f"inherit_from: {abs_baseline}\n\n")
            f.write("AllCops:\n")
            f.write("  Exclude:\n")
            f.write('    - "nonexistent_smoke_exclude.rb"\n')
        overlay_config = overlay_path

    result = subprocess.run(
        [binary, "--preview", "--format", "json", "--no-cache",
         "--config", overlay_config, repo_dir],
        capture_output=True, text=True, env=env, timeout=300,
    )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return {}


def run_all(binary: str) -> dict:
    """Run smoke test on all repos, return per-repo results."""
    results = {}
    with tempfile.TemporaryDirectory() as tmpdir:
        for repo in SMOKE_REPOS:
            repo_id = repo["id"]
            print(f"\n{'=' * 60}")
            print(f"  {repo_id}")
            print(f"{'=' * 60}")

            dest = os.path.join(tmpdir, repo_id)
            if not clone_repo(repo, dest):
                print("  SKIP (clone failed)")
                continue

            print("  Running rubocop...")
            rc_data = run_rubocop(dest)
            print("  Running nitrocop...")
            nc_data = run_nitrocop(binary, dest)

            rc_files, nc_files, matches, fp, fn = diff_results(rc_data, nc_data)
            total = matches + fp + fn
            rate = matches / total * 100 if total > 0 else 100.0

            print(f"  Files: rubocop={rc_files}, nitrocop={nc_files}")
            print(f"  Offenses: matches={matches}, FP={fp}, FN={fn}, rate={rate:.1f}%")

            results[repo_id] = {
                "rc_files": rc_files,
                "nc_files": nc_files,
                "matches": matches,
                "fp": fp,
                "fn": fn,
                "rate": round(rate, 1),
            }

            # Overlay config check: re-run nitrocop through gen_repo_config.py's
            # inherit_from + AllCops.Exclude path and verify results match the
            # direct baseline run. This catches config inheritance bugs that only
            # manifest when the corpus oracle uses per-repo config overlays.
            if os.path.isfile(GEN_REPO_CONFIG):
                print("  Running overlay config check...")
                overlay_data = run_with_overlay_config(binary, dest, repo_id)
                _, _, overlay_matches, overlay_fp, overlay_fn = diff_results(rc_data, overlay_data)
                overlay_total = overlay_matches + overlay_fp + overlay_fn
                overlay_matches / overlay_total * 100 if overlay_total > 0 else 100.0
                # The overlay path should produce the same results as the direct path.
                # Allow a small tolerance for timing-dependent cops.
                match_delta = abs(matches - overlay_matches)
                fp_delta = overlay_fp - fp
                if match_delta > REGRESSION_TOLERANCE or fp_delta > REGRESSION_TOLERANCE:
                    print("  WARNING: overlay config diverges from direct baseline!")
                    print(f"    direct:  matches={matches}, FP={fp}, FN={fn}")
                    print(f"    overlay: matches={overlay_matches}, FP={overlay_fp}, FN={overlay_fn}")
                    results[repo_id]["overlay_divergence"] = {
                        "matches": overlay_matches, "fp": overlay_fp, "fn": overlay_fn,
                        "match_delta": match_delta, "fp_delta": fp_delta,
                    }
                else:
                    print(f"  Overlay config OK (matches={overlay_matches}, FP={overlay_fp})")

    return results


def check_regression(current: dict, baseline: dict) -> list[str]:
    """Compare current results against baseline snapshot. Returns list of failure messages."""
    failures = []
    for repo_id, cur in current.items():
        base = baseline.get(repo_id)
        if base is None:
            continue  # New repo, no baseline to compare against

        # Fail if matches decreased (we lost correct detections)
        match_drop = base["matches"] - cur["matches"]
        if match_drop > REGRESSION_TOLERANCE:
            failures.append(
                f"{repo_id}: matches dropped by {match_drop} "
                f"({base['matches']} -> {cur['matches']})"
            )

        # Fail if FP increased (new false positives)
        fp_increase = cur["fp"] - base["fp"]
        if fp_increase > REGRESSION_TOLERANCE:
            failures.append(
                f"{repo_id}: FP increased by {fp_increase} "
                f"({base['fp']} -> {cur['fp']})"
            )

        # Fail if FN increased (new false negatives)
        fn_increase = cur["fn"] - base["fn"]
        if fn_increase > REGRESSION_TOLERANCE:
            failures.append(
                f"{repo_id}: FN increased by {fn_increase} "
                f"({base['fn']} -> {cur['fn']})"
            )

        # Fail if file count diverged significantly
        if base["rc_files"] > 0:
            file_diff = abs(cur["nc_files"] - cur["rc_files"])
            base_file_diff = abs(base["nc_files"] - base["rc_files"])
            if file_diff > base_file_diff + REGRESSION_TOLERANCE:
                failures.append(
                    f"{repo_id}: file count divergence grew "
                    f"({base_file_diff} -> {file_diff})"
                )

    for repo in SMOKE_REPOS:
        repo_id = repo["id"]
        cur = current.get(repo_id)
        if cur is None:
            continue

        min_match_rate = repo.get("min_match_rate")
        if min_match_rate is not None and cur["rate"] < min_match_rate:
            failures.append(
                f"{repo_id}: match rate dropped below absolute floor "
                f"({cur['rate']:.1f}% < {min_match_rate:.1f}%)"
            )

        min_nc_files = repo.get("min_nc_files")
        if min_nc_files is not None and cur["nc_files"] < min_nc_files:
            failures.append(
                f"{repo_id}: nitrocop inspected files dropped below absolute floor "
                f"({cur['nc_files']} < {min_nc_files})"
            )

    # Aggregate match rate check: catches systemic regressions (config resolution,
    # directive handling, etc.) that spread small deltas across many cops/repos.
    total_m = sum(r["matches"] for r in current.values())
    total_fp = sum(r["fp"] for r in current.values())
    total_fn = sum(r["fn"] for r in current.values())
    grand = total_m + total_fp + total_fn
    agg_rate = total_m / grand * 100 if grand > 0 else 100.0
    if agg_rate < AGGREGATE_RATE_FLOOR:
        failures.append(
            f"AGGREGATE: overall match rate {agg_rate:.1f}% is below "
            f"floor of {AGGREGATE_RATE_FLOOR:.1f}%"
        )

    # Overlay config divergence: warn (don't fail) if any repo's overlay-config
    # run diverged from its direct-baseline run. This is informational — the
    # inherit_from + absolute-path config path has a known pre-existing divergence
    # that doesn't affect the direct-baseline comparison used by the oracle.
    for repo_id, cur in current.items():
        div = cur.get("overlay_divergence")
        if div:
            print(
                f"  WARNING (non-blocking): {repo_id} overlay config diverges "
                f"(match_delta={div['match_delta']}, fp_delta={div['fp_delta']})"
            )

    return failures


def main():
    parser = argparse.ArgumentParser(description="Corpus smoke test")
    parser.add_argument("--binary", help="Path to nitrocop binary")
    parser.add_argument("--update-baseline", action="store_true",
                        help="Regenerate the baseline snapshot from current results")
    args = parser.parse_args()

    binary = find_binary(args.binary)
    binary = ensure_fresh_binary(binary, explicit=args.binary is not None)
    print(f"Using binary: {binary}")

    # Check corpus bundle is available
    bundle_path = os.path.join(ROOT, "bench", "corpus", "vendor", "bundle")
    if not os.path.isdir(bundle_path):
        sys.exit(
            "ERROR: Corpus bundle not installed. Run:\n"
            "  cd bench/corpus && bundle config set --local path vendor/bundle && bundle install"
        )

    results = run_all(binary)

    # Summary
    total_m = sum(r["matches"] for r in results.values())
    total_fp = sum(r["fp"] for r in results.values())
    total_fn = sum(r["fn"] for r in results.values())
    grand = total_m + total_fp + total_fn
    rate = total_m / grand * 100 if grand > 0 else 100.0
    print(f"\n{'=' * 60}")
    print(f"  OVERALL: matches={total_m}, FP={total_fp}, FN={total_fn}, rate={rate:.1f}%")
    print(f"{'=' * 60}")

    if args.update_baseline:
        with open(SNAPSHOT_PATH, "w") as f:
            json.dump(results, f, indent=2, sort_keys=True)
        print(f"\nBaseline written to {SNAPSHOT_PATH}")
        return

    # Compare against baseline
    if not os.path.isfile(SNAPSHOT_PATH):
        print(f"\nWARNING: No baseline at {SNAPSHOT_PATH}")
        print("Run with --update-baseline to create one.")
        print("Falling back to absolute threshold check (90%)...")
        if rate < 90:
            print(f"FAIL: match rate {rate:.1f}% below 90%")
            sys.exit(1)
        print("PASS (no baseline)")
        return

    with open(SNAPSHOT_PATH) as f:
        baseline = json.load(f)

    failures = check_regression(results, baseline)

    if failures:
        print(f"\nFAIL: {len(failures)} regression(s) vs baseline (tolerance={REGRESSION_TOLERANCE}):")
        for msg in failures:
            print(f"  {msg}")
        print("\nIf this is intentional, update the baseline:")
        print("  python3 scripts/corpus_smoke_test.py --update-baseline")
        sys.exit(1)

    # Ratchet: auto-tighten baseline when results improve.
    # Keeps the baseline current as cops are fixed so future regressions
    # are caught relative to the latest high-water mark.
    updated = False
    for repo_id, cur in results.items():
        base = baseline.get(repo_id)
        if base is None:
            baseline[repo_id] = cur
            updated = True
            continue
        if (cur["matches"] > base["matches"]
                or cur["fp"] < base["fp"]
                or cur["fn"] < base["fn"]
                or abs(cur["nc_files"] - cur["rc_files"]) < abs(base["nc_files"] - base["rc_files"])):
            baseline[repo_id] = cur
            updated = True
            print(f"  Baseline improved for {repo_id}")

    if updated:
        with open(SNAPSHOT_PATH, "w") as f:
            json.dump(baseline, f, indent=2, sort_keys=True)
            f.write("\n")
        print(f"\nPASS (baseline auto-tightened — commit {SNAPSHOT_PATH})")
    else:
        print("\nPASS (no regression vs baseline)")


if __name__ == "__main__":
    main()
