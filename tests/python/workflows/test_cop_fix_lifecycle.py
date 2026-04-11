#!/usr/bin/env python3
"""Tests for cop_fix_lifecycle.py."""
import json
import os
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[3] / "scripts" / "workflows"))
import cop_fix_lifecycle

SCRIPT = Path(__file__).parents[3] / "scripts" / "workflows" / "cop_fix_lifecycle.py"


# ── snake_case ──────────────────────────────────────────────────────────

def test_snake_case_simple():
    assert cop_fix_lifecycle.snake_case("NegatedWhile") == "negated_while"


def test_snake_case_consecutive_caps():
    assert cop_fix_lifecycle.snake_case("ABCSize") == "abc_size"


def test_snake_case_already_snake():
    assert cop_fix_lifecycle.snake_case("already_snake") == "already_snake"


def test_snake_case_single_word():
    assert cop_fix_lifecycle.snake_case("Foo") == "foo"


# ── cop_identifiers ────────────────────────────────────────────────────

def test_cop_identifiers_standard():
    ids = cop_fix_lifecycle.cop_identifiers("Style/NegatedWhile")
    assert ids["dept_dir"] == "style"
    assert ids["cop_snake"] == "negated_while"
    assert ids["branch_prefix"] == "fix/style-negated_while"
    assert ids["filter"] == "cop::style::negated_while"


def test_cop_identifiers_rspec():
    ids = cop_fix_lifecycle.cop_identifiers("RSpec/DescribedClass")
    assert ids["dept_dir"] == "rspec"
    assert ids["cop_snake"] == "described_class"
    assert ids["branch_prefix"] == "fix/rspec-described_class"


def test_cop_identifiers_factory_bot():
    ids = cop_fix_lifecycle.cop_identifiers("FactoryBot/CreateList")
    assert ids["dept_dir"] == "factory_bot"


def test_cop_identifiers_metrics():
    ids = cop_fix_lifecycle.cop_identifiers("Metrics/ABCSize")
    assert ids["dept_dir"] == "metrics"
    assert ids["cop_snake"] == "abc_size"


# ── cmd_init ────────────────────────────────────────────────────────────

def test_init_outputs(tmp_path):
    output_file = tmp_path / "output.txt"
    env = {**os.environ, "GITHUB_OUTPUT": str(output_file)}
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "init",
         "--cop", "Style/NegatedWhile",
         "--mode", "fix",
         "--backend-input", "auto",
         "--strength-input", "auto",
         "--run-id", "12345"],
        capture_output=True, text=True, env=env,
    )
    assert result.returncode == 0
    outputs = output_file.read_text()
    assert "branch=fix/style-negated_while-12345" in outputs
    assert "filter=cop::style::negated_while" in outputs
    assert "branch_prefix=fix/style-negated_while" in outputs


def test_init_retry_mode(tmp_path):
    output_file = tmp_path / "output.txt"
    env = {**os.environ, "GITHUB_OUTPUT": str(output_file)}
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "init",
         "--cop", "Metrics/AbcSize",
         "--mode", "retry",
         "--backend-input", "claude",
         "--strength-input", "hard",
         "--run-id", "99"],
        capture_output=True, text=True, env=env,
    )
    assert result.returncode == 0
    assert "Retry cop:" in result.stderr


# ── build-prompt ────────────────────────────────────────────────────────

