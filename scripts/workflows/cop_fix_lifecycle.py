#!/usr/bin/env python3
"""Orchestrate agent-cop-fix workflow lifecycle phases.

Consolidates shell logic previously scattered across inline bash blocks
in agent-cop-fix.yml.  Each subcommand corresponds to a workflow phase.

Runtime paths (TASK_FILE, AGENT_RESULT_FILE, etc.) are read from
environment variables set by agent_runtime.py.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPTS_DIR.parent.parent

DEPT_MAP = {
    "RSpec": "rspec",
    "RSpecRails": "rspec_rails",
    "FactoryBot": "factory_bot",
}


# ── Helpers ─────────────────────────────────────────────────────────────

def _env(name: str) -> str:
    val = os.environ.get(name)
    if not val:
        raise SystemExit(f"Required environment variable {name} is not set")
    return val


def _env_path(name: str) -> Path:
    return Path(_env(name))


def _opt_env(name: str, default: str = "") -> str:
    return os.environ.get(name, default)


def _log(msg: str) -> None:
    """Print an informational message to stderr (safe when stdout → $GITHUB_OUTPUT)."""
    print(msg, file=sys.stderr)


def _output(key: str, value: str) -> None:
    """Write a key=value pair to $GITHUB_OUTPUT and stdout."""
    output_file = os.environ.get("GITHUB_OUTPUT")
    if output_file:
        with open(output_file, "a") as f:
            f.write(f"{key}={value}\n")
    print(f"{key}={value}")


def _output_multiline(key: str, value: str) -> None:
    """Write a multi-line value to $GITHUB_OUTPUT."""
    output_file = os.environ.get("GITHUB_OUTPUT")
    delim = "MULTILINE_EOF_7f3a"
    if output_file:
        with open(output_file, "a") as f:
            f.write(f"{key}<<{delim}\n")
            f.write(value)
            if not value.endswith("\n"):
                f.write("\n")
            f.write(f"{delim}\n")


def _notice(msg: str) -> None:
    print(f"::notice::{msg}", file=sys.stderr)


def _error(msg: str) -> None:
    print(f"::error::{msg}", file=sys.stderr)


def _warning(msg: str) -> None:
    print(f"::warning::{msg}", file=sys.stderr)


def _run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(cmd, text=True, capture_output=True, check=True, **kwargs)
    except subprocess.CalledProcessError as exc:
        _error(f"Command failed (exit {exc.returncode}): {' '.join(cmd)}")
        if exc.stderr:
            _error(f"stderr: {exc.stderr.strip()}")
        raise


def _run_ok(cmd: list[str], **kwargs) -> subprocess.CompletedProcess[str]:
    """Run a command, returning the result even on failure."""
    return subprocess.run(cmd, text=True, capture_output=True, check=False, **kwargs)


def _warn_best_effort_failure(
    label: str, result: subprocess.CompletedProcess[str],
) -> subprocess.CompletedProcess[str]:
    """Emit a concise warning when a best-effort command fails."""
    if result.returncode == 0:
        return result

    detail = result.stderr.strip() or result.stdout.strip() or f"exit code {result.returncode}"
    detail = " | ".join(line.strip() for line in detail.splitlines() if line.strip())
    _warning(f"{label} failed: {detail}")
    return result


def _git(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], text=True, capture_output=True, check=check)


def _gh(*args: str) -> str:
    result = _run(["gh", *args])
    return result.stdout


def _gh_api(*args: str) -> str:
    return _gh("api", *args)


def _gh_api_json(*args: str) -> dict | list:
    return json.loads(_gh_api(*args))


def snake_case(s: str) -> str:
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", s)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


def cop_identifiers(cop: str) -> dict[str, str]:
    """Compute branch/filter identifiers from a cop name like Style/NegatedWhile."""
    dept, name = cop.split("/", 1)
    dept_dir = DEPT_MAP.get(dept, snake_case(dept))
    cop_snake = snake_case(name)
    return {
        "dept_dir": dept_dir,
        "cop_snake": cop_snake,
        "branch_prefix": f"fix/{dept_dir}-{cop_snake}",
        "filter": f"cop::{dept_dir}::{cop_snake}",
    }


def read_file_or(path: Path, default: str = "") -> str:
    try:
        return path.read_text()
    except FileNotFoundError:
        return default


def write_and_read(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


# ── init ────────────────────────────────────────────────────────────────

def cmd_init(args: list[str]) -> int:
    """Compute cop identifiers and emit GITHUB_OUTPUT values."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py init")
    p.add_argument("--cop", required=True)
    p.add_argument("--mode", required=True)
    p.add_argument("--backend-input", required=True)
    p.add_argument("--strength-input", default="hard")  # kept for compat, always hard
    p.add_argument("--run-id", required=True)
    opts = p.parse_args(args)

    ids = cop_identifiers(opts.cop)
    branch = f"{ids['branch_prefix']}-{opts.run_id}"

    _output("branch_prefix", ids["branch_prefix"])
    _output("branch", branch)
    _output("filter", ids["filter"])
    _output("dept_dir", ids["dept_dir"])
    _output("cop_snake", ids["cop_snake"])

    _notice(f"{opts.mode.title()} cop: {opts.cop} (backend input: {opts.backend_input})")
    return 0


# ── select-backend ──────────────────────────────────────────────────────

