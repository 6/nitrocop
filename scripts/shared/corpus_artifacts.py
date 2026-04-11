#!/usr/bin/env python3
"""Shared helper to download corpus-results.json from CI.

Tries multiple strategies in order:
1. `gh` CLI (if installed and authenticated)
2. GitHub REST API with GH_TOKEN / GH_TOKEN_FOR_ACTIONS_READ / GITHUB_TOKEN env var
3. Parse checked-in docs/corpus.md (summary data only, no per-file detail)
4. Helpful error message with instructions

All scripts that need corpus-results.json should use this module instead of
duplicating the download logic.

Usage:
    from shared.corpus_artifacts import download_corpus_results

    # Returns (path, run_id, head_sha)
    path, run_id, head_sha = download_corpus_results()
"""

from __future__ import annotations

import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


def _find_project_root() -> Path:
    """Find the git repo root."""
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True, text=True,
    )
    if result.returncode == 0:
        return Path(result.stdout.strip())
    return Path(__file__).resolve().parents[2]


def _detect_github_repo() -> str | None:
    """Detect the GitHub owner/repo from git remote origin URL.

    Supports formats:
        https://github.com/owner/repo.git
        git@github.com:owner/repo.git
        http://proxy@host:port/git/owner/repo
    """
    result = subprocess.run(
        ["git", "remote", "get-url", "origin"],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        return None

    url = result.stdout.strip()

    # http(s)://...github.com/owner/repo or proxy format /git/owner/repo
    import re
    # Standard GitHub URL
    m = re.search(r"github\.com[/:]([^/]+/[^/.]+?)(?:\.git)?$", url)
    if m:
        return m.group(1)

    # Local proxy format: http://proxy@host:port/git/owner/repo
    m = re.search(r"/git/([^/]+/[^/]+?)(?:\.git)?$", url)
    if m:
        return m.group(1)

    return None


def _clean_stale_local(project_root: Path) -> None:
    """Remove stale corpus-results.json from the project root."""
    stale = project_root / "corpus-results.json"
    if stale.exists():
        stale.unlink()
        print(f"Removed stale {stale.name} from project root", file=sys.stderr)


def _cache_dir() -> Path:
    """Get the cache directory for downloaded corpus results."""
    d = Path(tempfile.gettempdir()) / "nitrocop-corpus-cache"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _try_gh(repo: str | None) -> tuple[Path, int, str] | None:
    """Try downloading via gh CLI. Returns (path, run_id, head_sha) or None.

    Checks recent successful corpus oracle runs and downloads the
    "corpus-report" artifact.
    """
    if not shutil.which("gh"):
        print("corpus-dl: gh CLI not found", file=sys.stderr)
        return None

    has_gh_token = bool(os.environ.get("GH_TOKEN"))

    # Check if gh has auth available (stored credentials or GH_TOKEN env var)
    auth_check = subprocess.run(
        ["gh", "auth", "status"],
        capture_output=True, text=True,
    )
    if auth_check.returncode != 0 and not has_gh_token:
        print("corpus-dl: gh not authenticated and no GH_TOKEN", file=sys.stderr)
        return None

    # Build the command with optional repo flag
    repo_args = ["-R", repo] if repo else []

    # List recent successful runs
    result = subprocess.run(
        ["gh", "run", "list", *repo_args, "--workflow=corpus-oracle.yml",
         "--status=success", "--limit=100", "--json=databaseId,headSha"],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        print(f"corpus-dl: gh run list failed: {result.stderr.strip()}", file=sys.stderr)
        return None

    runs = json.loads(result.stdout)
    if not runs:
        print("corpus-dl: no successful corpus-oracle runs found", file=sys.stderr)
        return None

    for run in runs:
        run_id = run["databaseId"]
        head_sha = run.get("headSha", "")

        # Check cache
        cache_path = _cache_dir() / f"corpus-results-{run_id}.json"
        if cache_path.exists():
            print(f"Using cached corpus-results from run {run_id}", file=sys.stderr)
            return cache_path, run_id, head_sha

        # Try downloading the artifact
        tmpdir = tempfile.mkdtemp(prefix="corpus-dl-")
        dl_result = subprocess.run(
            ["gh", "run", "download", *repo_args, str(run_id),
             "--name=corpus-report", f"--dir={tmpdir}"],
            capture_output=True, text=True,
        )
        if dl_result.returncode != 0:
            print(
                f"corpus-dl: gh run download {run_id} failed: "
                f"{dl_result.stderr.strip()}",
                file=sys.stderr,
            )
            shutil.rmtree(tmpdir, ignore_errors=True)
            continue

        path = Path(tmpdir) / "corpus-results.json"
        if not path.exists():
            shutil.rmtree(tmpdir, ignore_errors=True)
            continue

        # Found it — cache and return
        cache_path = _cache_dir() / f"corpus-results-{run_id}.json"
        shutil.copy2(path, cache_path)
        _cache_synthetic_from_dir(Path(tmpdir), run_id)
        _cache_variant_results_from_dir(Path(tmpdir), run_id)

        print(f"Downloaded corpus-report from run {run_id} via gh", file=sys.stderr)
        return cache_path, run_id, head_sha

    return None


def _github_api_get(url: str, token: str | None = None) -> dict:
    """Make a GET request to the GitHub API."""
    headers = {"Accept": "application/vnd.github+json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = Request(url, headers=headers)
    with urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def _github_api_download(url: str, token: str) -> bytes:
    """Download binary content from the GitHub API (follows redirects).

    GitHub artifact downloads return a 302 redirect to Azure blob storage.
    Python's urlopen forwards the Authorization header to the redirect target,
    which Azure rejects with 401. We handle the redirect manually, stripping
    the auth header for the cross-domain follow-up request.
    """
    import urllib.request

    class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            # Don't auto-follow; we'll handle it ourselves
            raise HTTPError(newurl, code, msg, headers, fp)

    opener = urllib.request.build_opener(NoRedirectHandler)
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
    }
    req = Request(url, headers=headers)
    try:
        with opener.open(req, timeout=30) as resp:
            return resp.read()
    except HTTPError as e:
        if e.code in (301, 302, 303, 307, 308):
            # Follow the redirect WITHOUT the Authorization header
            redirect_url = e.headers.get("Location") or str(e.url)
            req2 = Request(redirect_url)
            with urlopen(req2, timeout=120) as resp:
                return resp.read()
        raise


def _try_curl_api(repo: str | None) -> tuple[Path, int, str] | None:
    """Try downloading via GitHub REST API with GH_TOKEN env var."""
    token = (os.environ.get("GH_TOKEN_FOR_ACTIONS_READ")
             or os.environ.get("GH_TOKEN")
             or os.environ.get("GITHUB_TOKEN"))
    if not token:
        # Fall back to gh CLI auth token if available
        try:
            result = subprocess.run(
                ["gh", "auth", "token"], capture_output=True, text=True, timeout=5
            )
            if result.stdout.strip():
                token = result.stdout.strip()
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass
    if not repo:
        return None

    api_base = f"https://api.github.com/repos/{repo}"

    # List recent successful runs
    try:
        data = _github_api_get(
            f"{api_base}/actions/workflows/corpus-oracle.yml/runs?status=success&per_page=100",
            token,
        )
    except (HTTPError, URLError) as e:
        print(f"GitHub API error listing runs: {e}", file=sys.stderr)
        return None

    runs = data.get("workflow_runs", [])
    if not runs:
        return None

    for run in runs:
        run_id = run["id"]
        head_sha = run.get("head_sha", "")

        # Check cache
        cache_path = _cache_dir() / f"corpus-results-{run_id}.json"
        if cache_path.exists():
            print(f"Using cached corpus-results from run {run_id}", file=sys.stderr)
            return cache_path, run_id, head_sha

        if not token:
            print("Found corpus oracle run but need a token to download artifacts.", file=sys.stderr)
            print("Set GH_TOKEN, GH_TOKEN_FOR_ACTIONS_READ, or GITHUB_TOKEN env var, or run: gh auth login", file=sys.stderr)
            return None

        # List artifacts for this run
        try:
            artifacts_data = _github_api_get(
                f"{api_base}/actions/runs/{run_id}/artifacts", token
            )
        except (HTTPError, URLError) as e:
            print(f"GitHub API error listing artifacts: {e}", file=sys.stderr)
            continue

        # Build name -> id map
        artifact_map = {a["name"]: a["id"] for a in artifacts_data.get("artifacts", [])}

        artifact_id = artifact_map.get("corpus-report")
        if not artifact_id:
            continue

        print(f"Downloading corpus-report from run {run_id} via API...", file=sys.stderr)
        try:
            zip_bytes = _github_api_download(
                f"{api_base}/actions/artifacts/{artifact_id}/zip", token
            )
        except (HTTPError, URLError) as e:
            print(f"GitHub API error downloading artifact: {e}", file=sys.stderr)
            continue

        try:
            with zipfile.ZipFile(io.BytesIO(zip_bytes)) as zf:
                if "corpus-results.json" not in zf.namelist():
                    continue
                cache_path = _cache_dir() / f"corpus-results-{run_id}.json"
                with zf.open("corpus-results.json") as f:
                    cache_path.write_bytes(f.read())
                _cache_synthetic_from_zip(zf, run_id)
                _cache_variant_results_from_zip(zf, run_id)
        except zipfile.BadZipFile:
            continue

        return cache_path, run_id, head_sha

    return None


def _try_corpus_md() -> tuple[Path, int, str] | None:
    """Parse checked-in corpus markdown to build a minimal corpus-results.json.

    This is a last-resort fallback when no GitHub token is available.
    It provides summary and by_cop data (enough for gem_progress.py, triage.py)
    but NOT per-file detail (check_cop.py needs the full JSON).
    """
    project_root = _find_project_root()
    corpus_md = project_root / "docs" / "corpus.md"
    if not corpus_md.exists():
        return None

    text = corpus_md.read_text()

    # Parse "Last updated: YYYY-MM-DD" from the header
    run_date = "unknown"
    m = re.search(r"Last updated:\s*(\d{4}-\d{2}-\d{2})", text)
    if m:
        run_date = m.group(1)

    # Parse summary table
    total_repos = 0
    total_offenses = 0
    total_fp = 0
    total_fn = 0
    total_matches = 0
    for line in text.splitlines():
        if "| Repos |" in line and "100%" not in line:
            m = re.search(r"\|\s*([\d,]+)\s*\|?\s*$", line)
            if m:
                total_repos = int(m.group(1).replace(",", ""))
        elif "| Offenses compared |" in line:
            m = re.search(r"\|\s*([\d,]+)\s*\|?\s*$", line)
            if m:
                total_offenses = int(m.group(1).replace(",", ""))
        elif "| FP (nitrocop extra) |" in line:
            m = re.search(r"\|\s*([\d,]+)\s*\|?\s*$", line)
            if m:
                total_fp = int(m.group(1).replace(",", ""))
        elif "| FN (nitrocop missing) |" in line:
            m = re.search(r"\|\s*([\d,]+)\s*\|?\s*$", line)
            if m:
                total_fn = int(m.group(1).replace(",", ""))
        elif "| Matches (both agree) |" in line:
            m = re.search(r"\|\s*([\d,]+)\s*\|?\s*$", line)
            if m:
                total_matches = int(m.group(1).replace(",", ""))

    # Parse diverging cops table (| Department/CopName | Matches | FP | FN | Match % |)
    by_cop = []
    seen_cops = set()
    diverging_pattern = re.compile(
        r"^\|\s*([A-Z]\w+/\w+)\s*\|\s*([\d,]+)\s*\|\s*([\d,]+)\s*\|\s*([\d,]+)\s*\|\s*([\d.]+)%\s*\|"
    )
    for line in text.splitlines():
        m = diverging_pattern.match(line)
        if m:
            cop_name = m.group(1)
            matches = int(m.group(2).replace(",", ""))
            fp = int(m.group(3).replace(",", ""))
            fn = int(m.group(4).replace(",", ""))
            total = matches + fp + fn
            match_rate = matches / total if total > 0 else 1.0
            by_cop.append({
                "cop": cop_name,
                "matches": matches,
                "fp": fp,
                "fn": fn,
                "match_rate": match_rate,
            })
            seen_cops.add(cop_name)

    # Parse perfect cops table (| Department/CopName | Matches |)
    perfect_pattern = re.compile(
        r"^\|\s*([A-Z]\w+/\w+)\s*\|\s*([\d,]+)\s*\|$"
    )
    for line in text.splitlines():
        m = perfect_pattern.match(line)
        if m:
            cop_name = m.group(1)
            if cop_name not in seen_cops:
                matches = int(m.group(2).replace(",", ""))
                by_cop.append({
                    "cop": cop_name,
                    "matches": matches,
                    "fp": 0,
                    "fn": 0,
                    "match_rate": 1.0,
                })
                seen_cops.add(cop_name)

    if not by_cop:
        return None

    data = {
        "run_date": run_date,
        "summary": {
            "total_repos": total_repos,
            "total_offenses_compared": total_offenses,
            "total_matches": total_matches,
            "total_fp": total_fp,
            "total_fn": total_fn,
        },
        "by_cop": by_cop,
        "_source": f"{corpus_md.relative_to(project_root)} (summary only, no per-file detail)",
    }

    # Write to cache
    cache_path = _cache_dir() / f"corpus-results-from-md-{run_date}.json"
    cache_path.write_text(json.dumps(data))

    # Determine head_sha from the corpus oracle commit if possible
    head_sha = ""
    sha_result = subprocess.run(
        ["git", "log", "--all", "--oneline", "--grep=corpus oracle", "-1",
         "--format=%H"],
        capture_output=True, text=True,
    )
    if sha_result.returncode == 0 and sha_result.stdout.strip():
        head_sha = sha_result.stdout.strip()

    print(
        f"Using {corpus_md.relative_to(project_root)} as fallback "
        f"(summary data only, dated {run_date})",
        file=sys.stderr,
    )
    return cache_path, 0, head_sha


def _cache_synthetic_from_dir(tmpdir: Path, run_id: int) -> None:
    """Cache synthetic-results.json from a gh-downloaded artifact directory."""
    # The artifact may nest it under bench/synthetic/ or at the top level
    for candidate in [
        tmpdir / "bench" / "synthetic" / "synthetic-results.json",
        tmpdir / "synthetic-results.json",
    ]:
        if candidate.exists():
            dest = _cache_dir() / f"synthetic-results-{run_id}.json"
            shutil.copy2(candidate, dest)
            print("Cached synthetic-results.json from artifact", file=sys.stderr)
            return


def _cache_synthetic_from_zip(zf: zipfile.ZipFile, run_id: int) -> None:
    """Cache synthetic-results.json from an artifact zip."""
    for name in zf.namelist():
        if name.endswith("synthetic-results.json"):
            dest = _cache_dir() / f"synthetic-results-{run_id}.json"
            with zf.open(name) as f:
                dest.write_bytes(f.read())
            print("Cached synthetic-results.json from artifact", file=sys.stderr)
            return


def _cache_variant_results_from_dir(tmpdir: Path, run_id: int) -> None:
    """Cache style-variant-results.json from a gh-downloaded artifact directory."""
    for candidate in [
        tmpdir / "style-variant-results.json",
    ]:
        if candidate.exists():
            dest = _cache_dir() / f"style-variant-results-{run_id}.json"
            shutil.copy2(candidate, dest)
            print("Cached style-variant-results.json from artifact", file=sys.stderr)
            return


def _cache_variant_results_from_zip(zf: zipfile.ZipFile, run_id: int) -> None:
    """Cache style-variant-results.json from an artifact zip."""
    for name in zf.namelist():
        if name.endswith("style-variant-results.json"):
            dest = _cache_dir() / f"style-variant-results-{run_id}.json"
            with zf.open(name) as f:
                dest.write_bytes(f.read())
            print("Cached style-variant-results.json from artifact", file=sys.stderr)
            return


def get_variant_results_path(run_id: int) -> Path | None:
    """Return the cached style-variant-results.json path for a given run, if available."""
    path = _cache_dir() / f"style-variant-results-{run_id}.json"
    return path if path.exists() else None


def get_synthetic_results_path(run_id: int) -> Path | None:
    """Return the cached synthetic-results.json path for a given run, if available."""
    path = _cache_dir() / f"synthetic-results-{run_id}.json"
    return path if path.exists() else None


def download_corpus_results(
    *, include_head_sha: bool = True,
) -> tuple[Path, int, str]:
    """Download corpus-results.json from the latest successful corpus-oracle CI run.

    Tries strategies in order:
    1. gh CLI (if installed and authenticated)
    2. GitHub REST API with GH_TOKEN/GH_TOKEN_FOR_ACTIONS_READ/GITHUB_TOKEN env var
    3. Parse checked-in docs/corpus.md (summary data only)
    4. Exit with helpful error message

    Returns (path_to_json, run_id, head_sha).
    If include_head_sha is False, head_sha may be empty.
    """
    project_root = _find_project_root()
    repo = _detect_github_repo()

    # Try gh first
    result = _try_gh(repo)
    if result:
        _clean_stale_local(project_root)
        return result

    # Try curl/API fallback
    result = _try_curl_api(repo)
    if result:
        _clean_stale_local(project_root)
        return result

    # Try parsing checked-in corpus markdown fallback.
    result = _try_corpus_md()
    if result:
        _clean_stale_local(project_root)
        return result

    # Nothing worked — give a helpful error
    print("\nFailed to download corpus-results.json.", file=sys.stderr)
    print("", file=sys.stderr)
    print("Options:", file=sys.stderr)
    print("  1. Install and authenticate gh: gh auth login", file=sys.stderr)
    print("  2. Set GH_TOKEN, GH_TOKEN_FOR_ACTIONS_READ, or GITHUB_TOKEN env var (needs actions:read scope)", file=sys.stderr)
    print("  3. Download manually and pass --input corpus-results.json", file=sys.stderr)
    if repo:
        print(f"  4. Visit https://github.com/{repo}/actions and download the", file=sys.stderr)
        print("     corpus-report artifact from the latest corpus-oracle run", file=sys.stderr)
    sys.exit(1)