def test_build_prompt_basic(tmp_path):
    task_file = tmp_path / "context" / "task.md"
    final_file = tmp_path / "context" / "final-task.md"
    prior_file = tmp_path / "context" / "prior-attempts.md"
    task_file.parent.mkdir(parents=True, exist_ok=True)
    task_file.write_text("Fix the cop.\n")

    env = {
        **os.environ,
        "TASK_FILE": str(task_file),
        "FINAL_TASK_FILE": str(final_file),
        "PRIOR_ATTEMPTS_FILE": str(prior_file),
        "GITHUB_OUTPUT": str(tmp_path / "output.txt"),
    }
    # Avoid actual gh CLI calls by using fix mode (no retry)
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "build-prompt",
         "--cop", "Style/NegatedWhile",
         "--mode", "fix",
         "--extra-context", "",
         "--filter", "cop::style::negated_while"],
        capture_output=True, text=True, env=env,
    )
    assert result.returncode == 0
    content = final_file.read_text()
    assert "Before making changes, read `docs/agent-ci.md`." in content
    assert "Fix the cop." in content
    # Should NOT have retry note
    assert "CRITICAL" not in content


def test_build_prompt_with_extra_context(tmp_path):
    task_file = tmp_path / "context" / "task.md"
    final_file = tmp_path / "context" / "final-task.md"
    prior_file = tmp_path / "context" / "prior-attempts.md"
    task_file.parent.mkdir(parents=True, exist_ok=True)
    task_file.write_text("Fix the cop.\n")

    env = {
        **os.environ,
        "TASK_FILE": str(task_file),
        "FINAL_TASK_FILE": str(final_file),
        "PRIOR_ATTEMPTS_FILE": str(prior_file),
        "GITHUB_OUTPUT": str(tmp_path / "output.txt"),
    }
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "build-prompt",
         "--cop", "Style/NegatedWhile",
         "--mode", "fix",
         "--extra-context", "Focus on the edge case with blocks.",
         "--filter", "cop::style::negated_while"],
        capture_output=True, text=True, env=env,
    )
    assert result.returncode == 0
    content = final_file.read_text()
    assert "## Additional Instructions" in content
    assert "Focus on the edge case with blocks." in content


# ── _build_claim_body ───────────────────────────────────────────────────

def test_build_claim_body_with_issue():
    body = cop_fix_lifecycle._build_claim_body(
        cop="Style/NegatedWhile",
        mode="fix",
        backend_label="claude / normal",
        model_label="Claude Sonnet",
        backend_reason="auto-selected",
        run_url="https://example.com/run/1",
        issue_number="42",
    )
    assert "Style/NegatedWhile" in body
    assert "Refs #42" in body
    assert "nitrocop-cop-issue" in body
    assert "claude / normal" in body


def test_build_claim_body_without_issue():
    body = cop_fix_lifecycle._build_claim_body(
        cop="Style/NegatedWhile",
        mode="retry",
        backend_label="codex / hard",
        model_label="GPT-5.4",
        backend_reason="manual override",
        run_url="https://example.com/run/2",
        issue_number="",
    )
    assert "Style/NegatedWhile" in body
    assert "Closes" not in body
    assert "codex / hard" in body


# ── _build_task_body ────────────────────────────────────────────────────

def test_build_task_body():
    body = cop_fix_lifecycle._build_task_body(
        cop="Metrics/AbcSize",
        mode="fix",
        backend_label="claude / hard",
        model_label="Claude Opus",
        run_url="https://example.com/run/3",
        issue_number="99",
        code_bugs="5",
        tokens="2000",
        task_text="## Task\n\nFix AbcSize.\n",
    )
    assert "Metrics/AbcSize" in body
    assert "Code bugs:** 5" in body
    assert "2000 tokens" in body
    assert "## Task" in body
    assert "Refs #99" in body


# ── _generate_summary ───────────────────────────────────────────────────

