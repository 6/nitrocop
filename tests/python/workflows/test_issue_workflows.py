#!/usr/bin/env python3
"""Smoke tests for issue-linked workflow wiring."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).parents[3]
AGENT_COP_FIX = ROOT / ".github" / "workflows" / "agent-cop-fix.yml"
COP_FIX_LIFECYCLE = ROOT / "scripts" / "workflows" / "cop_fix_lifecycle.py"
AGENT_PR_REPAIR = ROOT / ".github" / "workflows" / "agent-pr-repair.yml"
COP_ISSUE_SYNC = ROOT / ".github" / "workflows" / "cop-issue-sync.yml"
CORPUS_ORACLE = ROOT / ".github" / "workflows" / "corpus-oracle.yml"
RELEASE = ROOT / ".github" / "workflows" / "release.yml"


def test_agent_cop_fix_supports_issue_linking_and_auto_backend():
    yml = AGENT_COP_FIX.read_text()
    py = COP_FIX_LIFECYCLE.read_text()

    # Workflow inputs and orchestrator calls
    assert "issue_number:" in yml
    assert "- auto" in yml
    assert "cop_fix_lifecycle.py select-backend" in yml
    assert "cop_fix_lifecycle.py claim-pr" in yml
    assert "cop_fix_lifecycle.py finalize" in yml
    assert "Generate read-only GitHub token" not in yml
    assert "GH_TOKEN: ${{ github.token }}" in yml

    # Logic now lives in cop_fix_lifecycle.py
    assert "dispatch_cops.py" in py
    assert "offense_fixtures_have_no_unannotated_blocks" in py
    assert "Refs #{" in py
    assert "nitrocop-cop-issue" in py
    assert '"gh", "issue", "comment"' in py
    assert "docs/agent-ci.md" in py
    assert "validate_agent_changes.py" in py
    assert '"gh", "pr", "merge"' in py

    # Removed patterns should not appear in either
    assert "prepare_agent_workspace.py" not in yml
    assert "CI_SCRIPTS_DIR" not in yml
    assert "tmp: clean workspace" not in yml
    assert "git apply --3way" not in yml


def test_agent_pr_repair_reads_linked_issue_and_can_update_it():
    content = AGENT_PR_REPAIR.read_text()
    assert '--json state --jq \'.state\'' in content
    assert "--json number,title,url,body,state" in content
    assert "--require-trusted-bot" in content
    assert "types: [closed]" in content
    assert "github.event.pull_request.number" in content
    assert "linked_issue_number" in content
    assert 'gh issue comment "${{ steps.pr.outputs.linked_issue_number }}"' in content
    assert '--add-label "state:blocked"' in content
    assert "Skip closed PRs" in content
    assert "Reconfirm PR is still repairable before agent" in content
    assert "Skip closed or moved PR before agent" in content
    assert "python3 scripts/workflows/repair_retry_policy.py live-gate" in content
    assert "Skip closed or moved PR before publish" not in content
    assert 'echo "result=stale_pr" >> "$GITHUB_OUTPUT"' in content
    assert "Local Cop-Check Diagnosis" in content or "Precompute local cop-check diagnosis packet" in content
    assert '<summary>Task prompt (${{ steps.prompt.outputs.tokens }} tokens)</summary>' in content
    assert 'cat "$FINAL_TASK_FILE"' in content
    assert "Detect local cop-check verification" in content
    assert 'steps.verify_meta.outputs.needs_local_cop_check == \'true\'' in content
    assert "validate_agent_changes.py" in content
    assert "guard_profile" in content
    assert 'python3 scripts/workflows/precompute_repair_cop_check.py' in content
    assert 'python3 scripts/workflows/count_tokens.py "$FINAL_TASK_FILE"' in content
    assert "prepare_agent_workspace.py" not in content
    assert "CI_SCRIPTS_DIR" not in content
    assert "repair-workspace-" not in content
    assert "git apply --3way" not in content


def test_agent_pr_repair_checks_out_repo_before_running_local_scripts():
    content = AGENT_PR_REPAIR.read_text()
    checkout_index = content.index("uses: actions/checkout@v6")
    pr_state_index = content.index("python3 scripts/workflows/repair_retry_policy.py pr-state")
    assert checkout_index < pr_state_index


def test_agent_pr_repair_live_gate_reads_branch_name():
    content = AGENT_PR_REPAIR.read_text()
    assert "--json state,baseRefName,isCrossRepository,headRepository,author,labels,headRefName,headRefOid" in content


def test_agent_pr_repair_distinguishes_agent_failure_from_verify_failure():
    content = AGENT_PR_REPAIR.read_text()
    assert "id: agent" in content
    assert 'if: always() && steps.pr.outputs.should_run == \'true\'' in content
    assert 'if [ "${{ steps.agent.outcome }}" != "success" ]; then' in content
    assert 'echo "result=agent_failed" >> "$GITHUB_OUTPUT"' in content
    assert 'echo "result=file_guard_failed" >> "$GITHUB_OUTPUT"' in content
    assert "## Auto-repair Agent Failed" in content
    assert "## Auto-repair Verification Did Not Run" in content
    assert "## Auto-repair Rejected" in content
    assert "(verification did not run)" in content


def test_issue_sync_workflow_uses_github_token_and_dispatch_script():
    content = COP_ISSUE_SYNC.read_text()
    assert "actions/create-github-app-token@v3" not in content
    assert "GH_TOKEN: ${{ github.token }}" in content
    assert "python3 scripts/dispatch_cops.py issues-sync" in content
    assert "--binary target/debug/nitrocop" in content


def test_issue_close_workflow_uses_github_token() -> None:
    content = ROOT.joinpath(".github", "workflows", "cop-issue-close.yml").read_text()
    assert "actions/create-github-app-token@v3" not in content
    assert "GH_TOKEN: ${{ github.token }}" in content
    assert "gh issue close" in content



def test_corpus_oracle_workflow_uses_dynamic_pr_renderer():
    content = CORPUS_ORACLE.read_text()
    assert "scripts/workflows/render_corpus_oracle_pr.py" in content
    assert "actions/create-github-app-token@v3" in content
    assert "id: app-token" in content
    assert "client-id: ${{ secrets.GH_APP_ID }}" in content
    assert "private-key: ${{ secrets.GH_APP_PRIVATE_KEY }}" in content
    assert "GH_TOKEN: ${{ steps.app-token.outputs.token }}" in content
    assert "GH_TOKEN: ${{ github.token }}" not in content
    assert "--identity github-actions" in content
    assert "COMMIT_MSG=$(printf '%s' \"$PR_META\" | jq -r '.commit_message')" in content
    assert "PR_TITLE=$(printf '%s' \"$PR_META\" | jq -r '.pr_title')" in content
    assert "gh pr create \\" in content
    assert "--title \"$PR_TITLE\" \\" in content
    assert "--body-file \"$PR_BODY_FILE\" \\" in content


def test_release_workflow_merges_via_pr() -> None:
    content = RELEASE.read_text()
    assert "Release workflow must run from main" in content
    assert "GH_TOKEN: ${{ steps.app-token.outputs.token }}" in content
    assert "actions/create-github-app-token@v3" in content
    assert "--identity github-actions" in content
    assert "git push origin main" not in content
    assert 'gh pr create' in content
    assert 'gh pr merge' in content
    assert "--auto --squash --delete-branch" in content
    assert "pull-requests: write" in content


if __name__ == "__main__":
    test_agent_cop_fix_supports_issue_linking_and_auto_backend()
    test_agent_pr_repair_reads_linked_issue_and_can_update_it()
    test_agent_pr_repair_checks_out_repo_before_running_local_scripts()
    test_agent_pr_repair_distinguishes_agent_failure_from_verify_failure()
    test_issue_sync_workflow_uses_github_token_and_dispatch_script()
    test_issue_close_workflow_uses_github_token()
    test_corpus_oracle_workflow_uses_dynamic_pr_renderer()
    test_release_workflow_merges_via_pr()
    print("All tests passed.")