def cmd_select_backend(args: list[str]) -> int:
    """Select backend (auto or manual) and resolve its full config."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py select-backend")
    p.add_argument("--cop", required=True)
    p.add_argument("--mode", required=True)
    p.add_argument("--backend-input", required=True)
    p.add_argument("--strength-input", default="hard")  # kept for compat, always hard
    p.add_argument("--issue-number", default="")
    p.add_argument("--repo", required=True)
    p.add_argument("--binary", required=True)
    opts = p.parse_args(args)

    # Resolve issue difficulty from labels
    issue_difficulty = ""
    if opts.issue_number:
        r = _run_ok([
            "gh", "issue", "view", opts.issue_number,
            "--repo", opts.repo,
            "--json", "labels", "--jq", ".labels[].name",
        ])
        if r.returncode == 0:
            for label in r.stdout.strip().splitlines():
                label = label.strip()
                if label.startswith("difficulty:"):
                    issue_difficulty = label.split(":", 1)[1]

    if opts.backend_input == "auto":
        cmd = [
            sys.executable, str(SCRIPTS_DIR.parent / "dispatch_cops.py"), "backend",
            "--repo", opts.repo,
            "--cop", opts.cop,
            "--mode", opts.mode,
            "--binary", opts.binary,
        ]
        if issue_difficulty:
            cmd += ["--issue-difficulty", issue_difficulty]
        result = _run(cmd)
        # Forward all output lines (key=value)
        for line in result.stdout.strip().splitlines():
            line = line.strip()
            if "=" in line:
                key, _, val = line.partition("=")
                _output(key, val)
    else:
        # Manual backend selection — always hard
        result = _run([
            sys.executable, str(SCRIPTS_DIR / "resolve_backend.py"),
            "choose", opts.backend_input, "hard", "hard",
        ])
        for line in result.stdout.strip().splitlines():
            line = line.strip()
            if "=" in line:
                key, _, val = line.partition("=")
                _output(key, val)

        _output("tier", "3")
        _output("code_bugs", "0")
        _output("config_issues", "0")
        _output("easy", "false")

    return 0


# ── resolve-backend ─────────────────────────────────────────────────────

def cmd_resolve_backend(args: list[str]) -> int:
    """Resolve a backend name to its full config."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py resolve-backend")
    p.add_argument("--backend", required=True)
    opts = p.parse_args(args)

    result = _run([
        sys.executable, str(SCRIPTS_DIR / "resolve_backend.py"), opts.backend,
    ])
    for line in result.stdout.strip().splitlines():
        line = line.strip()
        if "=" in line:
            key, _, val = line.partition("=")
            _output(key, val)
    return 0


# ── skip-fixed ──────────────────────────────────────────────────────────

def cmd_skip_fixed(args: list[str]) -> int:
    """Handle the case where pre-diagnostic found no code bugs."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py skip-fixed")
    p.add_argument("--cop", required=True)
    p.add_argument("--issue-number", default="")
    p.add_argument("--repo", required=True)
    p.add_argument("--run-url", required=True)
    p.add_argument("--backend-input", default="")
    p.add_argument("--mode", default="fix")
    opts = p.parse_args(args)

    _warning("No code bugs found — cop appears already fixed. Skipping agent.")
    _log("All FP/FN examples are config/context issues or already detected.")

    if opts.issue_number:
        body = (
            f"No fix PR was created for `{opts.cop}`.\n\n"
            f"Pre-diagnostic found no reproducible code bugs in the current "
            f"corpus examples, so the workflow skipped agent execution.\n\n"
            f"- Backend input: `{opts.backend_input}`\n"
            f"- Mode: `{opts.mode}`\n"
            f"- Run: {opts.run_url}\n"
        )
        claim_body = _env_path("CLAIM_BODY_FILE")
        write_and_read(claim_body, body)
        _run_ok([
            "gh", "issue", "comment", opts.issue_number,
            "--repo", opts.repo,
            "--body-file", str(claim_body),
        ])
    return 0


# ── build-prompt ────────────────────────────────────────────────────────

def cmd_build_prompt(args: list[str]) -> int:
    """Assemble FINAL_TASK_FILE from task + prior attempts + extra context."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py build-prompt")
    p.add_argument("--cop", required=True)
    p.add_argument("--mode", required=True)
    p.add_argument("--extra-context", default="")
    p.add_argument("--filter", required=True)
    opts = p.parse_args(args)

    task_file = _env_path("TASK_FILE")
    final_task_file = _env_path("FINAL_TASK_FILE")
    prior_attempts_file = _env_path("PRIOR_ATTEMPTS_FILE")

    parts: list[str] = []
    parts.append("Before making changes, read `docs/agent-ci.md`.\n")
    parts.append(task_file.read_text())

    # Collect prior attempts so the agent knows what was already tried.
    # In retry mode, also close stale PRs.
    if opts.mode in ("retry", "reduce"):
        _run_ok([
            sys.executable, str(SCRIPTS_DIR.parent / "dispatch_cops.py"),
            "prior-attempts",
            "--cop", opts.cop,
            "--output", str(prior_attempts_file),
        ])
        prior = read_file_or(prior_attempts_file)
        if prior.strip():
            parts.append("\n" + prior)

    if opts.mode == "retry":
        # Close prior open PRs
        ids = cop_identifiers(opts.cop)
        r = _run_ok([
            "gh", "pr", "list", "--state", "open",
            "--head", ids["branch_prefix"],
            "--json", "number", "--jq", ".[].number",
        ])
        pr_numbers = r.stdout.strip().splitlines() if r.returncode == 0 and r.stdout.strip() else []
        if not pr_numbers:
            r2 = _run_ok([
                "gh", "pr", "list", "--state", "open",
                "--json", "number,headRefName",
                "--jq", f'.[] | select(.headRefName | startswith("{ids["branch_prefix"]}")) | .number',
            ])
            if r2.returncode == 0 and r2.stdout.strip():
                pr_numbers = r2.stdout.strip().splitlines()
        for pr_num in pr_numbers:
            pr_num = pr_num.strip()
            if pr_num:
                _log(f"Closing stale PR #{pr_num}")
                _run_ok(["gh", "pr", "close", pr_num, "--comment", "Superseded by retry attempt."])

    if opts.extra_context:
        parts.append(f"\n## Additional Instructions\n{opts.extra_context}")

    if opts.mode == "retry":
        parts.append(
            "\n**CRITICAL:** Do NOT repeat approaches that already failed. "
            "Read the prior attempts carefully and try a DIFFERENT strategy."
        )

    if opts.mode == "reduce":
        parts.append(
            f"\n## Reduce Mode — Progress Over Perfection\n"
            f"This cop has high divergence (many FP/FN). Your goal is to **significantly "
            f"reduce** the FP and FN counts, not necessarily reach zero.\n\n"
            f"### Strategy\n"
            f"- Focus on the **single most common pattern** in the corpus examples first\n"
            f"- A fix that eliminates 50% of FP or FN is a great outcome\n"
            f"- Do NOT try to handle every edge case — target the highest-impact pattern\n"
            f"- For **intermediate checks** while iterating, use:\n"
            f"  `python3 scripts/check_cop.py {opts.cop} --rerun --clone --sample 5`\n"
            f"  This is faster because it clones fewer extra repos.\n"
            f"- For the **final validation** before stopping, use:\n"
            f"  `python3 scripts/check_cop.py {opts.cop} --rerun --clone --sample 15`\n"
            f"  Run this exactly once at the end, not after every change.\n"
            f"- Stop when you have a clean improvement with no regressions, even if "
            f"FP/FN are not zero\n\n"
            f"### Time Budget\n"
            f"You have roughly 90 minutes total. Corpus validation runs are expensive "
            f"(15-20 minutes each for high-divergence cops). Plan accordingly:\n"
            f"- Spend at most 30 minutes investigating and writing the fix\n"
            f"- Use `--sample 5` for quick intermediate checks (1-2 runs max)\n"
            f"- Reserve time for one final `--sample 15` validation\n"
            f"- If you have a clean partial fix at 60 minutes, **commit and stop** — "
            f"a landed partial reduction is better than a perfect fix that times out\n"
            f"- Do NOT start a new corpus validation run if more than 70 minutes have "
            f"elapsed — commit what you have\n\n"
            f"### Success Criteria\n"
            f"- Any **meaningful net reduction** in FP+FN is a success\n"
            f"- Zero regressions in the opposite direction (do not trade FP for FN)\n"
            f"- The existing match count must not regress\n"
        )

    final_content = "\n".join(parts)
    write_and_read(final_task_file, final_content)
    _log(f"=== Final prompt ===\n{len(final_content.splitlines())} lines")
    return 0