def test_generate_summary_with_result(tmp_path):
    # Set up env vars pointing to tmp files
    agent_result = tmp_path / "agent" / "agent-result.json"
    stat_file = tmp_path / "recovery" / "diff.stat"
    diff_file = tmp_path / "recovery" / "diff.diff"
    git_dir = tmp_path / "recovery" / "git-activity"
    task_file = tmp_path / "context" / "task.md"
    logfile_ptr = tmp_path / "recovery" / "logfile-path.txt"
    agent_log = tmp_path / "agent" / "agent.log"

    for d in [agent_result.parent, stat_file.parent, git_dir, task_file.parent]:
        d.mkdir(parents=True, exist_ok=True)

    agent_result.write_text(json.dumps({
        "total_cost_usd": 1.23,
        "num_turns": 15,
        "result": "Fixed the cop successfully.",
    }))
    stat_file.write_text(" 3 files changed, 20 insertions(+), 5 deletions(-)\n")
    diff_file.write_text("")
    task_file.write_text("## Task\n\nFix the cop.\n")
    logfile_ptr.write_text("")
    agent_log.write_text("")

    env_patch = {
        "AGENT_RESULT_FILE": str(agent_result),
        "AGENT_RECOVERY_STAT_FILE": str(stat_file),
        "AGENT_RECOVERY_DIFF_FILE": str(diff_file),
        "AGENT_GIT_ACTIVITY_DIR": str(git_dir),
        "TASK_FILE": str(task_file),
        "AGENT_LOGFILE_POINTER_FILE": str(logfile_ptr),
        "AGENT_LOG_FILE": str(agent_log),
    }
    with patch.dict(os.environ, env_patch):
        summary = cop_fix_lifecycle._generate_summary(
            cop="Style/Foo",
            backend="claude-normal",
            mode="fix",
            run_url="https://example.com/run/1",
            run_number="42",
            base_sha="abc123",
        )

    assert "Style/Foo" in summary
    assert "$1.23" in summary
    assert "15" in summary
    assert "Fixed the cop successfully." in summary
    assert "3 files changed" in summary


def test_generate_summary_no_result(tmp_path):
    agent_result = tmp_path / "agent" / "agent-result.json"
    stat_file = tmp_path / "recovery" / "diff.stat"
    diff_file = tmp_path / "recovery" / "diff.diff"
    git_dir = tmp_path / "recovery" / "git-activity"
    task_file = tmp_path / "context" / "task.md"
    logfile_ptr = tmp_path / "recovery" / "logfile-path.txt"
    agent_log = tmp_path / "agent" / "agent.log"

    for d in [agent_result.parent, stat_file.parent, git_dir, task_file.parent]:
        d.mkdir(parents=True, exist_ok=True)

    # Empty result file
    agent_result.write_text("")
    stat_file.write_text("")
    diff_file.write_text("")
    task_file.write_text("## Task\n")
    logfile_ptr.write_text("")
    agent_log.write_text("")

    env_patch = {
        "AGENT_RESULT_FILE": str(agent_result),
        "AGENT_RECOVERY_STAT_FILE": str(stat_file),
        "AGENT_RECOVERY_DIFF_FILE": str(diff_file),
        "AGENT_GIT_ACTIVITY_DIR": str(git_dir),
        "TASK_FILE": str(task_file),
        "AGENT_LOGFILE_POINTER_FILE": str(logfile_ptr),
        "AGENT_LOG_FILE": str(agent_log),
    }
    with patch.dict(os.environ, env_patch):
        summary = cop_fix_lifecycle._generate_summary(
            cop="Style/Foo",
            backend="claude-normal",
            mode="fix",
            run_url="https://example.com/run/1",
            run_number="42",
            base_sha="abc123",
        )

    assert "empty or missing" in summary
    assert "No file changes detected." in summary


# ── cleanup-failure ─────────────────────────────────────────────────────

