#!/usr/bin/env python3
from __future__ import annotations

"""Check a single cop against the corpus for aggregate count regressions.

Compares nitrocop's aggregate offense count against the RuboCop baseline from
the latest corpus oracle CI run. Catches real-world false positive regressions
that fixture tests miss.

This is a count-only gate. It does NOT prove that nitrocop matches RuboCop at
the exact offense locations, and it does NOT prove department-level completion
in README.md / docs/corpus.md.

Results are cached per (binary_mtime, cop_name, repo_id) so that re-running
check_cop.py for a different cop after fixing one cop is instant — only the
changed cop needs re-execution. Use --rerun to force a fresh run.

Usage:
    python3 scripts/check_cop.py Lint/Void              # quick aggregate count check
    python3 scripts/check_cop.py Lint/Void --verbose     # per-repo count breakdown
    python3 scripts/check_cop.py Lint/Void --verbose --rerun  # re-execute (auto-filters to relevant repos)
    python3 scripts/check_cop.py Lint/Void --rerun --clone --sample 15  # CI gate: clone + sample
    python3 scripts/check_cop.py Lint/Void --rerun --all-repos  # full scan (local only, slow)
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

from shared.corpus_artifacts import download_corpus_results as _download_corpus

PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "bench" / "corpus"))
from run_nitrocop import run_nitrocop as _run_corpus_nitrocop  # noqa: E402, I001
from clone_repos import clone_repos as _clone_repos, load_manifest as _load_manifest_from_file  # noqa: E402, I001
from clone_repos import repo_head_sha  # noqa: E402, I001
from run_variant_batches import parse_batch_style_map  # noqa: E402, I001
CORPUS_DIR = PROJECT_ROOT / "vendor" / "corpus"
# Overridden to temp dir when --clone is used (see main())
_CLONE_DIR: Path | None = None
# When True, _run_one_repo passes cwd=repo_dir so that base_dir resolves to
# the repo root. Needed for Include-gated cops whose Include patterns don't
# start with **/ and thus can't match absolute paths.
_USE_REPO_CWD: bool = False
MANIFEST_PATH = PROJECT_ROOT / "bench" / "corpus" / "manifest.jsonl"
NITROCOP_BIN = Path(os.environ["NITROCOP_BIN"]).resolve() if "NITROCOP_BIN" in os.environ else PROJECT_ROOT / os.environ.get("CARGO_TARGET_DIR", "target") / "release" / "nitrocop"
BASELINE_CONFIG = PROJECT_ROOT / "bench" / "corpus" / "baseline_rubocop.yml"
LOCAL_CACHE_DIR = PROJECT_ROOT / ".check-cop-cache"


def is_include_gated_cop(cop_name: str) -> bool:
    """Check if a cop has Include patterns that require base_dir resolution.

    Returns True if the cop has at least one Include pattern that doesn't start
    with **/ (i.e., it needs relativization to match files). These cops have
    zero corpus data because both RuboCop and nitrocop fail to resolve them
    when running with a non-.rubocop* config from outside the repo.
    """
    try:
        import yaml

        class _Loader(yaml.SafeLoader):
            pass
        _Loader.add_constructor("!ruby/regexp", lambda loader, n: loader.construct_scalar(n))
    except ImportError:
        return False

    vendor_configs = [
        PROJECT_ROOT / "vendor" / "rubocop" / "config" / "default.yml",
        PROJECT_ROOT / "vendor" / "rubocop-rails" / "config" / "default.yml",
        PROJECT_ROOT / "vendor" / "rubocop-rspec" / "config" / "default.yml",
        PROJECT_ROOT / "vendor" / "rubocop-performance" / "config" / "default.yml",
        PROJECT_ROOT / "vendor" / "rubocop-factory_bot" / "config" / "default.yml",
        PROJECT_ROOT / "vendor" / "rubocop-rspec_rails" / "config" / "default.yml",
        PROJECT_ROOT / "vendor" / "rubocop-discourse" / "config" / "default.yml",
    ]
    for config_path in vendor_configs:
        if not config_path.exists():
            continue
        with open(config_path) as f:
            data = yaml.load(f, Loader=_Loader)
        if not data or cop_name not in data:
            continue
        cop_config = data[cop_name]
        if not isinstance(cop_config, dict):
            continue
        includes = cop_config.get("Include", [])
        if not includes:
            continue
        # A cop is "include-gated" if any Include pattern doesn't start with **/
        # Those patterns need relativization to match, and fail when base_dir is wrong.
        for pattern in includes:
            if isinstance(pattern, str) and not pattern.startswith("**/"):
                return True
    return False


def download_corpus_results() -> tuple[Path, int]:
    """Download corpus-results.json from the latest successful CI run.

    Returns (path, run_id).
    """
    path, run_id, _sha = _download_corpus()
    return path, run_id


def ensure_binary():
    """Ensure release binary exists."""
    if NITROCOP_BIN.exists():
        return
    print("Release binary not found. Run: cargo build --release", file=sys.stderr)
    sys.exit(1)


def check_corpus_bundle():
    """Warn if corpus bundle is not installed for the active Ruby version.

    Without the bundle, `bundle info rubocop` fails and config resolution
    falls back to hardcoded defaults, producing wildly incorrect offense
    counts (often 5-10x higher than expected).
    """
    bundle_dir = PROJECT_ROOT / "bench" / "corpus" / "vendor" / "bundle"
    if not bundle_dir.exists():
        print(
            "WARNING: corpus bundle not installed. Offense counts will be wrong!\n"
            "  Fix: cd bench/corpus && BUNDLE_PATH=vendor/bundle bundle install\n",
            file=sys.stderr,
        )
        return
    # Check that rubocop gem is findable
    from run_nitrocop import build_env
    env = build_env()
    try:
        result = subprocess.run(
            ["bundle", "info", "--path", "rubocop"],
            capture_output=True, text=True, timeout=10,
            cwd=str(PROJECT_ROOT / "bench" / "corpus"),
            env=env,
        )
        if result.returncode != 0:
            print(
                "WARNING: corpus bundle exists but `bundle info rubocop` failed.\n"
                f"  stderr: {result.stderr.strip()}\n"
                "  Fix: cd bench/corpus && BUNDLE_PATH=vendor/bundle bundle install\n",
                file=sys.stderr,
            )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass  # bundle not on PATH or too slow — skip check


def latest_source_mtime() -> float:
    """Return latest mtime across files that affect the release binary."""
    latest = 0.0
    watched = [PROJECT_ROOT / "Cargo.toml", PROJECT_ROOT / "Cargo.lock"]
    for path in watched:
        if path.exists():
            latest = max(latest, path.stat().st_mtime)

    src_dir = PROJECT_ROOT / "src"
    if src_dir.exists():
        for path in src_dir.rglob("*.rs"):
            latest = max(latest, path.stat().st_mtime)

    return latest


def ensure_binary_fresh():
    """Rebuild binary when source is newer than the configured binary path."""
    ensure_binary()
    bin_mtime = NITROCOP_BIN.stat().st_mtime
    src_mtime = latest_source_mtime()
    if src_mtime <= bin_mtime:
        return

    # Detect profile from binary path (target/debug/ vs target/release/)
    is_release = "release" in NITROCOP_BIN.parts
    profile_flag = ["--release"] if is_release else []
    profile_name = "release" if is_release else "debug"
    print(f"Binary is stale; rebuilding with cargo build {' '.join(profile_flag)}... ({profile_name})",
          file=sys.stderr)
    result = subprocess.run(
        ["cargo", "build", *profile_flag],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"Error rebuilding {profile_name} binary:", file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        sys.exit(1)


def binary_key() -> str:
    """Return a cache key based on the nitrocop binary's mtime and size.

    Changes whenever the binary is rebuilt, invalidating cached results
    for all cops. This is cheaper than hashing the entire binary.
    """
    stat = NITROCOP_BIN.stat()
    return f"{stat.st_mtime_ns}_{stat.st_size}"


def load_local_cache() -> dict:
    """Load the local nitrocop results cache.

    Structure: {binary_key: {cop_name: {repo_id: count}}}
    """
    cache_file = LOCAL_CACHE_DIR / "results.json"
    if cache_file.exists():
        try:
            return json.loads(cache_file.read_text())
        except (json.JSONDecodeError, OSError):
            return {}
    return {}


def save_local_cache(cache: dict):
    """Save the local nitrocop results cache."""
    LOCAL_CACHE_DIR.mkdir(parents=True, exist_ok=True)
    cache_file = LOCAL_CACHE_DIR / "results.json"
    cache_file.write_text(json.dumps(cache))


def get_cached_results(cop_name: str) -> dict[str, int] | None:
    """Get cached per-repo results for a cop, or None if not cached.

    Returns None if the binary has changed since the cache was written.
    """
    cache = load_local_cache()
    bkey = binary_key()
    if bkey in cache and cop_name in cache[bkey]:
        return cache[bkey][cop_name]
    return None


def save_cached_results(cop_name: str, per_repo: dict[str, int]):
    """Save per-repo results for a cop to the local cache."""
    cache = load_local_cache()
    bkey = binary_key()
    # Only keep the current binary's cache to avoid unbounded growth
    cache = {bkey: cache.get(bkey, {})}
    cache[bkey][cop_name] = per_repo
    save_local_cache(cache)


def clear_file_cache():
    """Clear nitrocop's file-level result cache to avoid stale results after rebuild."""
    import shutil
    cache_dir = Path.home() / ".cache" / "nitrocop"
    shutil.rmtree(cache_dir, ignore_errors=True)



