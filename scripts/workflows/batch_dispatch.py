#!/usr/bin/env python3
"""Rank cops by dispatchability and dispatch the top N via agent-cop-fix.

Called from the batch-dispatch workflow. Reads configuration from env vars
set by the workflow inputs.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time

# Cops with total divergence (FP+FN) above this threshold are dispatched
# in "reduce" mode where the agent aims to make progress, not fully solve.
REDUCE_MODE_THRESHOLD = 500


def _extract_cop_from_pr_title(title: str) -> str | None:
    """Extract cop name from PR titles like '[bot] Fix Department/CopName'."""
    parts = title.split("Fix ", 1)
    if len(parts) == 2:
        cop = parts[1].strip()
        # Strip " (retry)" or similar parenthetical suffixes
        paren = cop.find(" (")
        if paren != -1:
            cop = cop[:paren]
        return cop
    return None


def get_open_cop_fix_cops() -> set[str]:
    """Return cop names that already have an open type:cop-fix PR."""
    r = subprocess.run(
        ["gh", "pr", "list", "--state", "open",
         "--search", "[bot] Fix in:title",
         "--limit", "200",
         "--json", "title", "--jq", ".[].title"],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        return set()
    cops = set()
    for line in r.stdout.strip().splitlines():
        cop = _extract_cop_from_pr_title(line)
        if cop:
            cops.add(cop)
    return cops


def get_cop_issue_numbers(department: str) -> dict[str, int]:
    """Return {cop_name: issue_number} for open tracker issues."""
    r = subprocess.run(
        ["gh", "issue", "list", "--state", "open",
         "--label", "type:cop-issue",
         "--search", f"{department}/ in:title",
         "--limit", "200",
         "--json", "number,title"],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        return {}
    issues: dict[str, int] = {}
    for item in json.loads(r.stdout):
        title = item.get("title", "")
        # Titles look like "[cop] Department/CopName"
        if "[cop] " in title:
            cop = title.split("[cop] ", 1)[1].strip()
            issues[cop] = item["number"]
    return issues


def get_closed_cop_issue_cops(department: str) -> set[str]:
    """Return cop names that have a closed type:cop-issue tracker issue."""
    r = subprocess.run(
        ["gh", "issue", "list", "--state", "closed",
         "--label", "type:cop-issue",
         "--search", f"{department}/ in:title",
         "--limit", "200",
         "--json", "title"],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        return set()
    cops = set()
    for item in json.loads(r.stdout):
        title = item.get("title", "")
        if "[cop] " in title:
            cop = title.split("[cop] ", 1)[1].strip()
            cops.add(cop)
    return cops


def main() -> int:
    department = os.environ["INPUT_DEPARTMENT"]
    count = int(os.environ.get("INPUT_COUNT", "5"))
    backend = os.environ.get("INPUT_BACKEND", "codex")
    mode = os.environ.get("INPUT_MODE", "fix")

    # ── Find cops with open or draft PRs ────────────────────────────────
    open_cops = get_open_cop_fix_cops()
    skip_cops = set(open_cops)
    if open_cops:
        print(f"Skipping {len(open_cops)} cops with open PRs: {', '.join(sorted(open_cops))}")

    # ── Skip cops whose tracker issues are already closed ─────────────
    closed_issue_cops = get_closed_cop_issue_cops(department)
    if closed_issue_cops:
        print(f"Skipping {len(closed_issue_cops)} cops with closed issues: {', '.join(sorted(closed_issue_cops))}")
        skip_cops |= closed_issue_cops

    # ── Skip synth-only cops (no corpus data to validate against) ─────
    synth_only_cops: set[str] = set()
    r = subprocess.run(
        ["gh", "issue", "list", "--state", "open",
         "--label", "type:cop-issue,difficulty:synth-only",
         "--search", f"{department}/ in:title",
         "--limit", "200",
         "--json", "title"],
        capture_output=True, text=True,
    )
    if r.returncode == 0:
        for item in json.loads(r.stdout):
            title = item.get("title", "")
            if "[cop] " in title:
                cop = title.split("[cop] ", 1)[1].strip()
                synth_only_cops.add(cop)
    if synth_only_cops:
        print(f"Skipping {len(synth_only_cops)} synth-only cops: {', '.join(sorted(synth_only_cops))}")
        skip_cops |= synth_only_cops

    # ── Look up tracker issue numbers ────────────────────────────────
    cop_issues = get_cop_issue_numbers(department)
    if cop_issues:
        print(f"Found {len(cop_issues)} tracker issues for {department}")

    # ── Rank cops ──────────────────────────────────────────────────────
    # Request extra candidates so we still hit count after filtering
    rank_cmd = [
        sys.executable, "scripts/dispatch_cops.py", "rank",
        "--json",
        "--department", department,
        "--min-bugs", "0",
        "--min-matches", "0",
        "--limit", str(count + len(skip_cops)),
    ]

    print(f"Running: {' '.join(rank_cmd)}", flush=True)
    result = subprocess.run(rank_cmd, capture_output=True, text=True)
    # rank prints diagnostics to stderr
    if result.stderr:
        print(result.stderr, file=sys.stderr, end="")
    if result.returncode != 0:
        print(f"Error: rank exited with {result.returncode}", file=sys.stderr)
        return 1

    all_cops: list[dict] = json.loads(result.stdout)
    cops = [c for c in all_cops if c["cop"] not in skip_cops][:count]

    if not cops:
        skipped_open = [c["cop"] for c in all_cops if c["cop"] in open_cops]
        skipped_closed = [c["cop"] for c in all_cops if c["cop"] in closed_issue_cops]
        reasons = []
        if skipped_open:
            reasons.append(f"{len(skipped_open)} have open PRs")
        if skipped_closed:
            reasons.append(f"{len(skipped_closed)} have closed issues")
        if not all_cops:
            reasons.append("rank returned 0 candidates")
        detail = f" ({', '.join(reasons)})" if reasons else ""
        print(f"::warning::No dispatchable cops found for {department}{detail}. "
              f"Nothing to dispatch.")
        return 0

    if len(cops) < count:
        print(f"::warning::Only {len(cops)} cops available (requested {count}).")

    # ── Print selection ────────────────────────────────────────────────
    def _cop_mode(cop_entry: dict) -> str:
        if mode == "auto":
            total = cop_entry["fp"] + cop_entry["fn"]
            return "reduce" if total > REDUCE_MODE_THRESHOLD else "fix"
        return mode

    print(f"\n{'Cop':<42} {'FP':>3} {'FN':>3} {'Bugs':>4} {'Cfg':>4} {'Mode':>7}")
    print("-" * 67)
    for c in cops:
        print(f"{c['cop']:<42} {c['fp']:>3} {c['fn']:>3} "
              f"{c['code_bugs']:>4} {c['config_issues']:>4} {_cop_mode(c):>7}")
    print()

    # ── Dispatch ───────────────────────────────────────────────────────
    dispatched = 0
    failed = 0

    for c in cops:
        cop = c["cop"]
        cop_mode = _cop_mode(c)
        issue_num = cop_issues.get(cop)
        issue_suffix = f", issue #{issue_num}" if issue_num else ""
        print(f"Dispatching: {cop} ({backend}, {cop_mode}{issue_suffix})...",
              end=" ", flush=True)
        cmd = [
            "gh", "workflow", "run", "agent-cop-fix.yml",
            "-f", f"cop={cop}",
            "-f", f"backend={backend}",
            "-f", f"mode={cop_mode}",
        ]
        if issue_num:
            cmd += ["-f", f"issue_number={issue_num}"]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode == 0:
            dispatched += 1
            print("ok")
        else:
            failed += 1
            print(f"FAILED: {r.stderr.strip()}")
        # Small delay to avoid GitHub rate limits
        time.sleep(1)

    print(f"\nDispatched {dispatched}/{len(cops)}"
          + (f" ({failed} failed)" if failed else ""))

    # ── Job summary ────────────────────────────────────────────────────
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a") as f:
            f.write(f"### Batch Dispatch: {department} × {len(cops)} ({backend})\n\n")
            f.write("| Cop | FP | FN | Bugs | Cfg | Mode |\n")
            f.write("|-----|----|----|------|-----|------|\n")
            for c in cops:
                f.write(f"| {c['cop']} | {c['fp']} | {c['fn']} "
                        f"| {c['code_bugs']} | {c['config_issues']} | {_cop_mode(c)} |\n")
            f.write(f"\n**Dispatched:** {dispatched}/{len(cops)}")
            if failed:
                f.write(f" ({failed} failed)")
            mode_desc = f"{mode} (fix/reduce by divergence)" if mode == "auto" else mode
            f.write(f"  \n**Backend:** {backend} | **Mode:** {mode_desc}\n")

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