def test_cleanup_failure_closes_pr_then_deletes_branch_and_requeues_issue(tmp_path):
    claim_body = tmp_path / "context" / "claim-body.md"
    claim_body.parent.mkdir(parents=True, exist_ok=True)

    env_patch = {
        "CLAIM_BODY_FILE": str(claim_body),
        "AGENT_RESULT_FILE": str(tmp_path / "agent" / "agent-result.json"),
    }
    calls = []

    def fake_run_ok(cmd, **kwargs):
        del kwargs
        calls.append(cmd)
        if cmd[:4] == ["gh", "pr", "view", "https://github.com/6/nitrocop/pull/715"]:
            return subprocess.CompletedProcess(
                cmd, 0, stdout='{"headRefName":"fix/style-if_unless_modifier-23699434606"}', stderr="",
            )
        return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

    with (
        patch.dict(os.environ, env_patch),
        patch.object(cop_fix_lifecycle, "_run_ok", side_effect=fake_run_ok),
    ):
        result = cop_fix_lifecycle.cmd_cleanup_failure([
            "--cop", "Style/IfUnlessModifier",
            "--pr-url", "https://github.com/6/nitrocop/pull/715",
            "--issue-number", "376",
            "--repo", "6/nitrocop",
            "--backend-label", "claude-oauth / hard",
            "--model-label", "Claude Opus 4.6 (OAuth, high)",
            "--mode", "fix",
            "--run-url", "https://github.com/6/nitrocop/actions/runs/23699434606",
        ])

    assert result == 0
    assert calls[1] == [
        "gh", "pr", "close", "https://github.com/6/nitrocop/pull/715",
        "--repo", "6/nitrocop",
        "--comment", "Agent failed. See run: https://github.com/6/nitrocop/actions/runs/23699434606",
    ]
    assert calls[2] == [
        "gh", "api", "-X", "DELETE",
        "repos/6/nitrocop/git/refs/heads/fix/style-if_unless_modifier-23699434606",
    ]
    assert calls[4] == [
        "gh", "issue", "edit", "376",
        "--repo", "6/nitrocop",
        "--remove-label", "state:pr-open",
        "--add-label", "state:backlog",
    ]
    assert "The draft PR was closed automatically." in claim_body.read_text()


def test_cleanup_failure_includes_agent_findings_when_result_file_exists(tmp_path):
    claim_body = tmp_path / "context" / "claim-body.md"
    claim_body.parent.mkdir(parents=True, exist_ok=True)
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True, exist_ok=True)
    result_file.write_text(json.dumps({"result": "Tried collecting assignment offsets from Prism.\nNet -5 FP but 16 new FPs in ruby__tk."}))

    env_patch = {
        "CLAIM_BODY_FILE": str(claim_body),
        "AGENT_RESULT_FILE": str(result_file),
    }
    calls = []

    def fake_run_ok(cmd, **kwargs):
        del kwargs
        calls.append(cmd)
        if cmd[:4] == ["gh", "pr", "view", "https://github.com/6/nitrocop/pull/715"]:
            return subprocess.CompletedProcess(
                cmd, 0, stdout='{"headRefName":"fix/style-if_unless_modifier-23699434606"}', stderr="",
            )
        return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

    with (
        patch.dict(os.environ, env_patch),
        patch.object(cop_fix_lifecycle, "_run_ok", side_effect=fake_run_ok),
    ):
        result = cop_fix_lifecycle.cmd_cleanup_failure([
            "--cop", "Style/IfUnlessModifier",
            "--pr-url", "https://github.com/6/nitrocop/pull/715",
            "--issue-number", "376",
            "--repo", "6/nitrocop",
            "--backend-label", "claude-oauth / hard",
            "--model-label", "Claude Opus 4.6 (OAuth, high)",
            "--mode", "fix",
            "--run-url", "https://github.com/6/nitrocop/actions/runs/23699434606",
        ])

    assert result == 0
    body_text = claim_body.read_text()
    assert "Agent findings (what was tried)" in body_text
    assert "assignment offsets from Prism" in body_text