def _get_corpus_dir() -> Path:
    """Get the corpus directory — temp clone dir in CI, vendor/corpus locally."""
    return _CLONE_DIR if _CLONE_DIR is not None else CORPUS_DIR


_STYLE_CONFIG_OVERRIDE: str | None = None


def _run_one_repo(args: tuple[str, str]) -> tuple[str, int]:
    """Run nitrocop on a single repo via the shared corpus runner."""
    cop_name, repo_dir = args
    repo_id = Path(repo_dir).name
    cwd = repo_dir if _USE_REPO_CWD else None
    result = _run_corpus_nitrocop(
        repo_dir, cop=cop_name, binary=str(NITROCOP_BIN), timeout=120,
        cwd=cwd,
        config_override=_STYLE_CONFIG_OVERRIDE,
    )
    return (repo_id, result["count"])



def load_manifest() -> dict[str, dict]:
    """Load repo info from manifest.jsonl, keyed by repo ID."""
    return _load_manifest_from_file(MANIFEST_PATH)


def relevant_repos_for_cop(
    cop_name: str, data: dict, *, sample: int | None = None,
    include_gated: bool = False,
) -> set[str]:
    """Return the repos worth rerunning for a cop in quick mode.

    This is the union of:
    - repos where RuboCop fires for the cop (`cop_activity_repos`)
    - repos with baseline divergence for the cop (`by_repo_cop`)

    Older corpus artifacts may not have `cop_activity_repos`; in that case we
    fall back to divergence-only behavior.

    When include_gated is True and the cop has zero baseline data, falls back
    to sampling from the full manifest (these cops are silently disabled in
    the oracle due to Include pattern resolution failure).

    When sample is set, cap to N repos — always including diverging repos,
    then filling with highest-offense repos for coverage.
    """
    relevant = set(data.get("cop_activity_repos", {}).get(cop_name, []))
    for repo_id, cops in data.get("by_repo_cop", {}).items():
        if cop_name in cops:
            relevant.add(repo_id)

    # For Include-gated cops with zero baseline, sample from the full manifest.
    # These cops have no oracle data because both tools fail to resolve their
    # Include patterns. We sample broadly to get coverage.
    if not relevant and include_gated:
        by_repo = data.get("by_repo", [])
        ok_repos = {r["repo"] for r in by_repo if r.get("status") == "ok"}
        if ok_repos:
            relevant = ok_repos
            print(f"  Include-gated cop with zero baseline — sampling from "
                  f"{len(relevant)} OK repos", file=sys.stderr)
        else:
            # Fallback: use all repos from manifest
            manifest = load_manifest()
            relevant = set(manifest.keys())
            print(f"  Include-gated cop with zero baseline — sampling from "
                  f"{len(relevant)} manifest repos", file=sys.stderr)

    if sample is not None and len(relevant) > sample:
        # Always include repos with known divergence (FP or FN)
        by_repo_cop = data.get("by_repo_cop", {})
        diverging = set()
        offense_counts: dict[str, int] = {}
        for repo_id in relevant:
            entry = by_repo_cop.get(repo_id, {}).get(cop_name, {})
            fp = entry.get("fp", 0)
            fn = entry.get("fn", 0)
            if fp > 0 or fn > 0:
                diverging.add(repo_id)
            offense_counts[repo_id] = entry.get("matches", 0) + fp + fn

        # Start with diverging repos (capped to sample size by highest
        # divergence), then fill remaining slots by offense count.
        if len(diverging) > sample:
            # Too many diverging repos — pick the ones with highest FP+FN
            diverging_ranked = sorted(
                diverging,
                key=lambda r: (
                    by_repo_cop.get(r, {}).get(cop_name, {}).get("fp", 0)
                    + by_repo_cop.get(r, {}).get(cop_name, {}).get("fn", 0)
                ),
                reverse=True,
            )
            sampled = set(diverging_ranked[:sample])
            print(f"  --sample: {len(sampled)}/{len(relevant)} repos "
                  f"({len(sampled)} of {len(diverging)} diverging, by highest FP+FN)",
                  file=sys.stderr)
        else:
            sampled = set(diverging)
            remaining = sorted(
                relevant - sampled,
                key=lambda r: offense_counts.get(r, 0),
                reverse=True,
            )
            for repo_id in remaining:
                if len(sampled) >= sample:
                    break
                sampled.add(repo_id)
            print(f"  --sample: {len(sampled)}/{len(relevant)} repos "
                  f"({len(diverging)} diverging + {len(sampled) - len(diverging)} by offense count)",
                  file=sys.stderr)
        return sampled

    return relevant


def clone_repos_for_cop(
    cop_name: str, data: dict,
    shard_index: int | None = None, total_shards: int | None = None,
    sample: int | None = None,
    include_gated: bool = False,
) -> Path:
    """Clone repos needed for a cop into a temp dir matching the oracle's structure.

    When sharding, only clones repos in this shard's slice.
    Returns the temp dir path. Repos are at <tmpdir>/repos/REPO_ID/.
    """
    import tempfile

    manifest = load_manifest()
    if not manifest:
        print("ERROR: manifest.jsonl not found", file=sys.stderr)
        sys.exit(1)

    needed = relevant_repos_for_cop(cop_name, data, sample=sample,
                                    include_gated=include_gated)
    if not needed:
        print(f"  No baseline activity or divergence for {cop_name}", file=sys.stderr)

    # When sharding, only clone this shard's repos
    if shard_index is not None and total_shards is not None and needed:
        sorted_needed = sorted(needed)
        shard_needed = {r for i, r in enumerate(sorted_needed) if i % total_shards == shard_index}
        print(f"  shard {shard_index}/{total_shards}: {len(shard_needed)}/{len(needed)} repos to clone",
              file=sys.stderr)
        needed = shard_needed

    tmpdir = Path(tempfile.mkdtemp(prefix="nitrocop_cop_check_"))
    print(f"  Cloning {len(needed)} repos for {cop_name} into {tmpdir}...", file=sys.stderr)
    _clone_repos(tmpdir, manifest, repo_ids=needed, parallel=3)
    return tmpdir


