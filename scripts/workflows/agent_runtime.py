#!/usr/bin/env python3
"""Define canonical runtime paths for agent workflows."""

from __future__ import annotations

import argparse
import os
import tempfile
from pathlib import Path


def build_paths_from_root(workflow: str, root: Path) -> dict[str, str]:
    paths = {
        "AGENT_RUNTIME_ROOT": str(root),
        "AGENT_AGENT_DIR": str(root / "agent"),
        "AGENT_CONTEXT_DIR": str(root / "context"),
        "AGENT_RECOVERY_DIR": str(root / "recovery"),
        "AGENT_GIT_ACTIVITY_DIR": str(root / "recovery" / "git-activity"),
        "AGENT_LOG_FILE": str(root / "agent" / "agent.log"),
        "AGENT_EVENTS_FILE": str(root / "agent" / "agent-events.jsonl"),
        "AGENT_LAST_MESSAGE_FILE": str(root / "agent" / "agent-last-message.txt"),
        "AGENT_RESULT_FILE": str(root / "agent" / "agent-result.json"),
        "AGENT_STATUS_FILE": str(root / "recovery" / "status.txt"),
        "AGENT_COMMITS_FILE": str(root / "recovery" / "commits.txt"),
        "AGENT_RECOVERY_STAT_FILE": str(root / "recovery" / "diff.stat"),
        "AGENT_RECOVERY_DIFF_FILE": str(root / "recovery" / "diff.diff"),
        "AGENT_RECOVERY_PATCH_FILE": str(root / "recovery" / "diff.patch"),
        "AGENT_SCOPE_REPORT_FILE": str(root / "recovery" / "scope.md"),
        "AGENT_LOGFILE_POINTER_FILE": str(root / "recovery" / "logfile-path.txt"),
        "GIT_ACTIVITY_BEFORE_FILE": str(root / "recovery" / "git-activity" / "before.json"),
        "GIT_ACTIVITY_AFTER_FILE": str(root / "recovery" / "git-activity" / "after.json"),
        "AGENT_ARTIFACT_MANIFEST_FILE": str(root / "recovery" / "artifacts.txt"),
        "TASK_FILE": str(root / "context" / "task.md"),
        "FINAL_TASK_FILE": str(root / "context" / "final-task.md"),
    }

    if workflow == "agent-cop-fix":
        paths.update(
            {
                "SUMMARY_FILE": str(root / "context" / "summary.md"),
                "CLAIM_BODY_FILE": str(root / "context" / "claim-body.md"),
                "PR_BODY_FILE": str(root / "context" / "pr-body.md"),
                "PRIOR_ATTEMPTS_FILE": str(root / "context" / "prior-attempts.md"),
                "STANDARD_COP_CHECK_LOG": str(root / "context" / "standard-cop-check.log"),
            }
        )
    elif workflow == "agent-pr-repair":
        paths.update(
            {
                "AGENT_REPAIR_DIR": str(root / "repair"),
                "PR_DIFF_STAT_FILE": str(root / "context" / "pr-diff.stat"),
                "PR_DIFF_FILE": str(root / "context" / "pr.diff"),
                "REPAIR_JSON_FILE": str(root / "repair" / "repair.json"),
                "REPAIR_VERIFY_SCRIPT": str(root / "repair" / "verify.sh"),
                "REPAIR_VERIFY_LOG": str(root / "repair" / "verify.log"),
                "REPAIR_COMMENT_FILE": str(root / "repair" / "comment.md"),
                "REPAIR_SUMMARY_FILE": str(root / "repair" / "summary.md"),
                "FINAL_PR_DIFF_STAT_FILE": str(root / "repair" / "final-pr-diff.stat"),
                "FINAL_PR_DIFF_FILE": str(root / "repair" / "final-pr.diff"),
                "REPAIR_COP_CHECK_PACKET_FILE": str(root / "repair" / "cop-check-packet.md"),
                "REPAIR_CORPUS_FILE": str(root / "repair" / "corpus.json"),
                "REPAIR_CHANGED_COPS_FILE": str(root / "repair" / "changed-cops.txt"),
            }
        )
    else:
        raise SystemExit(f"unknown workflow: {workflow}")

    return paths


def build_paths(workflow: str, runner_temp: Path) -> dict[str, str]:
    return build_paths_from_root(workflow, runner_temp / workflow)


def runtime_root(workflow: str) -> Path:
    configured_root = os.environ.get("AGENT_RUNTIME_ROOT")
    if configured_root:
        return Path(configured_root).resolve()
    runner_temp = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())).resolve()
    return runner_temp / workflow


def current_paths(workflow: str) -> dict[str, str]:
    defaults = build_paths_from_root(workflow, runtime_root(workflow))
    return {key: os.environ.get(key, value) for key, value in defaults.items()}


def ensure_dirs(paths: dict[str, str]) -> None:
    dir_keys = [
        "AGENT_RUNTIME_ROOT",
        "AGENT_AGENT_DIR",
        "AGENT_CONTEXT_DIR",
        "AGENT_RECOVERY_DIR",
        "AGENT_GIT_ACTIVITY_DIR",
    ]
    if "AGENT_REPAIR_DIR" in paths:
        dir_keys.append("AGENT_REPAIR_DIR")

    for key in dir_keys:
        Path(paths[key]).mkdir(parents=True, exist_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description="Emit canonical runtime paths for agent workflows")
    parser.add_argument("workflow", choices=["agent-cop-fix", "agent-pr-repair", "agent-cop-audit"])
    parser.add_argument("--runner-temp", type=Path, required=True)
    args = parser.parse_args()

    paths = build_paths(args.workflow, args.runner_temp.resolve())
    ensure_dirs(paths)
    for key, value in paths.items():
        print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