def test_cleanup_failure_warns_and_keeps_issue_state_when_pr_close_fails(tmp_path):
    claim_body = tmp_path / "context" / "claim-body.md"
    claim_body.parent.mkdir(parents=True, exist_ok=True)

    env_patch = {
        "CLAIM_BODY_FILE": str(claim_body),
        "AGENT_RESULT_FILE": str(tmp_path / "agent" / "agent-result.json"),
    }
    calls = []
    warnings = []

    def fake_run_ok(cmd, **kwargs):
        del kwargs
        calls.append(cmd)
        if cmd[:4] == ["gh", "pr", "view", "https://github.com/6/nitrocop/pull/715"]:
            return subprocess.CompletedProcess(
                cmd, 0, stdout='{"headRefName":"fix/style-if_unless_modifier-23699434606"}', stderr="",
            )
        if cmd[:4] == ["gh", "pr", "close", "https://github.com/6/nitrocop/pull/715"]:
            return subprocess.CompletedProcess(cmd, 1, stdout="", stderr="HTTP 403: forbidden")
        return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

    with (
        patch.dict(os.environ, env_patch),
        patch.object(cop_fix_lifecycle, "_run_ok", side_effect=fake_run_ok),
        patch.object(cop_fix_lifecycle, "_warning", side_effect=warnings.append),
    ):
        result = cop_fix_lifecycle.cmd_cleanup_failure([
            "--cop", "Style/IfUnlessModifier",
            "--pr-url", "https://github.com/6/nitrocop/pull/715",
            "--issue-number", "376",
            "--repo", "6/nitrocop",
            "--backend-label", "claude-oauth / hard",
            "--model-label", "Claude Opus 4.6 (OAuth, high)",
            "--mode", "fix",
            "--run-url", "https://github.com/6/nitrocop/actions/runs/23699434606",
        ])

    assert result == 0
    assert any("Close failed draft PR failed: HTTP 403: forbidden" in msg for msg in warnings)
    assert [
        "gh", "api", "-X", "DELETE",
        "repos/6/nitrocop/git/refs/heads/fix/style-if_unless_modifier-23699434606",
    ] not in calls
    assert [
        "gh", "issue", "edit", "376",
        "--repo", "6/nitrocop",
        "--remove-label", "state:pr-open",
        "--add-label", "state:backlog",
    ] not in calls
    assert "automatic cleanup could not close the draft PR" in claim_body.read_text()


# ── _read_agent_findings ───────────────────────────────────────────────

def test_read_agent_findings_with_result(tmp_path):
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True)
    result_file.write_text(json.dumps({
        "result": "Tried adding per-file detection.\n+147 FNs. Reverted.",
    }))
    env_patch = {"AGENT_RESULT_FILE": str(result_file)}
    with patch.dict(os.environ, env_patch):
        findings = cop_fix_lifecycle._read_agent_findings()
    assert "per-file detection" in findings
    assert "+147 FNs" in findings


def test_read_agent_findings_empty_file(tmp_path):
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True)
    result_file.write_text("")
    env_patch = {"AGENT_RESULT_FILE": str(result_file)}
    with patch.dict(os.environ, env_patch):
        findings = cop_fix_lifecycle._read_agent_findings()
    assert findings == ""


def test_read_agent_findings_missing_file(tmp_path):
    result_file = tmp_path / "agent" / "agent-result.json"
    env_patch = {"AGENT_RESULT_FILE": str(result_file)}
    with patch.dict(os.environ, env_patch):
        findings = cop_fix_lifecycle._read_agent_findings()
    assert findings == ""


def test_read_agent_findings_no_result_key(tmp_path):
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True)
    result_file.write_text(json.dumps({"total_cost_usd": 0.5}))
    env_patch = {"AGENT_RESULT_FILE": str(result_file)}
    with patch.dict(os.environ, env_patch):
        findings = cop_fix_lifecycle._read_agent_findings()
    assert findings == ""


def test_read_agent_findings_truncates_long_result(tmp_path):
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True)
    long_result = "\n".join(f"Line {i}" for i in range(60))
    result_file.write_text(json.dumps({"result": long_result}))
    env_patch = {"AGENT_RESULT_FILE": str(result_file)}
    with patch.dict(os.environ, env_patch):
        findings = cop_fix_lifecycle._read_agent_findings()
    lines = findings.splitlines()
    assert len(lines) == 41  # 40 content + 1 truncation notice
    assert "20 more lines" in lines[-1]


