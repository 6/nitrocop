#!/usr/bin/env python3
"""Wait for the build-and-test job on main to be green before proceeding.

Only checks the ``build-and-test`` job, not the full workflow conclusion,
so agents aren't blocked by slow jobs like ``build-macos`` or ``cop-check``.

Handles [skip ci] commits gracefully: if HEAD has no checks run and the
most recent checks run (for an older SHA) isn't pending, proceeds with
a warning instead of blocking.

Usage:
    python3 scripts/workflows/wait_healthy_main.py --repo OWNER/REPO [--max-wait 600] [--interval 30]
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time

REQUIRED_JOB = "build-and-test"


def get_head_sha() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def get_latest_checks_run(repo: str) -> dict | None:
    result = subprocess.run(
        [
            "gh", "run", "list",
            "--workflow=checks.yml", "--branch=main",
            "--repo", repo, "--limit", "1",
            "--json", "headSha,conclusion,status,databaseId",
        ],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        return None
    runs = json.loads(result.stdout)
    return runs[0] if runs else None


def get_job_conclusion(repo: str, run_id: int, job_name: str) -> str | None:
    """Return the conclusion of a specific job within a workflow run."""
    result = subprocess.run(
        [
            "gh", "run", "view", str(run_id),
            "--repo", repo,
            "--json", "jobs",
        ],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        return None
    data = json.loads(result.stdout)
    for job in data.get("jobs", []):
        if job.get("name") == job_name:
            return job.get("conclusion") or job.get("status") or "unknown"
    return None


def main():
    parser = argparse.ArgumentParser(description="Wait for healthy main checks")
    parser.add_argument("--repo", required=True, help="GitHub repository (owner/repo)")
    parser.add_argument("--max-wait", type=int, default=900, help="Max wait seconds (default: 900)")
    parser.add_argument("--interval", type=int, default=30, help="Poll interval seconds (default: 30)")
    args = parser.parse_args()

    head_sha = get_head_sha()
    elapsed = 0

    while elapsed < args.max_wait:
        run = get_latest_checks_run(args.repo)

        if run is None:
            print("::notice::No checks.yml runs found — proceeding")
            return

        run_sha = run.get("headSha", "")
        run_id = run.get("databaseId")
        run_conclusion = run.get("conclusion") or run.get("status") or "unknown"

        # HEAD is a [skip ci] commit — checks.yml didn't run for it.
        # Don't wait for an older run that's already terminal.
        if run_sha != head_sha and run_conclusion not in ("in_progress", "queued"):
            print(
                f"::warning::Latest checks ({run_sha[:7]}) were {run_conclusion} "
                f"but HEAD ({head_sha[:7]}) has no checks — proceeding"
            )
            return

        # Check the specific job rather than the whole workflow
        job_status = get_job_conclusion(args.repo, run_id, REQUIRED_JOB) if run_id else None

        if job_status == "success":
            print(
                f"::notice::{REQUIRED_JOB} is green ({run_sha[:7]}) — proceeding"
            )
            return

        if job_status and job_status not in ("in_progress", "queued"):
            print(
                f"::error::{REQUIRED_JOB} concluded with {job_status} ({run_sha[:7]})"
            )
            sys.exit(1)

        status_label = job_status or run_conclusion
        print(
            f"{REQUIRED_JOB} status: {status_label} ({run_sha[:7]}) "
            f"— waiting {args.interval}s ({elapsed}s/{args.max_wait}s)"
        )
        time.sleep(args.interval)
        elapsed += args.interval

    print(f"::error::{REQUIRED_JOB} did not go green within {args.max_wait}s")
    sys.exit(1)


if __name__ == "__main__":
    main()