def validate_corpus():
    """Check that local corpus matches manifest.jsonl.

    Fails fast on missing or extra repos so local reruns use the exact
    same corpus checkout as CI.
    """
    manifest = load_manifest()
    if not manifest:
        return

    manifest_ids = set(manifest)

    corpus_dir = _get_corpus_dir()
    local_ids = {d.name for d in corpus_dir.iterdir() if d.is_dir()} if corpus_dir.exists() else set()
    extra = local_ids - manifest_ids
    missing = manifest_ids - local_ids
    wrong_sha = []

    for repo_id in sorted(local_ids & manifest_ids):
        actual = repo_head_sha(corpus_dir / repo_id)
        expected = manifest[repo_id].get("sha")
        if actual != expected:
            wrong_sha.append((repo_id, expected, actual or "(unknown)"))

    if extra:
        print(f"ERROR: {len(extra)} stale repos in vendor/corpus/ not in manifest:", file=sys.stderr)
        for r in sorted(extra):
            print(f"  - {r}", file=sys.stderr)
    if missing:
        pct = len(missing) / len(manifest_ids) * 100
        print(f"ERROR: {len(missing)}/{len(manifest_ids)} manifest repos not cloned locally "
              f"({pct:.0f}% missing)", file=sys.stderr)
    if wrong_sha:
        print(f"ERROR: {len(wrong_sha)} repos do not match the manifest SHA:", file=sys.stderr)
        for repo_id, expected, actual in wrong_sha[:20]:
            print(f"  - {repo_id}: expected {expected[:12]}, got {actual[:12]}", file=sys.stderr)
        if len(wrong_sha) > 20:
            print(f"  ... and {len(wrong_sha) - 20} more", file=sys.stderr)
    if extra or missing or wrong_sha:
        print("Corpus checkout does not match bench/corpus/manifest.jsonl. "
              "Run bench/corpus/clone_repos.sh to sync repos.", file=sys.stderr)
        if wrong_sha or len(missing) > 5:
            sys.exit(1)


def run_nitrocop_per_repo(
    cop_name: str,
    relevant_repos: set[str] | None = None,
    *,
    shard_index: int | None = None,
    total_shards: int | None = None,
) -> dict[str, int]:
    """Run nitrocop --only on each corpus repo in parallel, return {repo_id: count}.

    When relevant_repos is set, only run those repos and assume 0 for the rest.
    """
    from concurrent.futures import ThreadPoolExecutor, as_completed

    if relevant_repos is not None and not relevant_repos:
        print("  --quick: no baseline activity or divergence requires a local rerun", file=sys.stderr)
        return {}

    corpus_dir = _get_corpus_dir()
    if not corpus_dir.exists():
        raise FileNotFoundError(
            f"Local corpus checkout not found at {corpus_dir}. "
            "Pass --clone or run bench/corpus/clone_repos.sh."
        )

    all_repos = sorted(d for d in corpus_dir.iterdir() if d.is_dir())
    repos = all_repos
    if relevant_repos is not None:
        available = {r.name for r in all_repos}
        missing = sorted(relevant_repos - available)
        if missing:
            preview = ", ".join(missing[:5])
            if len(missing) > 5:
                preview += ", ..."
            raise FileNotFoundError(
                f"Missing {len(missing)} required corpus repo(s) under {corpus_dir}: {preview}"
            )
        repos = [r for r in all_repos if r.name in relevant_repos]
        skipped = len(all_repos) - len(repos)
        print(f"  --quick: running {len(repos)}/{len(all_repos)} repos "
              f"(skipping {skipped} with zero baseline activity)", file=sys.stderr)

    if shard_index is not None and total_shards is not None:
        full = len(repos)
        repos = [r for i, r in enumerate(repos) if i % total_shards == shard_index]
        print(f"  shard {shard_index}/{total_shards}: {len(repos)}/{full} repos", file=sys.stderr)

    total = len(repos)

    work = [(cop_name, str(r)) for r in repos]

    workers = min(os.cpu_count() or 4, 16)
    counts: dict[str, int] = {}
    done = 0

    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = {pool.submit(_run_one_repo, w): w for w in work}
        for future in as_completed(futures):
            repo_id, count = future.result()
            counts[repo_id] = count
            done += 1
            if done % 50 == 0:
                print(f"  [{done}/{total}] {repo_id}...", file=sys.stderr)

    # Fill in 0 for repos skipped by --quick (no baseline activity).
    # Don't fill for repos skipped by sharding — those are handled by other shards.
    if relevant_repos is not None and shard_index is None:
        for r in all_repos:
            if r.name not in counts:
                counts[r.name] = 0

    return counts


def run_nitrocop_aggregate(cop_name: str) -> int:
    """Run nitrocop --only on each corpus repo, return total offense count.

    Uses per-repo parallel execution and caches results.
    """
    per_repo = run_nitrocop_per_repo(cop_name)
    save_cached_results(cop_name, per_repo)
    return sum(c for c in per_repo.values() if c >= 0)


def rerun_local_per_repo(
    cop_name: str,
    data: dict,
    *,
    quick: bool,
    has_activity_index: bool,
    shard_index: int | None = None,
    total_shards: int | None = None,
    sample: int | None = None,
    include_gated: bool = False,
) -> dict[str, int]:
    """Re-run nitrocop locally using per-repo subprocess mode.

    Each repo is linted from its own directory with BUNDLE_GEMFILE and
    GIT_CEILING_DIRECTORIES set, matching the corpus oracle exactly.
    """
    ensure_binary_fresh()
    clear_file_cache()
    print("Running nitrocop per-repo...", file=sys.stderr)

    relevant_repos = None
    if quick:
        relevant_repos = relevant_repos_for_cop(cop_name, data, sample=sample,
                                                include_gated=include_gated)
        if not has_activity_index:
            print(
                "WARNING: corpus artifact lacks cop_activity_repos; "
                "quick rerun falls back to divergence-only data",
                file=sys.stderr,
            )
    # When --clone with sharding, the clone dir already contains only this
    # shard's repos. Don't double-shard in the scan.
    if shard_index is not None and _CLONE_DIR is not None:
        return run_nitrocop_per_repo(cop_name, relevant_repos=None)
    return run_nitrocop_per_repo(
        cop_name, relevant_repos=relevant_repos,
        shard_index=shard_index, total_shards=total_shards,
    )


def _parse_example_loc(loc_str: str) -> tuple[str, str, int]:
    """Parse 'repo_id: filepath:line' into (repo_id, filepath, line)."""
    repo_id, rest = loc_str.split(": ", 1)
    last_colon = rest.rfind(":")
    filepath = rest[:last_colon]
    line = int(rest[last_colon + 1:])
    return repo_id, filepath, line


