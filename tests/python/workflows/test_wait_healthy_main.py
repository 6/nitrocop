#!/usr/bin/env python3
"""Tests for scripts/workflows/wait_healthy_main.py."""

from __future__ import annotations

import sys
from pathlib import Path
from unittest.mock import patch

SCRIPT_DIR = Path(__file__).parents[3] / "scripts" / "workflows"
sys.path.insert(0, str(SCRIPT_DIR))

import wait_healthy_main


def _mock_run(head_sha: str, checks_run: dict | None, job_conclusion: str | None = None):
    """Return patchers for get_head_sha, get_latest_checks_run, and get_job_conclusion."""
    return (
        patch.object(wait_healthy_main, "get_head_sha", return_value=head_sha),
        patch.object(wait_healthy_main, "get_latest_checks_run", return_value=checks_run),
        patch.object(wait_healthy_main, "get_job_conclusion", return_value=job_conclusion),
        patch.object(wait_healthy_main.time, "sleep"),
    )


def test_job_success_same_sha(capsys):
    """build-and-test green on HEAD — proceed immediately."""
    sha = "abc1234567890"
    run = {"headSha": sha, "conclusion": None, "status": "in_progress", "databaseId": 1}
    p1, p2, p3, p4 = _mock_run(sha, run, "success")
    with p1, p2, p3, p4:
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "5", "--interval", "1"]):
            wait_healthy_main.main()
    out = capsys.readouterr().out
    assert "green" in out.lower()
    assert "build-and-test" in out


def test_workflow_success_implies_job_success(capsys):
    """Whole workflow green — job must also be green, proceed."""
    sha = "abc1234567890"
    run = {"headSha": sha, "conclusion": "success", "status": "completed", "databaseId": 1}
    p1, p2, p3, p4 = _mock_run(sha, run, "success")
    with p1, p2, p3, p4:
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "5", "--interval", "1"]):
            wait_healthy_main.main()
    out = capsys.readouterr().out
    assert "green" in out.lower()


def test_no_runs(capsys):
    """No checks.yml runs at all — proceed."""
    p1, p2, p3, p4 = _mock_run("head111", None)
    with p1, p2, p3, p4:
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "5", "--interval", "1"]):
            wait_healthy_main.main()
    out = capsys.readouterr().out
    assert "No checks" in out


def test_skip_ci_failed_old_run(capsys):
    """HEAD is [skip ci], latest checks (old SHA) failed — proceed with warning."""
    run = {"headSha": "old222", "conclusion": "failure", "status": "completed", "databaseId": 1}
    p1, p2, p3, p4 = _mock_run("head111", run)
    with p1, p2, p3, p4:
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "5", "--interval", "1"]):
            wait_healthy_main.main()
    out = capsys.readouterr().out
    assert "no checks" in out.lower()


def test_job_in_progress_waits_then_succeeds(capsys):
    """build-and-test in_progress — wait, then succeed."""
    sha = "head111"
    run = {"headSha": sha, "conclusion": None, "status": "in_progress", "databaseId": 42}
    call_count = 0

    def mock_job(repo, run_id, job_name):
        nonlocal call_count
        call_count += 1
        if call_count < 3:
            return "in_progress"
        return "success"

    with (
        patch.object(wait_healthy_main, "get_head_sha", return_value=sha),
        patch.object(wait_healthy_main, "get_latest_checks_run", return_value=run),
        patch.object(wait_healthy_main, "get_job_conclusion", side_effect=mock_job),
        patch.object(wait_healthy_main.time, "sleep"),
    ):
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "300", "--interval", "1"]):
            wait_healthy_main.main()
    assert call_count == 3


def test_job_failure_exits_immediately():
    """build-and-test fails — exit 1 without waiting."""
    sha = "head111"
    run = {"headSha": sha, "conclusion": None, "status": "in_progress", "databaseId": 1}
    p1, p2, p3, p4 = _mock_run(sha, run, "failure")
    with p1, p2, p3, p4:
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "300", "--interval", "1"]):
            try:
                wait_healthy_main.main()
                assert False, "Should have called sys.exit(1)"
            except SystemExit as e:
                assert e.code == 1


def test_timeout_exits_nonzero():
    """Job never goes green — exit 1 after max-wait."""
    sha = "head111"
    run = {"headSha": sha, "conclusion": None, "status": "in_progress", "databaseId": 1}
    p1, p2, p3, p4 = _mock_run(sha, run, "in_progress")
    with p1, p2, p3, p4:
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "2", "--interval", "1"]):
            try:
                wait_healthy_main.main()
                assert False, "Should have called sys.exit(1)"
            except SystemExit as e:
                assert e.code == 1


def test_skip_ci_in_progress_old_run_waits(capsys):
    """HEAD is [skip ci], old checks still in_progress — should wait for job."""
    call_count = 0

    def mock_checks(repo):
        nonlocal call_count
        call_count += 1
        status = "in_progress" if call_count < 3 else "completed"
        return {"headSha": "old222", "conclusion": None if call_count < 3 else "success",
                "status": status, "databaseId": 99}

    def mock_job(repo, run_id, job_name):
        # Job succeeds on the third poll
        return "success" if call_count >= 3 else "in_progress"

    with (
        patch.object(wait_healthy_main, "get_head_sha", return_value="head111"),
        patch.object(wait_healthy_main, "get_latest_checks_run", side_effect=mock_checks),
        patch.object(wait_healthy_main, "get_job_conclusion", side_effect=mock_job),
        patch.object(wait_healthy_main.time, "sleep"),
    ):
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "300", "--interval", "1"]):
            wait_healthy_main.main()
    assert call_count == 3


def test_job_conclusion_none_treated_as_pending(capsys):
    """get_job_conclusion returns None (job not found yet) — keep waiting."""
    sha = "head111"
    run = {"headSha": sha, "conclusion": None, "status": "in_progress", "databaseId": 1}
    call_count = 0

    def mock_job(repo, run_id, job_name):
        nonlocal call_count
        call_count += 1
        if call_count < 2:
            return None  # job not started yet
        return "success"

    with (
        patch.object(wait_healthy_main, "get_head_sha", return_value=sha),
        patch.object(wait_healthy_main, "get_latest_checks_run", return_value=run),
        patch.object(wait_healthy_main, "get_job_conclusion", side_effect=mock_job),
        patch.object(wait_healthy_main.time, "sleep"),
    ):
        with patch("sys.argv", ["prog", "--repo", "test/repo", "--max-wait", "300", "--interval", "1"]):
            wait_healthy_main.main()
    assert call_count == 2