# ── claim-pr ────────────────────────────────────────────────────────────

def _build_claim_body(
    cop: str,
    mode: str,
    backend_label: str,
    model_label: str,
    backend_reason: str,
    run_url: str,
    issue_number: str,
) -> str:
    lines = [
        "> **Status:** Agent is working on this fix...",
        ">",
        f"> **Cop:** `{cop}` | **Backend:** {backend_label} | **Model:** {model_label} | **Mode:** {mode}",
        f"> **Backend reason:** {backend_reason}",
        f"> **Run:** {run_url}",
        "",
    ]
    if issue_number:
        lines += [
            f"Refs #{issue_number}",
            "",
            f"<!-- nitrocop-cop-issue: number={issue_number} cop={cop} -->",
            "",
        ]
    return "\n".join(lines)


def _build_task_body(
    cop: str,
    mode: str,
    backend_label: str,
    model_label: str,
    run_url: str,
    issue_number: str,
    code_bugs: str,
    tokens: str,
    task_text: str,
) -> str:
    lines = [
        "> **Status:** Agent is working on this fix...",
        ">",
        f"> **Cop:** `{cop}` | **Backend:** {backend_label} | **Model:** {model_label} | **Mode:** {mode}",
        f"> **Code bugs:** {code_bugs} | **Run:** {run_url}",
        "",
    ]
    if issue_number:
        lines += [
            f"Refs #{issue_number}",
            "",
            f"<!-- nitrocop-cop-issue: number={issue_number} cop={cop} -->",
            "",
        ]
    lines += [
        "<details>",
        f"<summary>Task prompt ({tokens} tokens)</summary>",
        "",
        task_text,
        "",
        "</details>",
    ]
    return "\n".join(lines)


def cmd_claim_pr(args: list[str]) -> int:
    """Create branch via API, create draft PR, update body with task prompt."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py claim-pr")
    p.add_argument("--cop", required=True)
    p.add_argument("--mode", required=True)
    p.add_argument("--branch", required=True)
    p.add_argument("--backend", required=True)
    p.add_argument("--backend-label", required=True)
    p.add_argument("--model-label", required=True)
    p.add_argument("--backend-reason", required=True)
    p.add_argument("--repo", required=True)
    p.add_argument("--run-url", required=True)
    p.add_argument("--issue-number", default="")
    p.add_argument("--code-bugs", required=True)
    p.add_argument("--tokens", required=True)
    opts = p.parse_args(args)

    task_file = _env_path("TASK_FILE")
    claim_body_file = _env_path("CLAIM_BODY_FILE")

    # Check for existing PR
    r = _run_ok([
        "gh", "pr", "list", "--state", "open",
        "--search", f"[bot] Fix {opts.cop} in:title",
        "--json", "number,title,url", "--jq", ".[0].url // empty",
    ])
    existing = r.stdout.strip() if r.returncode == 0 else ""
    if existing:
        _error(f"Skipping {opts.cop} — PR already open: {existing}")
        return 1

    # Create branch via API
    parent_sha = _gh_api_json(f"repos/{opts.repo}/git/ref/heads/main")["object"]["sha"]
    tree_sha = _gh_api_json(f"repos/{opts.repo}/git/commits/{parent_sha}")["tree"]["sha"]
    commit = _gh_api_json(
        f"repos/{opts.repo}/git/commits",
        "-f", f"message=[bot] Fix {opts.cop}: in progress",
        "-f", f"tree={tree_sha}",
        "-f", f"parents[]={parent_sha}",
    )
    _gh_api(
        f"repos/{opts.repo}/git/refs",
        "-f", f"ref=refs/heads/{opts.branch}",
        "-f", f"sha={commit['sha']}",
    )

    # Ensure labels exist
    model_label_name = f"model:{opts.backend}"
    for label, color in [
        ("type:cop-fix", "0e8a16"),
        (model_label_name, "c2e0c6"),
        ("state:backlog", "fbca04"),
        ("state:pr-open", "0e8a16"),
        ("state:blocked", "b60205"),
    ]:
        _run_ok(["gh", "label", "create", label, "--repo", opts.repo, "--color", color])

    mode_note = f" ({opts.mode})" if opts.mode not in ("fix",) else ""

    # Create draft PR with initial body
    body = _build_claim_body(
        opts.cop, opts.mode, opts.backend_label, opts.model_label,
        opts.backend_reason, opts.run_url, opts.issue_number,
    )
    write_and_read(claim_body_file, body)

    pr_cmd = [
        "gh", "pr", "create",
        "--draft", "--base", "main", "--head", opts.branch,
        "--label", f"type:cop-fix,{model_label_name}",
        "--title", f"[bot] Fix {opts.cop}{mode_note}",
        "--body-file", str(claim_body_file),
    ]
    # Retry: the branch was just created via API and may not be visible yet
    import time
    for attempt in range(3):
        r = _run_ok(pr_cmd)
        if r.returncode == 0:
            break
        _warning(f"gh pr create attempt {attempt + 1}/3 failed: {r.stderr.strip()}")
        if attempt < 2:
            time.sleep(2)
    else:
        _error(f"gh pr create failed after 3 attempts: {r.stderr.strip()}")
        raise subprocess.CalledProcessError(r.returncode, pr_cmd, r.stdout, r.stderr)
    pr_url = r.stdout.strip()
    _output("pr_url", pr_url)
    _log(f"Created draft PR: {pr_url}")

    # Transition issue label
    if opts.issue_number:
        _run_ok([
            "gh", "issue", "edit", opts.issue_number, "--repo", opts.repo,
            "--remove-label", "state:backlog,state:blocked",
            "--add-label", "state:pr-open",
        ])

    # Update body with task prompt
    task_body = _build_task_body(
        opts.cop, opts.mode, opts.backend_label, opts.model_label,
        opts.run_url, opts.issue_number, opts.code_bugs, opts.tokens,
        task_file.read_text(),
    )
    write_and_read(claim_body_file, task_body)
    _run(["gh", "pr", "edit", pr_url, "--body-file", str(claim_body_file)])

    return 0


# ── prepare-branch ──────────────────────────────────────────────────────

def cmd_prepare_branch(args: list[str]) -> int:
    """Switch to claimed PR branch, prepopulate fixtures, append helpers."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py prepare-branch")
    p.add_argument("--branch", required=True)
    p.add_argument("--cop", required=True)
    p.add_argument("--filter", required=True)
    opts = p.parse_args(args)

    task_file = _env_path("TASK_FILE")
    final_task_file = _env_path("FINAL_TASK_FILE")

    # Configure git identity
    _run([sys.executable, str(SCRIPTS_DIR / "workflow_git.py"), "configure"])

    # Switch to branch
    _git("fetch", "origin", opts.branch)
    check = _git("show-ref", "--verify", "--quiet", f"refs/heads/{opts.branch}", check=False)
    if check.returncode == 0:
        _git("switch", opts.branch)
    else:
        _git("switch", "-c", opts.branch, "--track", f"origin/{opts.branch}")

    base_sha = _git("rev-parse", "HEAD").stdout.strip()
    _output("branch_base_sha", base_sha)

    # Prepopulate fixtures with FN corpus examples
    # cop::dept::name -> tests/fixtures/cops/dept/name
    fixture_dir = "tests/fixtures/cops/" + opts.filter.replace("cop::", "").replace("::", "/")
    _run_ok([
        sys.executable, str(SCRIPTS_DIR / "prepopulate_fixtures.py"),
        str(task_file), opts.cop, fixture_dir,
    ])

    # Append helper scripts section
    helper_result = _run_ok(
        [sys.executable, str(SCRIPTS_DIR / "render_helper_scripts_section.py")],
        cwd=str(REPO_ROOT),
    )
    if helper_result.returncode == 0 and helper_result.stdout.strip():
        with open(final_task_file, "a") as f:
            f.write(helper_result.stdout)

    return 0