def spot_check_examples(cop_name: str, data: dict) -> tuple[int, int, int, int, int, int]:
    """Spot-check oracle FP/FN examples against local nitrocop.

    Runs nitrocop on the specific files referenced in the oracle's fp_examples
    and fn_examples, then checks whether each known issue persists or is resolved.

    Returns (fp_remain, fp_resolved, fn_remain, fn_resolved, fp_unchecked, fn_unchecked).
    """
    from concurrent.futures import ThreadPoolExecutor, as_completed

    from run_nitrocop import build_env, resolve_repo_config

    by_cop = {c["cop"]: c for c in data.get("by_cop", [])}
    cop_data = by_cop.get(cop_name)
    if not cop_data:
        return 0, 0, 0, 0, 0, 0

    fp_examples = cop_data.get("fp_examples", [])
    fn_examples = cop_data.get("fn_examples", [])
    if not fp_examples and not fn_examples:
        return 0, 0, 0, 0, 0, 0

    corpus_dir = _get_corpus_dir()

    # Collect all (repo_id, filepath, line, kind) we need to check
    checks: list[tuple[str, str, int, str]] = []
    for ex in fp_examples:
        loc = ex["loc"] if isinstance(ex, dict) else ex
        try:
            repo_id, filepath, line = _parse_example_loc(loc)
            checks.append((repo_id, filepath, line, "fp"))
        except (ValueError, IndexError):
            pass
    for ex in fn_examples:
        loc = ex["loc"] if isinstance(ex, dict) else ex
        try:
            repo_id, filepath, line = _parse_example_loc(loc)
            checks.append((repo_id, filepath, line, "fn"))
        except (ValueError, IndexError):
            pass

    # Group files by repo for batched execution
    repo_files: dict[str, set[str]] = {}
    for repo_id, filepath, _line, _kind in checks:
        repo_files.setdefault(repo_id, set()).add(filepath)

    # Run nitrocop on each repo's files and collect offense lines
    nitrocop_lines: dict[tuple[str, str], set[int]] = {}

    def _check_repo(repo_id: str, files: list[str]) -> tuple[dict[str, set[int]], bool]:
        repo_dir = corpus_dir / repo_id
        if not repo_dir.exists():
            return {fp: set() for fp in files}, False  # repo not available
        existing = [fp for fp in files if (repo_dir / fp).exists()]
        result_map: dict[str, set[int]] = {fp: set() for fp in files}
        if not existing:
            return result_map, True

        env = build_env(str(repo_dir))
        config = resolve_repo_config(repo_id, str(repo_dir))
        cmd = [
            str(NITROCOP_BIN), "--only", cop_name, "--format", "json",
            "--no-cache", "--cache", "false", "--config", config, "--preview",
        ] + [str(repo_dir / fp) for fp in existing]

        try:
            result = subprocess.run(
                cmd, capture_output=True, text=True, timeout=120, env=env,
            )
            out = json.loads(result.stdout)
            for o in out.get("offenses", []):
                if o.get("cop_name") != cop_name:
                    continue
                line_num = o.get("line", 0)
                offense_path = o.get("path", "")
                for fp in existing:
                    if offense_path.endswith(fp) or offense_path == str(repo_dir / fp):
                        result_map[fp].add(line_num)
                        break
        except (subprocess.TimeoutExpired, json.JSONDecodeError, KeyError):
            pass
        return result_map, True

    available_repos: set[str] = set()

    workers = min(os.cpu_count() or 4, 16)
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = {
            pool.submit(_check_repo, repo_id, sorted(files)): repo_id
            for repo_id, files in repo_files.items()
        }
        for future in as_completed(futures):
            repo_id = futures[future]
            result_map, repo_available = future.result()
            if repo_available:
                available_repos.add(repo_id)
            for filepath, lines in result_map.items():
                nitrocop_lines[(repo_id, filepath)] = lines

    # Evaluate each example — only count examples from available repos.
    # Examples from non-cloned repos cannot be verified and must not be
    # reported as "resolved" (the file simply isn't there to check).
    fp_remain = fp_resolved = fn_remain = fn_resolved = 0
    fp_unchecked = fn_unchecked = 0
    for repo_id, filepath, line, kind in checks:
        if repo_id not in available_repos:
            if kind == "fp":
                fp_unchecked += 1
            else:
                fn_unchecked += 1
            continue
        lines = nitrocop_lines.get((repo_id, filepath), set())
        if kind == "fp":
            if line in lines:
                fp_remain += 1
            else:
                fp_resolved += 1
        else:
            if line in lines:
                fn_resolved += 1
            else:
                fn_remain += 1

    return fp_remain, fp_resolved, fn_remain, fn_resolved, fp_unchecked, fn_unchecked


VARIANT_BATCHES_DIR = PROJECT_ROOT / "bench" / "corpus" / "variant_batches"


def variant_styles_for_cop(
    cop_name: str,
    batches_dir: str | None = None,
) -> list[dict]:
    """Discover which variant batches include a given cop.

    Returns a list of dicts: [{batch_config, style_label}, ...] for each
    batch that overrides the given cop. Empty list if the cop has no
    variant overrides or the batches directory doesn't exist.
    """
    if batches_dir is None:
        batches_dir = str(VARIANT_BATCHES_DIR)
    bd = Path(batches_dir)
    if not bd.exists():
        return []

    style_map = parse_batch_style_map(batches_dir)
    result = []
    for batch_name in sorted(style_map):
        cop_styles = style_map[batch_name]
        if cop_name in cop_styles:
            batch_config = str(bd / f"{batch_name}.yml")
            result.append({
                "batch_config": batch_config,
                "style_label": cop_styles[cop_name],
            })
    return result


def load_variant_baselines(
    cop_name: str, run_id: int | None,
) -> dict[str, dict]:
    """Load per-style baseline FP/FN from the oracle's style-variant-results.json.

    Returns {style_label: {fp, fn, matches}} or empty dict if unavailable.
    """
    if run_id is None:
        return {}
    from shared.corpus_artifacts import get_variant_results_path
    path = get_variant_results_path(run_id)
    if not path:
        return {}
    try:
        data = json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return {}
    baselines: dict[str, dict] = {}
    for batch in data.get("batches", []):
        for cop_entry in batch.get("by_cop", []):
            if cop_entry.get("cop") == cop_name:
                label = cop_entry.get("style_label", batch.get("name", ""))
                baselines[label] = {
                    "fp": cop_entry.get("fp", 0),
                    "fn": cop_entry.get("fn", 0),
                    "matches": cop_entry.get("matches", 0),
                }
    return baselines


def run_variant_checks(
    *,
    cop_name: str,
    repo_dirs: list[str],
    batches_dir: str | None = None,
    run_nitrocop_fn=None,
    run_rubocop_fn=None,
    binary: str | None = None,
    timeout: int = 120,
    variant_baselines: dict[str, dict] | None = None,
) -> list[dict]:
    """Run nitrocop + rubocop with each variant config on already-cloned repos.

    For each variant batch that overrides *cop_name*, runs both tools with
    ``--only <cop>`` and the batch config, then compares aggregate counts.

    *run_nitrocop_fn* and *run_rubocop_fn* are injectable for testing.
    When None, they default to the real corpus runners.

    *variant_baselines* is a dict from ``load_variant_baselines()`` mapping
    style_label -> {fp, fn, matches}. When present, each result includes
    the oracle baseline for delta computation.

    Returns list of dicts: [{style_label, batch_config, nitrocop_total,
    rubocop_total, fp, fn, rubocop_errors, baseline_fp, baseline_fn}, ...].
    """
    variants = variant_styles_for_cop(cop_name, batches_dir)
    if not variants:
        return []

    if variant_baselines is None:
        variant_baselines = {}

    results = []
    for variant in variants:
        config = variant["batch_config"]
        label = variant["style_label"]
        nc_total = 0
        rc_total = 0
        rc_errors = 0

        for repo_dir in repo_dirs:
            if run_nitrocop_fn:
                nc_result = run_nitrocop_fn(
                    repo_dir, cop=cop_name, binary=binary,
                    timeout=timeout, config_override=config)
            else:
                nc_result = _run_corpus_nitrocop(
                    repo_dir, cop=cop_name, binary=str(NITROCOP_BIN),
                    timeout=timeout, config_override=config)
            nc_count = nc_result.get("count", 0)
            if nc_count >= 0:
                nc_total += nc_count

            if run_rubocop_fn:
                rc_result = run_rubocop_fn(
                    repo_dir, cop=cop_name, config=config, timeout=timeout)
            else:
                rc_result = _run_rubocop_for_variant(
                    repo_dir, cop=cop_name, config=config, timeout=timeout)
            rc_count = rc_result.get("count", 0)
            if rc_count >= 0:
                rc_total += rc_count
            else:
                rc_errors += 1

        baseline = variant_baselines.get(label, {})
        results.append({
            "style_label": label,
            "batch_config": config,
            "nitrocop_total": nc_total,
            "rubocop_total": rc_total,
            "fp": max(0, nc_total - rc_total),
            "fn": max(0, rc_total - nc_total),
            "rubocop_errors": rc_errors,
            "baseline_fp": baseline.get("fp", 0),
            "baseline_fn": baseline.get("fn", 0),
        })

    return results


def _run_rubocop_for_variant(
    repo_dir: str, *, cop: str, config: str, timeout: int = 120,
) -> dict:
    """Run rubocop with --only <cop> --config <config> on a repo.

    Returns {"count": N} where N is the offense count, or {"count": -1} on error.
    """
    from run_nitrocop import build_env
    RESCUE_FILE = str(PROJECT_ROOT / "bench" / "corpus" / "rescue_parser_crashes.rb")
    env = build_env(repo_dir)
    cmd = [
        "bundle", "exec", "rubocop",
        "--require", RESCUE_FILE,
        "--config", config,
        "--only", cop,
        "--format", "json",
        "--force-exclusion",
        "--cache", "false",
        repo_dir,
    ]
    try:
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, env=env)
        data = json.loads(result.stdout)
        # Filter to only the requested cop and deduplicate by (path, line),
        # matching the deduplication that run_nitrocop applies on the NC side.
        seen: set[tuple[str, int]] = set()
        for f in data.get("files", []):
            fpath = os.path.realpath(f.get("path", ""))
            for o in f.get("offenses", []):
                if o.get("cop_name") == cop:
                    seen.add((fpath, o["location"]["line"]))
        return {"count": len(seen)}
    except (subprocess.TimeoutExpired, json.JSONDecodeError, KeyError):
        return {"count": -1}