def test_read_agent_findings_invalid_json(tmp_path):
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True)
    result_file.write_text("not valid json{{{")
    env_patch = {"AGENT_RESULT_FILE": str(result_file)}
    with patch.dict(os.environ, env_patch):
        findings = cop_fix_lifecycle._read_agent_findings()
    assert findings == ""


# ── _close_pr_no_changes with findings ─────────────────────────────────

def test_close_pr_no_changes_includes_findings(tmp_path):
    claim_body = tmp_path / "context" / "claim-body.md"
    claim_body.parent.mkdir(parents=True)
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True)
    result_file.write_text(json.dumps({
        "result": "Investigated: FNs live in post-processing, not per-file scope.",
    }))
    env_patch = {
        "CLAIM_BODY_FILE": str(claim_body),
        "AGENT_RESULT_FILE": str(result_file),
    }
    calls = []

    def fake_run_ok(cmd, **kwargs):
        del kwargs
        calls.append(cmd)
        return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

    with (
        patch.dict(os.environ, env_patch),
        patch.object(cop_fix_lifecycle, "_run_ok", side_effect=fake_run_ok),
    ):
        cop_fix_lifecycle._close_pr_no_changes(
            pr_url="https://github.com/6/nitrocop/pull/999",
            cop="Lint/RedundantCopDisableDirective",
            backend_label="codex / hard",
            model_label="gpt-5.4",
            mode="reduce",
            run_url="https://github.com/6/nitrocop/actions/runs/123",
            issue_number="293",
            repo="6/nitrocop",
        )

    body = claim_body.read_text()
    assert "Agent findings (what was tried)" in body
    assert "post-processing" in body
    assert "Lint/RedundantCopDisableDirective" in body
    # Should NOT have the bare "did not produce" message when findings exist
    assert "did not produce any branch changes" not in body
    # PR should also get findings posted via gh pr comment
    pr_comment_cmds = [c for c in calls if "pr" in c and "comment" in c]
    assert pr_comment_cmds, "Expected gh pr comment call for findings"
    pr_body_file = Path(str(claim_body) + ".pr")
    assert pr_body_file.exists()
    pr_body = pr_body_file.read_text()
    assert "Agent findings" in pr_body
    assert "post-processing" in pr_body


def test_close_pr_no_changes_bare_message_without_findings(tmp_path):
    claim_body = tmp_path / "context" / "claim-body.md"
    claim_body.parent.mkdir(parents=True)
    result_file = tmp_path / "agent" / "agent-result.json"
    result_file.parent.mkdir(parents=True)
    result_file.write_text("")
    env_patch = {
        "CLAIM_BODY_FILE": str(claim_body),
        "AGENT_RESULT_FILE": str(result_file),
    }
    calls = []

    def fake_run_ok(cmd, **kwargs):
        del kwargs
        calls.append(cmd)
        return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

    with (
        patch.dict(os.environ, env_patch),
        patch.object(cop_fix_lifecycle, "_run_ok", side_effect=fake_run_ok),
    ):
        cop_fix_lifecycle._close_pr_no_changes(
            pr_url="https://github.com/6/nitrocop/pull/999",
            cop="Lint/Foo",
            backend_label="codex / hard",
            model_label="gpt-5.4",
            mode="fix",
            run_url="https://github.com/6/nitrocop/actions/runs/456",
            issue_number="100",
            repo="6/nitrocop",
        )

    body = claim_body.read_text()
    assert "did not produce any branch changes" in body
    assert "Agent findings" not in body


# ── build-prompt reduce mode collects prior attempts ───────────────────