# ── snapshot ────────────────────────────────────────────────────────────

def _generate_summary(
    cop: str,
    backend: str,
    mode: str,
    run_url: str,
    run_number: str,
    base_sha: str,
) -> str:
    """Generate the workflow step summary markdown."""
    agent_result_file = _env_path("AGENT_RESULT_FILE")
    recovery_stat_file = _env_path("AGENT_RECOVERY_STAT_FILE")
    recovery_diff_file = _env_path("AGENT_RECOVERY_DIFF_FILE")
    git_activity_dir = _env_path("AGENT_GIT_ACTIVITY_DIR")
    task_file = _env_path("TASK_FILE")
    logfile_pointer = _env_path("AGENT_LOGFILE_POINTER_FILE")
    agent_log_file = _env_path("AGENT_LOG_FILE")

    lines: list[str] = []
    lines.append(f"# Agent Cop {mode.title()}: {cop}")
    lines.append("")
    lines.append(f"**Backend:** {backend} | **Mode:** {mode}")
    lines.append(f"**Run:** [#{run_number}]({run_url})")
    lines.append("")

    # Agent result
    result_data: dict = {}
    if agent_result_file.exists() and agent_result_file.stat().st_size > 0:
        try:
            result_data = json.loads(agent_result_file.read_text())
        except json.JSONDecodeError:
            pass

    if result_data:
        cost = result_data.get("total_cost_usd", "?")
        turns = result_data.get("num_turns", "?")
        result_text = str(result_data.get("result", "no result"))
        lines.append("## Result\n")
        lines.append("| Metric | Value |")
        lines.append("|--------|-------|")
        lines.append(f"| Cost | ${cost} |")
        lines.append(f"| Turns | {turns} |")
        lines.append("")
        lines.append("### Agent Output\n")
        lines.append("```")
        lines.extend(result_text.splitlines()[:50])
        lines.append("```")
    else:
        lines.append("## Result\n")
        lines.append("No agent result (agent-result.json is empty or missing).")
    lines.append("")

    # Changes
    lines.append("## Changes\n")
    stat_text = read_file_or(recovery_stat_file)
    if stat_text.strip():
        lines.append("```")
        lines.append(stat_text.rstrip())
        lines.append("```")
    else:
        lines.append("No file changes detected.")
    lines.append("")

    # Full diff
    diff_text = read_file_or(recovery_diff_file)
    if diff_text.strip():
        diff_lines = diff_text.splitlines()
        lines.append("<details>")
        lines.append("<summary>Full diff</summary>\n")
        lines.append("```diff")
        lines.extend(diff_lines[:400])
        if len(diff_lines) > 400:
            lines.append("")
            lines.append("... diff truncated after 400 lines ...")
        lines.append("```")
        lines.append("</details>")
        lines.append("")

    # Git activity
    git_report = git_activity_dir / "report.md" if isinstance(git_activity_dir, Path) else Path(str(git_activity_dir)) / "report.md"
    report_text = read_file_or(git_report)
    if report_text.strip():
        lines.append("<details>")
        lines.append("<summary>Additional git activity</summary>\n")
        lines.append(report_text.rstrip())
        lines.append("")
        lines.append("</details>")
        lines.append("")

    # Task prompt excerpt
    task_text = read_file_or(task_file)
    lines.append("## Task Prompt (first 30 lines)\n")
    lines.append("```markdown")
    lines.extend(task_text.splitlines()[:30] if task_text else ["(task.md not found)"])
    lines.append("```")
    lines.append("")

    # Pre-diagnostic results
    if task_text:
        diag_match = re.search(r"(## Pre-diagnostic Results.*?)(?=\n## |\Z)", task_text, re.DOTALL)
        if diag_match:
            diag_text = diag_match.group(1).strip()
            diag_lines = diag_text.splitlines()
            lines.append("## Pre-diagnostic Results\n")
            # Skip the header line itself
            lines.extend(diag_lines[1:51])
            lines.append("")

    # Agent conversation log
    logfile_path = read_file_or(logfile_pointer).strip()
    if logfile_path and Path(logfile_path).exists():
        lines.append("## Agent Conversation\n")
        r = _run_ok([
            sys.executable, str(SCRIPTS_DIR / "agent_logs.py"),
            "extract", logfile_path,
        ])
        if r.returncode == 0 and r.stdout.strip():
            lines.append(r.stdout.rstrip())
        lines.append("")

    # Agent stderr tail
    stderr_text = read_file_or(agent_log_file)
    if stderr_text.strip():
        stderr_lines = stderr_text.splitlines()
        lines.append("<details>")
        lines.append("<summary>Agent stderr (last 30 lines)</summary>\n")
        lines.append("```")
        lines.extend(stderr_lines[-30:])
        lines.append("```")
        lines.append("</details>")

    return "\n".join(lines)


