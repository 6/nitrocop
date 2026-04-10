#!/usr/bin/env python3
"""Clone corpus repos into a directory matching the oracle's structure.

Shared by check_cop.py and corpus-oracle.yml to ensure identical file trees.
Clones into <dest>/repos/REPO_ID/ with shallow depth-1 fetches at pinned SHAs.

CLI usage:
    python3 bench/corpus/clone_repos.py --dest /tmp/run --manifest bench/corpus/manifest.jsonl
    python3 bench/corpus/clone_repos.py --dest . --repo-ids "id1,id2,id3" --parallel 3

Library usage:
    from bench.corpus.clone_repos import clone_repos, load_manifest
    manifest = load_manifest(Path("bench/corpus/manifest.jsonl"))
    clone_repos(Path("/tmp/run"), manifest, repo_ids={"id1", "id2"})
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


def load_manifest(manifest_path: Path) -> dict[str, dict]:
    """Load repo info from manifest.jsonl, keyed by repo ID."""
    repos = {}
    if not manifest_path.exists():
        return repos
    with open(manifest_path) as f:
        import json
        for line in f:
            line = line.strip()
            if line:
                entry = json.loads(line)
                repos[entry["id"]] = entry
    return repos


def repo_head_sha(repo_dir: Path) -> str | None:
    """Get the HEAD SHA of a cloned repo, or None if not a git repo."""
    try:
        result = subprocess.run(
            ["git", "-C", str(repo_dir), "rev-parse", "HEAD"],
            capture_output=True, text=True, timeout=5,
        )
        return result.stdout.strip() if result.returncode == 0 else None
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return None


def clone_repos(
    dest: Path,
    manifest: dict[str, dict],
    repo_ids: set[str] | None = None,
    parallel: int = 3,
) -> int:
    """Clone repos into dest/repos/REPO_ID/. Returns count of newly cloned repos.

    Skips repos already cloned at the correct SHA.
    """
    import shutil

    repos_dir = dest / "repos"
    repos_dir.mkdir(parents=True, exist_ok=True)

    ids_to_clone = repo_ids if repo_ids is not None else set(manifest.keys())
    to_clone = []
    for repo_id in sorted(ids_to_clone):
        if repo_id not in manifest:
            continue
        repo_dir = repos_dir / repo_id
        if repo_dir.exists():
            if repo_head_sha(repo_dir) == manifest[repo_id].get("sha"):
                continue
            shutil.rmtree(repo_dir, ignore_errors=True)
        to_clone.append(manifest[repo_id])

    if not to_clone:
        print(f"  All {len(ids_to_clone)} repos already cloned", file=sys.stderr)
        return 0

    print(f"  Cloning {len(to_clone)} repos...", file=sys.stderr)

    def _clone_one(repo_info: dict) -> tuple[bool, str]:
        repo_id = repo_info["id"]
        repo_dir = repos_dir / repo_id
        try:
            repo_dir.mkdir(parents=True, exist_ok=True)
            subprocess.run(["git", "init", str(repo_dir)],
                           capture_output=True, check=True, timeout=10)
            subprocess.run(["git", "-C", str(repo_dir), "fetch", "--depth", "1",
                            repo_info["repo_url"], repo_info["sha"]],
                           capture_output=True, check=True, timeout=120)
            subprocess.run(["git", "-C", str(repo_dir), "checkout", "FETCH_HEAD"],
                           capture_output=True, check=True, timeout=30)
            return True, repo_id
        except subprocess.TimeoutExpired:
            shutil.rmtree(repo_dir, ignore_errors=True)
            return False, f"{repo_id} (timeout)"
        except subprocess.CalledProcessError:
            shutil.rmtree(repo_dir, ignore_errors=True)
            return False, f"{repo_id} (clone failed — repo may be deleted or private)"

    ok = 0
    failed: list[str] = []
    with ThreadPoolExecutor(max_workers=parallel) as pool:
        futures = {pool.submit(_clone_one, r): r["id"] for r in to_clone}
        for f in as_completed(futures):
            success, detail = f.result()
            if success:
                ok += 1
            else:
                failed.append(detail)
    print(f"  Cloned {ok}/{len(to_clone)} repos", file=sys.stderr)
    for msg in failed:
        print(f"  ::warning::Clone failed: {msg}", file=sys.stderr)
    return ok


def main():
    parser = argparse.ArgumentParser(description="Clone corpus repos for oracle/check-cop")
    parser.add_argument("--dest", type=Path, required=True, help="Clone into <dest>/repos/REPO_ID/")
    parser.add_argument("--manifest", type=Path, default=Path("bench/corpus/manifest.jsonl"))
    parser.add_argument("--repo-ids", help="Comma-separated repo IDs (default: all)")
    parser.add_argument("--parallel", type=int, default=3, help="Concurrent clones (default: 3)")
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    if not manifest:
        print("ERROR: empty or missing manifest", file=sys.stderr)
        sys.exit(1)

    repo_ids = set(args.repo_ids.split(",")) if args.repo_ids else None
    clone_repos(args.dest, manifest, repo_ids=repo_ids, parallel=args.parallel)


if __name__ == "__main__":
    main()