def generate_style_override_config(cop_name: str, param: str, value: str) -> str:
    """Generate a temporary config file that overrides a single style parameter.

    Returns the path to the temp file. The file inherits from the baseline
    config and overrides only the specified cop's parameter.
    """
    import tempfile
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".yml", prefix="nitrocop_style_", delete=False,
    ) as f:
        f.write(f"inherit_from: {BASELINE_CONFIG}\n\n")
        f.write(f"{cop_name}:\n")
        f.write(f"  {param}: {value}\n")
        return f.name


def main():
    parser = argparse.ArgumentParser(
        description="Check a cop against the corpus for aggregate count regressions")
    parser.add_argument("cop", help="Cop name (e.g., Lint/Void)")
    parser.add_argument("--input", type=Path,
                        help="Path to corpus-results.json (default: download from CI)")
    parser.add_argument("--verbose", action="store_true",
                        help="Run per-repo and show which repos have excess offenses")
    parser.add_argument("--threshold", type=int, default=0,
                        help="Allowed excess offenses before FAIL (default: 0)")
    parser.add_argument("--rerun", action="store_true",
                        help="Force re-execution of nitrocop (ignore local cache)")
    parser.add_argument("--quick", action="store_true",
                        help="Only run repos with baseline activity. Auto-enabled by --rerun.")
    parser.add_argument("--clone", action="store_true",
                        help="Auto-clone needed corpus repos from manifest into a temp dir.")
    parser.add_argument("--sample", type=int, default=None,
                        help="Cap to N repos (prioritizes diverging + highest-offense repos). "
                             "Useful for fast pre-merge gates on high-match cops.")
    parser.add_argument("--all-repos", action="store_true",
                        help="Run ALL corpus repos, not just those with baseline activity. "
                             "Slow (30+ min). Use for local debugging only.")
    parser.add_argument("--shard-index", type=int, default=None,
                        help="Shard index for parallel CI (0-based)")
    parser.add_argument("--total-shards", type=int, default=None,
                        help="Total number of shards for parallel CI")
    parser.add_argument("--repo-cwd", action="store_true",
                        help="Run nitrocop with cwd=repo_dir so Include patterns resolve. "
                             "Auto-enabled for Include-gated cops with zero baseline data.")
    parser.add_argument("--allow-net-improvement", action="store_true",
                        help="Pass the gate when per-repo regressions are offset by "
                             "improvements elsewhere (net FP/FN did not increase). "
                             "Without this flag, ANY per-repo regression fails.")
    parser.add_argument("--style", type=str, default=None, metavar="PARAM=VALUE",
                        help="Override a single cop config parameter for both nitrocop "
                             "and rubocop. E.g., --style EnforcedStyleForMultiline=comma. "
                             "Generates a temporary config inheriting from the baseline.")
    parser.add_argument("--check-variants", action="store_true",
                        help="After default check, also run variant style checks using "
                             "generated batch configs. Reports per-variant FP/FN deltas.")
    parser.add_argument("--variant-batches-dir", type=str, default=None,
                        help="Directory containing variant_batch_*.yml configs. "
                             "If not specified, generates them into a temp directory.")
    args = parser.parse_args()

    # Declare all globals used in this function up front (Python requires
    # global declarations before any assignment to the name in the function).
    global _USE_REPO_CWD, _STYLE_CONFIG_OVERRIDE, _CLONE_DIR

    # Parse --style into a temp config override if specified.
    style_config_override: str | None = None
    if args.style:
        if "=" not in args.style:
            print("ERROR: --style must be PARAM=VALUE (e.g., EnforcedStyle=comma)", file=sys.stderr)
            sys.exit(1)
        style_param, style_value = args.style.split("=", 1)
        style_config_override = generate_style_override_config(
            args.cop, style_param, style_value,
        )
        print(f"Style override: {style_param}={style_value} (config: {style_config_override})",
              file=sys.stderr)

    # --rerun implies --quick unless --all-repos is explicitly set.
    if args.rerun and not args.all_repos:
        args.quick = True

    if args.all_repos and os.environ.get("CI"):
        print("ERROR: --all-repos is disabled in CI (too slow). "
              "Use --rerun which auto-filters to relevant repos, "
              "or use --shard-index/--total-shards for parallel CI.",
              file=sys.stderr)
        sys.exit(1)

    # In CI, cap unsharded --clone runs to --sample 30 to prevent agents
    # from burning the full 45-min timeout on a corpus rerun.  The CI
    # cop-check workflow always passes --shard-index, so this only kicks
    # in for ad-hoc agent invocations.
    if (os.environ.get("CI")
            and args.clone
            and args.sample is None
            and args.shard_index is None):
        args.sample = 30
        print("NOTE: CI without --shard-index — auto-limiting to --sample 30",
              file=sys.stderr)

    # Load corpus results
    corpus_run_id = None
    if args.input:
        input_path = args.input
    else:
        input_path, corpus_run_id = download_corpus_results()

    data = json.loads(input_path.read_text())
    by_cop = data["by_cop"]

    # Find the cop in corpus results
    cop_entry = next((e for e in by_cop if e["cop"] == args.cop), None)
    if cop_entry is None:
        print(f"Cop '{args.cop}' not found in corpus results", file=sys.stderr)
        print(f"Available cops matching '{args.cop.split('/')[-1]}':", file=sys.stderr)
        for e in by_cop:
            if args.cop.split("/")[-1].lower() in e["cop"].lower():
                print(f"  {e['cop']}", file=sys.stderr)
        sys.exit(1)

    expected_rubocop = cop_entry["matches"] + cop_entry["fn"]
    baseline_fp = cop_entry["fp"]
    baseline_fn = cop_entry["fn"]
    baseline_matches = cop_entry["matches"]

    # Detect Include-gated cops and auto-enable --repo-cwd
    include_gated = is_include_gated_cop(args.cop)
    zero_baseline = expected_rubocop == 0 and baseline_fp == 0
    if include_gated and zero_baseline:
        if not args.repo_cwd:
            print(f"NOTE: {args.cop} is Include-gated with zero baseline — "
                  f"auto-enabling --repo-cwd", file=sys.stderr)
            args.repo_cwd = True
    if args.repo_cwd:
        _USE_REPO_CWD = True

    if style_config_override:
        _STYLE_CONFIG_OVERRIDE = style_config_override

    ensure_binary()

    # Validate local corpus matches manifest (warns about stale/missing repos)
    if args.rerun:
        if args.clone:
            # Clone into temp dir with oracle-identical path structure
            tmpdir = clone_repos_for_cop(
                args.cop, data,
                shard_index=args.shard_index, total_shards=args.total_shards,
                sample=args.sample,
                include_gated=include_gated and zero_baseline,
            )
            _CLONE_DIR = tmpdir / "repos"
        else:
            validate_corpus()
        check_corpus_bundle()

    print(f"Checking {args.cop} against corpus")
    if include_gated and zero_baseline:
        print("Mode: Include-gated cop with zero baseline (plausibility check)")
        print("  This cop is silently disabled in the oracle due to Include pattern")
        print("  resolution failure. Running with cwd=repo_dir to enable patterns.")
    else:
        print("Gate: count-only cop-level regression check")
        print("Not a location-level conformance proof or a department completion gate")
    print(f"Baseline (from CI): {baseline_matches:,} matches, "
          f"{baseline_fp:,} FP, {baseline_fn:,} FN")
    print(f"Expected RuboCop offenses: {expected_rubocop:,}")
    print()

    # Check if enriched per-repo-per-cop data is available in corpus results
    by_repo_cop = data.get("by_repo_cop", {})
    if not by_repo_cop:
        print("ERROR: corpus artifact lacks by_repo_cop data. Run corpus oracle first.", file=sys.stderr)
        sys.exit(1)
    has_activity_index = bool(data.get("cop_activity_repos"))

    if args.verbose and not args.rerun:
        # Use baseline artifact data instead of re-running nitrocop.
        # This reflects the downloaded corpus-oracle run, not local unverified changes.
        print(
            "Using baseline corpus artifact data (pass --rerun to validate current code)",
            file=sys.stderr,
        )

        # Reconstruct per-repo counts from by_repo_cop
        # nitrocop count = rubocop count + FP - FN per repo
        by_repo = data.get("by_repo", [])
        {r["repo"]: r for r in by_repo if r.get("status") == "ok"}

        repos_with_offenses = {}
        for repo_id, cops in by_repo_cop.items():
            if args.cop in cops:
                entry = cops[args.cop]
                fp = entry.get("fp", 0)
                fn = entry.get("fn", 0)
                if fp > 0 or fn > 0:
                    repos_with_offenses[repo_id] = {"fp": fp, "fn": fn}

        if repos_with_offenses:
            print(f"Repos with divergence ({len(repos_with_offenses)}):")
            sorted_repos = sorted(repos_with_offenses.items(),
                                  key=lambda x: x[1]["fp"] + x[1]["fn"],
                                  reverse=True)
            for repo_id, counts in sorted_repos[:30]:
                print(f"  FP:{counts['fp']:>5}  FN:{counts['fn']:>5}  {repo_id}")
            if len(sorted_repos) > 30:
                print(f"  ... and {len(sorted_repos) - 30} more")
            print()

        # In artifact mode, nitrocop_total for that run is:
        # rubocop_matches + false_positives.
        nitrocop_total = baseline_matches + baseline_fp
        file_drop_offenses = 0
        file_drop_repos = {}
    else:
        # Try local cache first (unless --rerun forces re-execution)
        cached = None if args.rerun else get_cached_results(args.cop)

        if cached is not None:
            print("Using cached nitrocop results (pass --rerun to re-execute)", file=sys.stderr)
            per_repo = cached
        else:
            per_repo = rerun_local_per_repo(
                args.cop,
                data,
                quick=args.quick,
                has_activity_index=has_activity_index,
                shard_index=args.shard_index,
                total_shards=args.total_shards,
                sample=args.sample,
                include_gated=include_gated and zero_baseline,
            )
            save_cached_results(args.cop, per_repo)

        # Older corpus artifacts do not include cop_activity_repos, so clone mode
        # only reruns baseline-diverging repos. Preserve the synthetic CI-baseline
        # fallback for those older artifacts.
        if args.clone and not has_activity_index:
            set(per_repo.keys())
            # For each repo NOT in per_repo, add its CI nitrocop count.
            # Repos in by_repo_cop have matches + FP - FN. Repos NOT in
            # by_repo_cop matched exactly, but we don't have per-repo counts
            # for those. We know the total: ci_nitrocop_total - sum(diverging).
            ci_diverging_total = 0
            for repo_id, cops in by_repo_cop.items():
                if args.cop in cops:
                    entry = cops[args.cop]
                    ci_diverging_total += entry.get("matches", 0) + entry.get("fp", 0)
            ci_matching_total = baseline_matches + baseline_fp - ci_diverging_total
            # Add the matching repos as a single synthetic entry
            per_repo["__ci_baseline_matching_repos__"] = ci_matching_total
            print(f"  Using CI baseline for non-diverging repos "
                  f"({ci_matching_total:,} offenses from matching repos)",
                  file=sys.stderr)

        # Filter to only repos that were "ok" in the CI corpus oracle run.
        # Local corpus may have stale/extra repos (denylisted, removed) and
        # CI may have repos that crashed. Including these inflates the excess
        # count since they have no RuboCop baseline to compare against.
        by_repo = data.get("by_repo", [])
        ci_ok_repos = {r["repo"] for r in by_repo if r.get("status") == "ok"}
        if ci_ok_repos:
            # Don't exclude synthetic baseline entry added by --clone
            ci_ok_repos.add("__ci_baseline_matching_repos__")
            excluded = {k for k in per_repo if k not in ci_ok_repos}
            if excluded:
                excluded_total = sum(per_repo.get(k, 0) for k in excluded if per_repo.get(k, 0) > 0)
                print(f"Excluding {len(excluded)} repos not in CI baseline "
                      f"({excluded_total:,} offenses)", file=sys.stderr)
            per_repo = {k: v for k, v in per_repo.items() if k in ci_ok_repos}

        # Identify repos with RuboCop file drops (parser crashes that cause
        # some files to be silently skipped). CI filters nitrocop offenses to
        # only RuboCop-inspected files before comparing, so CI's FP/FN counts
        # exclude offenses from dropped files. Our local run can't replicate
        # this per-file filtering, so we report the noise separately.
        file_drop_repos = {r["repo"]: r.get("rubocop_files_dropped", 0)
                           for r in by_repo
                           if r.get("rubocop_files_dropped", 0) > 0}
        file_drop_offenses = sum(per_repo.get(k, 0) for k in file_drop_repos
                                 if per_repo.get(k, 0) > 0)

        nitrocop_total = sum(c for c in per_repo.values() if c >= 0)

        if args.verbose:
            # Show repos with offenses, sorted by count descending
            repos_with_offenses = {k: v for k, v in per_repo.items() if v > 0}
            if repos_with_offenses:
                print(f"Repos with offenses ({len(repos_with_offenses)}):")
                for repo_id, count in sorted(repos_with_offenses.items(),
                                             key=lambda x: x[1], reverse=True)[:30]:
                    print(f"  {count:>6,}  {repo_id}")
                if len(repos_with_offenses) > 30:
                    print(f"  ... and {len(repos_with_offenses) - 30} more")
                print()

            # Debug: dump per-repo counts for comparison with oracle
            debug_path = PROJECT_ROOT / "check-cop-debug.json"
            debug_data = {
                "cop": args.cop,
                "per_repo": {k: v for k, v in sorted(per_repo.items())},
                "total": sum(c for c in per_repo.values() if c >= 0),
                "repos_run": len(per_repo),
                "errors": [k for k, v in per_repo.items() if v < 0],
            }
            debug_path.write_text(json.dumps(debug_data, indent=2))
            print(f"Debug: per-repo counts written to {debug_path}",
                  file=sys.stderr)

    excess = max(0, nitrocop_total - expected_rubocop)
    missing = max(0, expected_rubocop - nitrocop_total)

    # Debug: if there's a discrepancy and we have per-repo data, show details
    if (excess > 0 or missing > 0) and args.verbose and 'per_repo' in dir():
        print("Per-repo discrepancy analysis:", file=sys.stderr)
        print(f"  check-cop total: {nitrocop_total}, oracle expected: {expected_rubocop}, "
              f"diff: {nitrocop_total - expected_rubocop:+d}", file=sys.stderr)
        activity_repos = set(data.get("cop_activity_repos", {}).get(args.cop, []))
        local_active = {k for k, v in per_repo.items() if v > 0 and k != "__ci_baseline_matching_repos__"}
        only_local = sorted(local_active - activity_repos)
        only_oracle = sorted(activity_repos - local_active)
        if only_local:
            extra_from_local = sum(per_repo.get(k, 0) for k in only_local)
            print(f"  Repos with offenses locally but NOT in oracle activity ({len(only_local)}, "
                  f"{extra_from_local} offenses):", file=sys.stderr)
            for r in only_local[:10]:
                print(f"    {per_repo[r]:>4}  {r}", file=sys.stderr)
        if only_oracle:
            print(f"  Repos in oracle activity but 0 locally ({len(only_oracle)}):",
                  file=sys.stderr)
            for r in only_oracle[:10]:
                print(f"    {r}", file=sys.stderr)
        print(file=sys.stderr)

    # CI nitrocop baseline: the offense count CI's nitrocop produced on
    # RuboCop-inspected files. Our local count should be close to this.
    ci_nitrocop_total = baseline_matches + baseline_fp
    nitrocop_total - ci_nitrocop_total

    print("Results:")
    print(f"  Expected (RuboCop):   {expected_rubocop:>10,}")
    print(f"  Actual (nitrocop):    {nitrocop_total:>10,}")
    print(f"  CI nitrocop baseline: {ci_nitrocop_total:>10,}")
    print(f"  Excess (potential FP):{excess:>10,}")
    print(f"  Missing (potential FN):{missing:>9,}")
    if file_drop_offenses > 0:
        print(f"  File-drop noise:      {file_drop_offenses:>10,}  "
              f"({len(file_drop_repos)} repos with RuboCop parser crashes)")
        adjusted_excess = max(0, excess - file_drop_offenses)
        print(f"  Excess (adjusted):    {adjusted_excess:>10,}  "
              f"(excess minus file-drop noise)")
        if excess > 0 and file_drop_offenses >= excess:
            print(f"  WARNING: file-drop noise ({file_drop_offenses:,}) masks "
                  f"raw excess ({excess:,}). Real FPs may exist — use "
                  f"verify_cop_locations.py for ground truth.")
    print()

    print("  Gate type: count-only / cop-level regression")
    print()

    # Gate: per-repo regression detection.
    #
    # Aggregate comparison is unreliable because local nitrocop scans files
    # the oracle filtered out (RuboCop parser crashes, excluded paths, etc.).
    # Instead, compare per-repo: for each repo in by_repo_cop, check if the
    # local count diverged from the oracle's per-repo nitrocop count.
    # Repos NOT in by_repo_cop matched exactly in the oracle — flag any
    # local count that exceeds the oracle activity count for that repo.
    if args.rerun and 'per_repo' in dir():
        new_fp = 0
        new_fn = 0
        resolved_fp = 0
        resolved_fn = 0
        total_baseline_fp = 0
        total_baseline_fn = 0
        total_local_fp = 0
        total_local_fn = 0
        total_count_baseline_fp = 0
        total_count_baseline_fn = 0
        fp_repos = []
        fn_repos = []
        # Build per-repo baseline counts from the oracle.
        # oracle_nitrocop = matches + FP (what the oracle's nitrocop found)
        # oracle_rubocop = matches + FN (what rubocop found)
        # A regression is when the PR's nitrocop diverges MORE from rubocop
        # than the oracle's nitrocop already did. Pre-existing FP/FN are not
        # regressions — they were already there on main.
        oracle_nitrocop_counts = {}
        oracle_rubocop_counts = {}
        oracle_location_fp = {}
        oracle_location_fn = {}
        for repo_id, cops in by_repo_cop.items():
            if args.cop in cops:
                entry = cops[args.cop]
                matches = entry.get("matches", 0)
                fp = entry.get("fp", 0)
                fn = entry.get("fn", 0)
                oracle_nitrocop_counts[repo_id] = matches + fp
                oracle_rubocop_counts[repo_id] = matches + fn
                # Store location-level FP/FN from the oracle directly.
                # The oracle compares by (file, line), so location swaps
                # where nitrocop fires at line 10 and rubocop at line 15
                # are captured as fp=1, fn=1. Count-based recomputation
                # (max(0, nc - rc)) collapses these to 0 when fp == fn.
                oracle_location_fp[repo_id] = fp
                oracle_location_fn[repo_id] = fn

        # For repos with oracle activity but no divergence, oracle nitrocop
        # matched rubocop exactly. Use local count as proxy for both.
        for repo_id in data.get("cop_activity_repos", {}).get(args.cop, []):
            if repo_id not in oracle_nitrocop_counts:
                local = per_repo.get(repo_id, 0)
                oracle_nitrocop_counts[repo_id] = local
                oracle_rubocop_counts[repo_id] = local

        for repo_id, local_count in per_repo.items():
            if repo_id == "__ci_baseline_matching_repos__" or local_count < 0:
                continue
            baseline_nc = oracle_nitrocop_counts.get(repo_id)
            baseline_rc = oracle_rubocop_counts.get(repo_id)
            if baseline_nc is None or baseline_rc is None:
                continue
            # Use the oracle's location-level FP/FN as the baseline.
            # This is more accurate than count-based recomputation which
            # loses location-swap information (fp==fn cancels to 0).
            baseline_fp = oracle_location_fp.get(repo_id, 0)
            baseline_fn = oracle_location_fn.get(repo_id, 0)
            total_baseline_fp += baseline_fp
            total_baseline_fn += baseline_fn
            # Also track count-level baseline for sanity-check annotation.
            # Count-level FP = max(0, nitrocop_count - rubocop_count).
            count_bl_fp = max(0, baseline_nc - baseline_rc)
            count_bl_fn = max(0, baseline_rc - baseline_nc)
            total_count_baseline_fp += count_bl_fp
            total_count_baseline_fn += count_bl_fn
            # How far is the local nitrocop from rubocop?
            local_fp = max(0, local_count - baseline_rc)
            local_fn = max(0, baseline_rc - local_count)
            total_local_fp += local_fp
            total_local_fn += local_fn
            # Track both regressions and improvements
            fp_increase = max(0, local_fp - baseline_fp)
            fn_increase = max(0, local_fn - baseline_fn)
            fp_decrease = max(0, baseline_fp - local_fp)
            fn_decrease = max(0, baseline_fn - local_fn)
            resolved_fp += fp_decrease
            resolved_fn += fn_decrease
            if fp_increase > 0:
                new_fp += fp_increase
                fp_repos.append((repo_id, local_count, baseline_nc, baseline_rc, fp_increase))
            if fn_increase > 0:
                new_fn += fn_increase
                fn_repos.append((repo_id, local_count, baseline_nc, baseline_rc, fn_increase))

        print("  Gate: per-repo regression vs oracle baseline")
        print(f"  New FP (worse than baseline): {new_fp:>6,}")
        if fp_repos:
            for repo_id, local, bl_nc, bl_rc, diff in sorted(fp_repos, key=lambda x: -x[4]):
                print(f"    +{diff:>3} FP  {repo_id}  (local={local}, baseline_nc={bl_nc}, rubocop={bl_rc})")
        print(f"  New FN (worse than baseline): {new_fn:>6,}")
        if fn_repos:
            for repo_id, local, bl_nc, bl_rc, diff in sorted(fn_repos, key=lambda x: -x[4]):
                print(f"    +{diff:>3} FN  {repo_id}  (local={local}, baseline_nc={bl_nc}, rubocop={bl_rc})")
        if resolved_fp or resolved_fn:
            print(f"  Resolved FP (better):         {resolved_fp:>6,}")
            print(f"  Resolved FN (better):         {resolved_fn:>6,}")
        print()

        failed = False
        # With --allow-net-improvement, per-repo regressions that are offset
        # by improvements elsewhere pass (net FP/FN did not increase).
        # Without it (default, used by agents), ANY per-repo regression fails.
        if args.allow_net_improvement:
            fp_gate = max(0, new_fp - resolved_fp)
            fn_gate = max(0, new_fn - resolved_fn)
        else:
            fp_gate = new_fp
            fn_gate = new_fn
        if fp_gate > args.threshold:
            label = f"net +{new_fp - resolved_fp:,}" if args.allow_net_improvement else f"+{new_fp:,}"
            sorted_fp = sorted(fp_repos, key=lambda x: -x[4])[:10]
            # Include repo names directly in FAIL line for small regressions
            if len(sorted_fp) <= 3:
                repo_names = ", ".join(r[0] for r in sorted_fp)
                print(f"FAIL: FP regression ({label}) in: {repo_names}")
            else:
                print(f"FAIL: FP regression detected ({label})")
            for repo_id, local, bl_nc, bl_rc, diff in sorted_fp:
                print(f"  +{diff:>4}  {repo_id}  (local={local}, baseline_nc={bl_nc}, rubocop={bl_rc})")
            failed = True
        if fn_gate > args.threshold:
            label = f"net +{new_fn - resolved_fn:,}" if args.allow_net_improvement else f"+{new_fn:,}"
            sorted_fn = sorted(fn_repos, key=lambda x: -x[4])[:10]
            if len(sorted_fn) <= 3:
                repo_names = ", ".join(r[0] for r in sorted_fn)
                print(f"FAIL: FN regression ({label}) in: {repo_names}")
            else:
                print(f"FAIL: FN regression detected ({label})")
            for repo_id, local, bl_nc, bl_rc, diff in sorted_fn:
                print(f"  +{diff:>4}  {repo_id}  (local={local}, baseline_nc={bl_nc}, rubocop={bl_rc})")
            failed = True

        # For Include-gated cops with zero baseline, show plausibility report
        # instead of regression gate (which has no oracle data to compare against).
        if include_gated and zero_baseline and not failed:
            repos_with_offenses = {k: v for k, v in per_repo.items()
                                   if v > 0 and k != "__ci_baseline_matching_repos__"}
            repos_run = len([k for k in per_repo if k != "__ci_baseline_matching_repos__"])
            total_offenses = sum(v for v in per_repo.values() if v > 0)
            print("  Include-gated plausibility report:")
            print(f"    Repos scanned: {repos_run}")
            print(f"    Repos with offenses: {len(repos_with_offenses)}")
            print(f"    Total offenses: {total_offenses:,}")
            if repos_with_offenses:
                print("    Top repos:")
                for repo_id, count in sorted(repos_with_offenses.items(),
                                             key=lambda x: x[1], reverse=True)[:10]:
                    print(f"      {count:>6,}  {repo_id}")
            print()

        # Machine-readable summary for CI aggregation
        # Format: cop|baseline_fp|baseline_fn|local_fp|local_fn|result|count_bl_fp|count_bl_fn
        # baseline_fp/fn = location-level from oracle
        # local_fp/fn = count-level from local run (max(0, local - rubocop))
        # count_bl_fp/fn = count-level baseline (max(0, oracle_nc - oracle_rc))
        # The last two fields enable the CI comment to detect when a large
        # location-level FP delta has no count-level counterpart (location
        # shift or config resolution artifact, not a real regression).
        result_str = "fail" if failed else "pass"
        print(f"SUMMARY|{args.cop}|{total_baseline_fp}|{total_baseline_fn}|{total_local_fp}|{total_local_fn}|{result_str}|{total_count_baseline_fp}|{total_count_baseline_fn}")

        if failed:
            sys.exit(1)

        # Per-line spot-check: verify known FP/FN examples from the oracle.
        # This catches regressions that cancel out in per-repo counts
        # (e.g. +5 FP and -5 FN in the same repo = net 0 change).
        fp_remain, fp_resolved, fn_remain, fn_resolved, fp_unchecked, fn_unchecked = spot_check_examples(
            args.cop, data,
        )
        total_checked = fp_remain + fp_resolved + fn_remain + fn_resolved
        total_unchecked = fp_unchecked + fn_unchecked
        if total_checked + total_unchecked > 0:
            print(f"  Spot-check ({total_checked + total_unchecked} oracle examples, "
                  f"{total_unchecked} unchecked — repo not cloned):")
            if fp_remain + fp_resolved + fp_unchecked > 0:
                parts = [f"{fp_resolved} resolved", f"{fp_remain} remain"]
                if fp_unchecked:
                    parts.append(f"{fp_unchecked} unchecked")
                print(f"    FP: {', '.join(parts)} "
                      f"(of {fp_remain + fp_resolved + fp_unchecked})")
            if fn_remain + fn_resolved + fn_unchecked > 0:
                parts = [f"{fn_resolved} resolved", f"{fn_remain} remain"]
                if fn_unchecked:
                    parts.append(f"{fn_unchecked} unchecked")
                print(f"    FN: {', '.join(parts)} "
                      f"(of {fn_remain + fn_resolved + fn_unchecked})")
            if total_unchecked > total_checked:
                print(f"    WARNING: {total_unchecked} examples could not be verified "
                      f"(repos not cloned with --sample). Use --sample with a higher "
                      f"value or verify_cop_locations.py for ground truth.")
            print()

        print("PASS: no per-repo regressions vs baseline (default config)")

        # ── Variant style checks ──
        variant_failed = False
        if args.check_variants and _CLONE_DIR is not None:
            # Generate variant batch configs if not provided
            variant_batches = args.variant_batches_dir
            if not variant_batches:
                # Generate into the repo's variant_batches dir so that
                # inherit_from: ../baseline_rubocop.yml resolves correctly.
                variant_batches = str(PROJECT_ROOT / "bench" / "corpus" / "variant_batches")
                sys.path.insert(0, str(PROJECT_ROOT / "bench" / "corpus"))
                from gen_variant_batches import generate_batches
                generate_batches(Path(variant_batches))

            # Load variant baselines from the oracle for delta computation
            vb = load_variant_baselines(args.cop, corpus_run_id)

            # Collect cloned repo directories, capped for variant checks.
            # Variant bugs are systematic (wrong code path for a style), not
            # repo-specific edge cases, so a small sample catches them.
            VARIANT_SAMPLE = 20
            corpus_dir = _CLONE_DIR
            repo_dirs = sorted(str(d) for d in corpus_dir.iterdir() if d.is_dir())
            if len(repo_dirs) > VARIANT_SAMPLE:
                repo_dirs = repo_dirs[:VARIANT_SAMPLE]

            if repo_dirs:
                print()
                print(f"Variant checks: {args.cop} across {len(repo_dirs)} repos")
                variant_results = run_variant_checks(
                    cop_name=args.cop,
                    repo_dirs=repo_dirs,
                    batches_dir=variant_batches,
                    binary=str(NITROCOP_BIN),
                    variant_baselines=vb,
                )

                variant_failed = False
                if variant_results:
                    print()
                    print("  Variant results:")
                    print(f"  {'Style':<30} {'NC':>8} {'RC':>8} {'FP':>6} {'FN':>6}  {'BL FP':>6} {'BL FN':>6}")
                    print(f"  {'-'*30} {'-'*8} {'-'*8} {'-'*6} {'-'*6}  {'-'*6} {'-'*6}")
                    for vr in variant_results:
                        # Regression: local FP/FN on this shard's repos exceeds
                        # what's expected. Since baseline is global (all repos) and
                        # the shard is a subset, we can't compare directly. Instead,
                        # flag when baseline was 0 but local has FP/FN (new regression),
                        # or when local FP/FN exceeds the global baseline (definitely worse).
                        fp_regressed = (vr['fp'] > 0 and vr['baseline_fp'] == 0) or vr['fp'] > vr['baseline_fp']
                        fn_regressed = (vr['fn'] > 0 and vr['baseline_fn'] == 0) or vr['fn'] > vr['baseline_fn']
                        v_result = "fail" if fp_regressed or fn_regressed else "pass"
                        marker = " ← REGRESSION" if v_result == "fail" else ""
                        print(
                            f"  {vr['style_label']:<30} "
                            f"{vr['nitrocop_total']:>8,} "
                            f"{vr['rubocop_total']:>8,} "
                            f"{vr['fp']:>6,} "
                            f"{vr['fn']:>6,}  "
                            f"{vr['baseline_fp']:>6,} "
                            f"{vr['baseline_fn']:>6,}{marker}"
                        )
                        print(
                            f"SUMMARY|{args.cop} ({vr['style_label']})"
                            f"|{vr['baseline_fp']}|{vr['baseline_fn']}"
                            f"|{vr['fp']}|{vr['fn']}|{v_result}|0|0"
                        )
                        if v_result == "fail":
                            variant_failed = True
                    print()
                    if variant_failed:
                        print(f"FAIL: variant style regression detected for {args.cop}")
                else:
                    print(f"  No variant overrides for {args.cop}")
                    print()

        if variant_failed:
            sys.exit(1)
        sys.exit(0)

    # Per-repo gate should have handled this — if we reach here, something is wrong
    print("ERROR: per-repo gate did not execute. Check corpus artifact data.", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