def cmd_snapshot(args: list[str]) -> int:
    """Generate cop-specific summary from agent outputs.

    Git snapshot, log finding, git activity capture, artifact manifest,
    and leak scanning are handled by the run-agent composite action
    (for both CLI and action backend paths).  This command only generates
    the cop-specific summary markdown and prints the agent result.
    """
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py snapshot")
    p.add_argument("--base-sha", required=True)
    p.add_argument("--cop", required=True)
    p.add_argument("--backend", required=True)
    p.add_argument("--mode", required=True)
    p.add_argument("--run-url", required=True)
    p.add_argument("--run-number", required=True)
    opts = p.parse_args(args)

    summary_file = _env_path("SUMMARY_FILE")

    # Generate summary (reads from files already populated by run-agent action)
    summary = _generate_summary(
        opts.cop, opts.backend, opts.mode,
        opts.run_url, opts.run_number, opts.base_sha,
    )
    write_and_read(summary_file, summary)

    # Print agent summary
    agent_result_file = _env_path("AGENT_RESULT_FILE")
    if agent_result_file.exists() and agent_result_file.stat().st_size > 0:
        try:
            data = json.loads(agent_result_file.read_text())
            _log("=== Agent Summary ===")
            _log(f"Cost: {data.get('total_cost_usd', '?')}")
            _log(f"Turns: {data.get('num_turns', '?')}")
            result_text = str(data.get("result", "no result"))
            _log(f"Result: {result_text[:200]}")
        except json.JSONDecodeError:
            pass

    return 0


def _is_rs_patch_docs_only(rs_patch: str) -> bool:
    """Return True when every added/removed line is a doc comment or blank.

    A fix that deletes code logic (even if it only adds ``///`` doc
    comments) is a real change, not docs-only.
    """
    for line in rs_patch.splitlines():
        # Check both added (+) and removed (-) lines for real code.
        # Skip diff headers (+++/---) and context lines.
        if line.startswith("+++") or line.startswith("---"):
            continue
        if not line.startswith("+") and not line.startswith("-"):
            continue
        content = line[1:].strip()
        if not content or content.startswith("///"):
            continue
        # Any non-doc, non-blank changed line in .rs means real logic
        return False
    return True


def _is_docs_only_change(signed_sha: str, repo: str) -> bool:
    """Check if .rs file changes are only doc comments (///) — no logic changes.

    Fixture files (.rb) are always allowed. Returns True only when every
    added AND removed line in .rs files is a doc comment or blank.
    A fix that deletes code logic (even if it only adds doc comments) is
    a real change, not docs-only.
    """
    r = _run_ok(["gh", "api", f"repos/{repo}/compare/main...{signed_sha}",
                 "--jq", '.files[] | select(.filename | endswith(".rs")) | .patch // empty'])
    if r.returncode != 0:
        return False
    rs_patch = r.stdout.strip()
    if not rs_patch:
        # No .rs files changed at all — only fixtures. That's docs-only.
        return True
    return _is_rs_patch_docs_only(rs_patch)


# ── finalize ────────────────────────────────────────────────────────────

def _read_agent_findings() -> str:
    """Extract the agent's investigation summary from the result file.

    When an agent investigates a cop, tries a fix, and reverts, the findings
    are only in the result file.  Including them in the issue comment prevents
    future agents from repeating the same failed approach.
    """
    result_file = _env_path("AGENT_RESULT_FILE")
    if not result_file.exists() or result_file.stat().st_size == 0:
        return ""
    try:
        data = json.loads(result_file.read_text())
    except (json.JSONDecodeError, OSError):
        return ""
    result_text = str(data.get("result", "")).strip()
    if not result_text:
        return ""
    # Truncate to keep issue comments reasonable
    lines = result_text.splitlines()
    if len(lines) > 40:
        lines = [*lines[:40], f"... ({len(lines) - 40} more lines, see run log)"]
    return "\n".join(lines)


def _close_pr_no_changes(
    pr_url: str,
    cop: str,
    backend_label: str,
    model_label: str,
    mode: str,
    run_url: str,
    issue_number: str,
    repo: str,
) -> None:
    if issue_number:
        body = (
            f"No fix PR was produced for `{cop}`.\n\n"
            f"- Backend: `{backend_label}`\n"
            f"- Model: `{model_label}`\n"
            f"- Mode: `{mode}`\n"
            f"- Run: {run_url}\n\n"
        )
        findings = _read_agent_findings()
        if findings:
            body += (
                "<details>\n"
                "<summary>Agent findings (what was tried)</summary>\n\n"
                f"```\n{findings}\n```\n"
                "</details>\n"
            )
        else:
            body += "The agent did not produce any branch changes.\n"
        claim_body = _env_path("CLAIM_BODY_FILE")
        write_and_read(claim_body, body)
        _run_ok(["gh", "issue", "comment", issue_number, "--repo", repo, "--body-file", str(claim_body)])
        _run_ok([
            "gh", "issue", "edit", issue_number, "--repo", repo,
            "--remove-label", "state:pr-open",
            "--add-label", "state:backlog",
        ])
    # Post agent findings to the PR so they're visible without checking the
    # linked issue or run log.
    pr_findings = _read_agent_findings()
    if pr_findings:
        pr_body = (
            "Agent produced no changes.\n\n"
            "<details>\n"
            "<summary>Agent findings (what was tried)</summary>\n\n"
            f"```\n{pr_findings}\n```\n"
            "</details>\n"
        )
        pr_comment_file = Path(str(_env_path("CLAIM_BODY_FILE")) + ".pr")
        write_and_read(pr_comment_file, pr_body)
        _run_ok(["gh", "pr", "comment", pr_url, "--body-file", str(pr_comment_file)])
        _run_ok(["gh", "pr", "close", pr_url, "--delete-branch"])
    else:
        _run_ok(["gh", "pr", "close", pr_url, "--comment", "Agent produced no changes.", "--delete-branch"])