def test_build_prompt_reduce_collects_prior_attempts(tmp_path):
    task_file = tmp_path / "context" / "task.md"
    final_file = tmp_path / "context" / "final-task.md"
    prior_file = tmp_path / "context" / "prior-attempts.md"
    task_file.parent.mkdir(parents=True, exist_ok=True)
    task_file.write_text("Fix the cop.\n")

    env = {
        **os.environ,
        "TASK_FILE": str(task_file),
        "FINAL_TASK_FILE": str(final_file),
        "PRIOR_ATTEMPTS_FILE": str(prior_file),
        "GITHUB_OUTPUT": str(tmp_path / "output.txt"),
    }

    def fake_run_ok(cmd, **kwargs):
        del kwargs
        # Simulate prior-attempts writing content
        if "prior-attempts" in cmd:
            prior_file.write_text("## Prior Attempts (1 failed, 0 merged)\n\nPrior attempt info.\n")
        return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

    with (
        patch.dict(os.environ, env),
        patch.object(cop_fix_lifecycle, "_run_ok", side_effect=fake_run_ok),
    ):
        result = cop_fix_lifecycle.cmd_build_prompt([
            "--cop", "Lint/RedundantCopDisableDirective",
            "--mode", "reduce",
            "--extra-context", "",
            "--filter", "cop::lint::redundant_cop_disable_directive",
        ])

    assert result == 0
    content = final_file.read_text()
    assert "Prior Attempts" in content
    assert "Reduce Mode" in content


def test_build_prompt_reduce_has_time_budget_and_sample_guidance(tmp_path):
    task_file = tmp_path / "context" / "task.md"
    final_file = tmp_path / "context" / "final-task.md"
    prior_file = tmp_path / "context" / "prior-attempts.md"
    task_file.parent.mkdir(parents=True, exist_ok=True)
    task_file.write_text("Fix the cop.\n")

    env = {
        **os.environ,
        "TASK_FILE": str(task_file),
        "FINAL_TASK_FILE": str(final_file),
        "PRIOR_ATTEMPTS_FILE": str(prior_file),
        "GITHUB_OUTPUT": str(tmp_path / "output.txt"),
    }

    def fake_run_ok(cmd, **kwargs):
        del kwargs
        return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

    with (
        patch.dict(os.environ, env),
        patch.object(cop_fix_lifecycle, "_run_ok", side_effect=fake_run_ok),
    ):
        result = cop_fix_lifecycle.cmd_build_prompt([
            "--cop", "Layout/IndentationWidth",
            "--mode", "reduce",
            "--extra-context", "",
            "--filter", "cop::layout::indentation_width",
        ])

    assert result == 0
    content = final_file.read_text()
    # Intermediate checks use --sample 5
    assert "--sample 5" in content
    # Final validation uses --sample 15
    assert "--sample 15" in content
    # Time budget section exists
    assert "Time Budget" in content
    assert "90 minutes" in content
    assert "commit and stop" in content
    # Should NOT tell agent to use --sample 15 for every change
    assert "after each change" not in content


def test_build_prompt_fix_mode_has_no_time_budget(tmp_path):
    task_file = tmp_path / "context" / "task.md"
    final_file = tmp_path / "context" / "final-task.md"
    prior_file = tmp_path / "context" / "prior-attempts.md"
    task_file.parent.mkdir(parents=True, exist_ok=True)
    task_file.write_text("Fix the cop.\n")

    env = {
        **os.environ,
        "TASK_FILE": str(task_file),
        "FINAL_TASK_FILE": str(final_file),
        "PRIOR_ATTEMPTS_FILE": str(prior_file),
        "GITHUB_OUTPUT": str(tmp_path / "output.txt"),
    }

    result = subprocess.run(
        [sys.executable, str(SCRIPT), "build-prompt",
         "--cop", "Style/NegatedWhile",
         "--mode", "fix",
         "--extra-context", "",
         "--filter", "cop::style::negated_while"],
        capture_output=True, text=True, env=env,
    )
    assert result.returncode == 0
    content = final_file.read_text()
    assert "Time Budget" not in content
    assert "Reduce Mode" not in content