def _close_pr_rejected(
    pr_url: str,
    cop: str,
    issue_number: str,
    repo: str,
    run_url: str,
    scope_report: str,
) -> None:
    body = (
        f"## Agent Fix Rejected\n\n"
        f"The workflow rejected this attempt because it edited files outside "
        f"the allowed scope for `agent-cop-fix`.\n\n"
        f"{scope_report}\n"
        f"- Run: {run_url}\n"
    )
    claim_body = _env_path("CLAIM_BODY_FILE")
    write_and_read(claim_body, body)
    _run_ok(["gh", "pr", "comment", pr_url, "--repo", repo, "--body-file", str(claim_body)])

    if issue_number:
        _run_ok(["gh", "issue", "comment", issue_number, "--repo", repo, "--body-file", str(claim_body)])
        # Return to backlog so the cop can be retried — scope violations are
        # transient (e.g., agent scratch files), not permanent blockers.
        _run_ok([
            "gh", "issue", "edit", issue_number, "--repo", repo,
            "--remove-label", "state:pr-open,state:blocked",
            "--add-label", "state:backlog",
        ])

    _run_ok([
        "gh", "pr", "close", pr_url,
        "--comment", "Agent edited files outside the allowed scope.",
        "--delete-branch",
    ])


def _build_final_pr_body(
    cop: str,
    mode: str,
    backend_label: str,
    model_label: str,
    run_url: str,
    run_number: str,
    issue_number: str,
    tokens: str,
    signed_sha: str,
    parent_sha: str,
    repo: str,
    *,
    docs_only: bool = False,
) -> str:
    agent_result_file = _env_path("AGENT_RESULT_FILE")
    task_file = _env_path("TASK_FILE")

    result_data: dict = {}
    if agent_result_file.exists() and agent_result_file.stat().st_size > 0:
        try:
            result_data = json.loads(agent_result_file.read_text())
        except json.JSONDecodeError:
            pass

    turns = result_data.get("num_turns", "?")
    result_text = str(result_data.get("result", "no result"))

    # Diff stat
    diff_stat = ""
    r = _git("diff", "--stat", f"{parent_sha}..{signed_sha}", check=False)
    if r.returncode == 0 and r.stdout.strip():
        diff_stat = r.stdout.strip().splitlines()[-1]
    if not diff_stat:
        r2 = _run_ok([
            "gh", "api", f"repos/{repo}/compare/{parent_sha}...{signed_sha}",
            "--jq", r'"\(.files | length) files changed"',
        ])
        if r2.returncode == 0:
            diff_stat = f"  {r2.stdout.strip()}"

    lines = [
        f"Automated {mode} fix for `{cop}` corpus conformance.",
        "",
    ]
    if issue_number:
        link_keyword = "Refs"
        lines += [
            f"{link_keyword} #{issue_number}",
            "",
            f"<!-- nitrocop-cop-issue: number={issue_number} cop={cop} -->",
            "",
        ]
    lines += [
        "## Details",
        "",
        "| | |",
        "|---|---|",
        f"| **Cop** | `{cop}` |",
        f"| **Backend** | {backend_label} |",
        f"| **Model** | {model_label} |",
        f"| **Mode** | {mode} |",
        f"| **Turns** | {turns} |",
        f"| **Run** | [#{run_number}]({run_url}) |",
        "",
        "## Result",
        "",
        "```",
        *result_text.splitlines()[:80],
        "```",
        "",
        "## Changes",
        "",
        "```",
        diff_stat,
        "```",
        "",
    ]

    # Agent conversation excerpt
    logfile_path = read_file_or(Path(str(_env_path("AGENT_LOGFILE_POINTER_FILE")))).strip()
    if logfile_path and Path(logfile_path).exists():
        r = _run_ok([
            sys.executable, str(SCRIPTS_DIR / "agent_logs.py"),
            "extract", logfile_path, "--max-lines", "120",
        ])
        if r.returncode == 0 and r.stdout.strip():
            lines += [
                "<details>",
                "<summary>Agent conversation excerpt</summary>",
                "",
                r.stdout.rstrip(),
                "",
                "</details>",
                "",
            ]

    # Task prompt
    task_text = read_file_or(task_file)
    lines += [
        "<details>",
        f"<summary>Task prompt ({tokens} tokens)</summary>",
        "",
        task_text.rstrip(),
        "",
        "</details>",
    ]
    return "\n".join(lines)


def cmd_finalize(args: list[str]) -> int:
    """Post-agent decision tree: validate, format, push, update PR, gate, mark ready."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py finalize")
    p.add_argument("--cop", required=True)
    p.add_argument("--branch", required=True)
    p.add_argument("--base-sha", required=True)
    p.add_argument("--pr-url", required=True)
    p.add_argument("--backend", required=True)
    p.add_argument("--backend-label", required=True)
    p.add_argument("--model-label", required=True)
    p.add_argument("--mode", required=True)
    p.add_argument("--issue-number", default="")
    p.add_argument("--repo", required=True)
    p.add_argument("--run-url", required=True)
    p.add_argument("--run-number", required=True)
    p.add_argument("--tokens", required=True)
    p.add_argument("--code-bugs", default="0",
                   help="Number of CODE BUG examples from pre-diagnostic")
    opts = p.parse_args(args)

    scope_report_file = _env_path("AGENT_SCOPE_REPORT_FILE")
    pr_body_file = _env_path("PR_BODY_FILE")

    # 1. Check for changes
    diff_check = _git("diff", "--quiet", check=False)
    head_sha = _git("rev-parse", "HEAD").stdout.strip()
    has_changes = diff_check.returncode != 0 or head_sha != opts.base_sha

    if not has_changes:
        _log("No changes made")
        _close_pr_no_changes(
            opts.pr_url, opts.cop, opts.backend_label, opts.model_label,
            opts.mode, opts.run_url, opts.issue_number, opts.repo,
        )
        _output("result", "no_changes")
        _output("has_pr", "false")
        return 0

    _log("Changes detected on claimed branch")
    r = _git("log", "--oneline", f"{opts.base_sha}..HEAD", check=False)
    if r.stdout.strip():
        _log(r.stdout.rstrip())
    r = _git("diff", "--stat", check=False)
    if r.stdout.strip():
        _log(r.stdout.rstrip())

    # 2. Validate scope
    r = _run([
        sys.executable, str(SCRIPTS_DIR / "validate_agent_changes.py"),
        "--repo-root", str(REPO_ROOT),
        "--base-ref", opts.base_sha,
        "--profile", "agent-cop-fix",
        "--report-out", str(scope_report_file),
    ])
    scope_valid = False
    for line in r.stdout.strip().splitlines():
        if line.startswith("valid="):
            scope_valid = line.split("=", 1)[1] == "true"

    if not scope_valid:
        scope_report = read_file_or(scope_report_file)
        _close_pr_rejected(
            opts.pr_url, opts.cop, opts.issue_number,
            opts.repo, opts.run_url, scope_report,
        )
        _output("result", "rejected")
        _output("has_pr", "false")
        return 0

    # 3. Configure git for push
    _run([
        sys.executable, str(SCRIPTS_DIR / "workflow_git.py"),
        "configure",
        "--repo", opts.repo,
        "--unset-extraheader",
    ])

    # 4. Auto-format changed Rust files
    r = _git("diff", "--name-only", opts.base_sha, "--", "*.rs", check=False)
    rust_files = [f for f in r.stdout.strip().splitlines() if f.strip().endswith(".rs")]
    if rust_files:
        _run(["cargo", "fmt", "--"] + rust_files)

    # 5. Reject vacuous offense fixture edits
    _run(["cargo", "test", "--test", "integration", "offense_fixtures_have_no_unannotated_blocks"])

    # 6. Commit formatting changes
    mode_note = f" ({opts.mode})" if opts.mode not in ("fix",) else ""
    diff_check = _git("diff", "--quiet", check=False)
    if diff_check.returncode != 0:
        _git("add", "-u")
        _git("add", "src/", "tests/", "bench/", "scripts/", check=False)
        _git("commit", "-m", f"Fix {opts.cop}: agent-generated fix{mode_note} ({opts.backend})")

    # 6b. Re-validate scope after formatting commit (git add bench/ can
    #     sweep in stray gitlinks from cloned corpus repos).
    r = _run([
        sys.executable, str(SCRIPTS_DIR / "validate_agent_changes.py"),
        "--repo-root", str(REPO_ROOT),
        "--base-ref", opts.base_sha,
        "--profile", "agent-cop-fix",
        "--report-out", str(scope_report_file),
    ])
    post_scope_valid = False
    for line in r.stdout.strip().splitlines():
        if line.startswith("valid="):
            post_scope_valid = line.split("=", 1)[1] == "true"
    if not post_scope_valid:
        scope_report = read_file_or(scope_report_file)
        _log(f"Post-format scope check failed:\n{scope_report}")
        _close_pr_rejected(
            opts.pr_url, opts.cop, opts.issue_number,
            opts.repo, opts.run_url, scope_report,
        )
        _output("result", "rejected")
        _output("has_pr", "false")
        _output("file_guard_valid", "false")
        return 0

    # 7. Push + promote
    _git("push", "origin", f"HEAD:{opts.branch}", "--force")
    pushed_sha = _git("rev-parse", "HEAD").stdout.strip()
    r = _run([
        sys.executable, str(SCRIPTS_DIR / "workflow_git.py"),
        "promote",
        "--repo", opts.repo,
        "--branch", opts.branch,
        "--message", f"Fix {opts.cop}: agent-generated fix{mode_note} ({opts.backend})",
        "--expected-sha", pushed_sha,
    ])
    promote_result = {}
    for line in r.stdout.strip().splitlines():
        if "=" in line:
            k, _, v = line.partition("=")
            promote_result[k] = v

    signed_sha = promote_result.get("signed_sha", "")
    parent_sha = promote_result.get("parent_sha", "")

    # 8. Check for empty PR after push
    r = _run_ok([
        "gh", "api", f"repos/{opts.repo}/compare/main...{signed_sha}",
        "--jq", "(.files | length) // 0",
    ])
    file_count = r.stdout.strip() if r.returncode == 0 else ""
    if file_count == "0":
        _log("Final PR diff is empty after replay/push")
        _close_pr_no_changes(
            opts.pr_url, opts.cop, opts.backend_label, opts.model_label,
            opts.mode, opts.run_url, opts.issue_number, opts.repo,
        )
        _output("result", "empty")
        _output("has_pr", "false")
        return 0

    # 8b. Detect docs-only changes (no real cop logic fix)
    docs_only = _is_docs_only_change(signed_sha, opts.repo)
    had_code_bugs = int(opts.code_bugs or "0") > 0

    # If docs-only AND pre-diagnostic reported CODE BUGs, the agent gave up on
    # real fixes. Close the PR instead of merging — doc-only commits add noise
    # without closing the FP/FN gap.
    if docs_only and had_code_bugs:
        _log("Docs-only change but pre-diagnostic had CODE BUGs — closing PR")
        _close_pr_no_changes(
            opts.pr_url, opts.cop, opts.backend_label, opts.model_label,
            opts.mode, opts.run_url, opts.issue_number, opts.repo,
        )
        _output("result", "docs_only_rejected")
        _output("has_pr", "false")
        return 0

    if docs_only:
        _log("Docs-only change (config-only task) — will merge documentation but keep issue open as blocked")

    # 9. Build and update PR body
    body = _build_final_pr_body(
        opts.cop, opts.mode, opts.backend_label, opts.model_label,
        opts.run_url, opts.run_number, opts.issue_number, opts.tokens,
        signed_sha, parent_sha, opts.repo,
        docs_only=docs_only,
    )
    write_and_read(pr_body_file, body)
    _run(["gh", "pr", "edit", opts.pr_url, "--body-file", str(pr_body_file)])

    # 10. Mark PR ready + auto-merge
    _run(["gh", "pr", "ready", opts.pr_url])
    _log(f"PR ready: {opts.pr_url}")
    _run(["gh", "pr", "merge", opts.pr_url, "--auto", "--squash", "--delete-branch"])

    # 11. If docs-only, mark issue blocked (don't close it — the gap is still open)
    if docs_only and opts.issue_number:
        body = (
            f"Agent investigated `{opts.cop}` and documented findings, "
            f"but no cop logic was changed.\n\n"
            f"- Backend: `{opts.backend_label}`\n"
            f"- Model: `{opts.model_label}`\n"
            f"- Mode: `{opts.mode}`\n"
            f"- Run: {opts.run_url}\n\n"
            f"The FP/FN gap is likely caused by file-discovery or config differences, "
            f"not a cop detection bug. Documentation PR was merged. "
            f"Marking as blocked for manual investigation.\n"
        )
        claim_body = _env_path("CLAIM_BODY_FILE")
        write_and_read(claim_body, body)
        _run_ok(["gh", "issue", "comment", opts.issue_number, "--repo", opts.repo,
                 "--body-file", str(claim_body)])
        _run_ok([
            "gh", "issue", "edit", opts.issue_number, "--repo", opts.repo,
            "--remove-label", "state:pr-open,state:backlog",
            "--add-label", "state:blocked",
        ])

    _output("result", "docs_only" if docs_only else "success")
    _output("has_pr", "true")
    _output("pr_url", opts.pr_url)
    return 0


# ── cleanup-failure ─────────────────────────────────────────────────────

def cmd_cleanup_failure(args: list[str]) -> int:
    """Close draft PR and comment on linked issue when the workflow fails."""
    import argparse

    p = argparse.ArgumentParser(prog="cop_fix_lifecycle.py cleanup-failure")
    p.add_argument("--cop", required=True)
    p.add_argument("--pr-url", default="")
    p.add_argument("--issue-number", default="")
    p.add_argument("--repo", required=True)
    p.add_argument("--backend-label", default="n/a")
    p.add_argument("--model-label", default="n/a")
    p.add_argument("--mode", default="fix")
    p.add_argument("--run-url", required=True)
    p.add_argument("--file-guard-valid", default="")
    opts = p.parse_args(args)

    claim_body = _env_path("CLAIM_BODY_FILE")
    pr_closed = False
    head_ref = ""

    # If scope validation explicitly failed, the reject step already closed the PR
    if opts.file_guard_valid == "false":
        return 0

    if opts.pr_url:
        pr_view = _run_ok([
            "gh", "pr", "view", opts.pr_url,
            "--repo", opts.repo,
            "--json", "headRefName",
        ])
        if pr_view.returncode == 0:
            try:
                head_ref = json.loads(pr_view.stdout).get("headRefName", "")
            except json.JSONDecodeError:
                _warning(f"Failed to parse PR metadata for cleanup: {opts.pr_url}")
        else:
            _warn_best_effort_failure("Read PR metadata for cleanup", pr_view)

        pr_close = _warn_best_effort_failure(
            "Close failed draft PR",
            _run_ok([
                "gh", "pr", "close", opts.pr_url,
                "--repo", opts.repo,
                "--comment", f"Agent failed. See run: {opts.run_url}",
            ]),
        )
        pr_closed = pr_close.returncode == 0

        if pr_closed and head_ref:
            _warn_best_effort_failure(
                f"Delete failed branch `{head_ref}`",
                _run_ok([
                    "gh", "api", "-X", "DELETE",
                    f"repos/{opts.repo}/git/refs/heads/{head_ref}",
                ]),
            )

    if opts.issue_number:
        if opts.pr_url:
            if pr_closed:
                body = (
                    f"Agent fix failed before producing a usable PR for `{opts.cop}`.\n\n"
                    f"- Backend: `{opts.backend_label}`\n"
                    f"- Model: `{opts.model_label}`\n"
                    f"- Mode: `{opts.mode}`\n"
                    f"- Run: {opts.run_url}\n\n"
                    f"The draft PR was closed automatically. "
                    f"See the workflow summary and uploaded artifacts for the agent result and recovery patch.\n"
                )
            else:
                body = (
                    f"Agent fix failed for `{opts.cop}`, but automatic cleanup could not close the draft PR.\n\n"
                    f"- Backend: `{opts.backend_label}`\n"
                    f"- Model: `{opts.model_label}`\n"
                    f"- Mode: `{opts.mode}`\n"
                    f"- Run: {opts.run_url}\n\n"
                    f"Manual cleanup may be required for the stale draft PR.\n"
                )
        else:
            body = (
                f"Agent fix failed before it could create a draft PR for `{opts.cop}`.\n\n"
                f"- Backend input: `{opts.backend_label}`\n"
                f"- Mode: `{opts.mode}`\n"
                f"- Run: {opts.run_url}\n\n"
                f"Review the failed workflow run for details.\n"
            )
        # Include agent findings so future retry runs see what was tried.
        findings = _read_agent_findings()
        if findings:
            body += (
                "\n<details>\n"
                "<summary>Agent findings (what was tried)</summary>\n\n"
                f"```\n{findings}\n```\n"
                "</details>\n"
            )
        write_and_read(claim_body, body)
        _warn_best_effort_failure(
            f"Comment on issue #{opts.issue_number}",
            _run_ok([
                "gh", "issue", "comment", opts.issue_number,
                "--repo", opts.repo,
                "--body-file", str(claim_body),
            ]),
        )
        if opts.pr_url and pr_closed:
            _warn_best_effort_failure(
                f"Move issue #{opts.issue_number} back to backlog",
                _run_ok([
                    "gh", "issue", "edit", opts.issue_number,
                    "--repo", opts.repo,
                    "--remove-label", "state:pr-open",
                    "--add-label", "state:backlog",
                ]),
            )

    return 0


# ── main ────────────────────────────────────────────────────────────────

COMMANDS = {
    "init": cmd_init,
    "select-backend": cmd_select_backend,
    "resolve-backend": cmd_resolve_backend,
    "skip-fixed": cmd_skip_fixed,
    "build-prompt": cmd_build_prompt,
    "claim-pr": cmd_claim_pr,
    "prepare-branch": cmd_prepare_branch,
    "snapshot": cmd_snapshot,
    "finalize": cmd_finalize,
    "cleanup-failure": cmd_cleanup_failure,
}


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
        cmds = ", ".join(sorted(COMMANDS))
        print(f"Usage: {sys.argv[0]} <command> [args...]\nCommands: {cmds}", file=sys.stderr)
        return 1
    return COMMANDS[sys.argv[1]](sys.argv[2:])


if __name__ == "__main__":
    raise SystemExit(main())