# ── _is_rs_patch_docs_only ─────────────────────────────────────────────

def test_rs_patch_docs_only_pure_doc_additions():
    patch = (
        "+/// This is a doc comment.\n"
        "+/// Another doc line.\n"
        "+\n"
    )
    assert cop_fix_lifecycle._is_rs_patch_docs_only(patch) is True


def test_rs_patch_docs_only_code_addition():
    patch = (
        "+/// Doc comment.\n"
        "+    let x = 1;\n"
    )
    assert cop_fix_lifecycle._is_rs_patch_docs_only(patch) is False


def test_rs_patch_docs_only_code_removal():
    """The PR #1317 bug: removing code lines while only adding doc comments."""
    patch = (
        "+/// FN fix: removed the receiver check.\n"
        "+/// RuboCop does not require a receiver.\n"
        "-            if call.receiver().is_none() {\n"
        "-                return;\n"
        "-            }\n"
    )
    assert cop_fix_lifecycle._is_rs_patch_docs_only(patch) is False


def test_rs_patch_docs_only_removes_doc_comments_only():
    patch = (
        "-/// Old doc comment.\n"
        "+/// New doc comment.\n"
    )
    assert cop_fix_lifecycle._is_rs_patch_docs_only(patch) is True


def test_rs_patch_docs_only_skips_diff_headers():
    patch = (
        "--- a/src/cop/foo.rs\n"
        "+++ b/src/cop/foo.rs\n"
        "@@ -1,3 +1,3 @@\n"
        "+/// Doc line.\n"
    )
    assert cop_fix_lifecycle._is_rs_patch_docs_only(patch) is True


def test_rs_patch_docs_only_empty_patch():
    assert cop_fix_lifecycle._is_rs_patch_docs_only("") is True


def test_rs_patch_docs_only_context_lines_ignored():
    patch = (
        " fn main() {\n"
        "+/// Added doc.\n"
        " }\n"
    )
    assert cop_fix_lifecycle._is_rs_patch_docs_only(patch) is True


# ── CLI error handling ──────────────────────────────────────────────────

def test_unknown_command():
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "nonexistent"],
        capture_output=True, text=True,
    )
    assert result.returncode == 1


def test_no_args():
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True, text=True,
    )
    assert result.returncode == 1


# ── Manual runner ───────────────────────────────────────────────────────

if __name__ == "__main__":
    import tempfile

    test_snake_case_simple()
    test_snake_case_consecutive_caps()
    test_snake_case_already_snake()
    test_snake_case_single_word()
    test_cop_identifiers_standard()
    test_cop_identifiers_rspec()
    test_cop_identifiers_factory_bot()
    test_cop_identifiers_metrics()
    test_build_claim_body_with_issue()
    test_build_claim_body_without_issue()
    test_build_task_body()
    test_unknown_command()
    test_no_args()

    with tempfile.TemporaryDirectory() as d:
        test_init_outputs(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_init_retry_mode(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_build_prompt_basic(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_build_prompt_with_extra_context(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_generate_summary_with_result(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_generate_summary_no_result(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_read_agent_findings_with_result(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_read_agent_findings_empty_file(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_read_agent_findings_missing_file(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_read_agent_findings_no_result_key(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_read_agent_findings_truncates_long_result(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_read_agent_findings_invalid_json(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_close_pr_no_changes_includes_findings(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_close_pr_no_changes_bare_message_without_findings(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_build_prompt_reduce_collects_prior_attempts(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_build_prompt_reduce_has_time_budget_and_sample_guidance(Path(d))
    with tempfile.TemporaryDirectory() as d:
        test_build_prompt_fix_mode_has_no_time_budget(Path(d))
    print("All tests passed.")
