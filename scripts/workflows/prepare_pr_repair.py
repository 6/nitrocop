#!/usr/bin/env python3
"""Prepare an automatic PR repair attempt from a failed Checks run.

Fetches the failed workflow run, classifies the failure as easy/hard/skip,
builds a concise repair prompt, and writes a deterministic verification script
that the workflow can rerun before pushing any changes.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import agent_runtime
from shared.corpus_artifacts import download_corpus_results as _download_corpus

ANSI_RE = re.compile(r"\x1B\[[0-?]*[ -/]*[@-~]")
ACTIONS_LOG_PREFIX_RE = re.compile(
    r"^[^\t]+\t[^\t]+\t\d{4}-\d{2}-\d{2}T[\d:.+-]+Z\s*"
)
FAILED_CONCLUSIONS = {"failure", "cancelled", "timed_out", "action_required", "startup_failure"}
SKIPPABLE_STEP_NAMES = {
    "Set up job",
    "Run actions/checkout@v4",
    "Run dtolnay/rust-toolchain@stable",
    "Run Swatinem/rust-cache@v2",
    "Run ruby/setup-ruby@v1",
    "Cache corpus bundle",
    "Install corpus gems",
    "Post Cache corpus bundle",
    "Post Run Swatinem/rust-cache@v2",
    "Post Run actions/checkout@v4",
    "Complete job",
}

PYTHON_WORKFLOW_STEPS = {
    "Validate corpus manifest",
    "No vendor include macros",
    "Python script tests",
}

# Steps that a cop-fix bot (which only changes src/cop/*.rs and test fixtures)
# could never have caused to fail. If ALL failed steps are in this set or
# match COP_FIX_UNRELATED_RE, the failure is an infra issue — skip repair.
COP_FIX_UNRELATED_STEPS = {
    "Validate corpus manifest",
    "No vendor include macros",
    "Gem builder tests",
}
COP_FIX_UNRELATED_RE = re.compile(r"^Python\b", re.IGNORECASE)

EASY_STEP_COMMANDS = {
    "Format": "cargo fmt --check",
    "Validate corpus manifest": "python3 bench/corpus/validate_manifest.py",
    "No vendor include macros": "bash scripts/workflows/check_no_vendor_include_macros.sh",
    "Gem builder tests": "ruby gem/test/gem_builder_test.rb",
    "Python script tests": "uv run pytest tests/python/ -v",
    "Clippy": "cargo clippy --profile ci -- -D warnings",
    "Build": "cargo build --profile ci",
    "Build release binary": "cargo build --release",
    "Compile tests": "cargo test --no-run",
    "NodePattern verifier": "cargo test verifier -- --nocapture",
    "Config audit": "cargo test config_audit -- --nocapture",
    "Prism pitfalls": "cargo test prism_pitfalls -- --nocapture",
    "File discovery sync": "cargo test file_discovery_sync -- --nocapture",
    "Test": "cargo test",
}

HARD_STEP_COMMANDS = {
    "Check cops against corpus baseline": (
        "python3 scripts/dispatch_cops.py changed --base origin/main --head HEAD > \"$REPAIR_CHANGED_COPS_FILE\"\n"
        "failed=0\n"
        "while IFS= read -r cop; do\n"
        "  [ -z \"$cop\" ] && continue\n"
        "  echo \"==============================\"\n"
        "  echo \"Checking: $cop (re-running against corpus)\"\n"
        "  echo \"==============================\"\n"
        "  if ! python3 scripts/check_cop.py \"$cop\" --verbose --rerun --clone; then\n"
        "    echo \"FAIL: $cop regression detected\"\n"
        "    failed=$((failed + 1))\n"
        "  fi\n"
        "done < \"$REPAIR_CHANGED_COPS_FILE\"\n"
        "test \"$failed\" -eq 0"
    ),
    "Run smoke test": "python3 scripts/corpus_smoke_test.py --binary target/release/nitrocop",
}


def strip_ansi(text: str) -> str:
    return ANSI_RE.sub("", text)


def strip_actions_log_prefix(line: str) -> str:
    return ACTIONS_LOG_PREFIX_RE.sub("", line, count=1).lstrip()


def is_actions_log_noise(line: str) -> bool:
    stripped = line.strip()
    if not stripped:
        return False
    noise_prefixes = (
        "[command]/usr/bin/git ",
        "Entering '",
        "Temporarily overriding HOME=",
        "Adding repository directory to the temporary git global config as a safe directory",
        "Cleaning up orphan processes",
    )
    if stripped.startswith(noise_prefixes):
        return True
    noise_exact = {
        "Post job cleanup.",
        "http.https://github.com/.extraheader",
    }
    if stripped in noise_exact:
        return True
    if stripped.startswith("git version "):
        return True
    if stripped.startswith("##[warning]Node.js "):
        return True
    if stripped.startswith("file:") and stripped.endswith("\tremote.origin.url"):
        return True
    return False


def normalize_log(text: str, max_lines: int = 80) -> str:
    lines = []
    for raw_line in text.splitlines():
        line = strip_actions_log_prefix(strip_ansi(raw_line).rstrip())
        if not line.strip():
            continue
        if is_actions_log_noise(line):
            continue
        lines.append(line)
    if len(lines) > max_lines:
        lines = ["... (truncated, showing last %d lines) ..." % max_lines] + lines[-max_lines:]
    return "\n".join(lines)


def run_gh(args: list[str], repo: str | None = None, check: bool = True) -> str:
    cmd = ["gh"] + args
    if repo:
        cmd += ["-R", repo]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if check and result.returncode != 0:
        raise subprocess.CalledProcessError(result.returncode, cmd, result.stdout, result.stderr)
    return result.stdout


def load_run(repo: str, run_id: str) -> dict:
    output = run_gh(
        [
            "run",
            "view",
            run_id,
            "--json",
            "databaseId,workflowName,headSha,event,conclusion,jobs,url,number,headBranch",
        ],
        repo=repo,
    )
    return json.loads(output)


def failed_steps(job: dict) -> list[str]:
    names: list[str] = []
    for step in job.get("steps", []):
        if step.get("conclusion") in FAILED_CONCLUSIONS:
            name = step.get("name", "").strip()
            if name:
                names.append(name)
    if names:
        return names
    if job.get("conclusion") in FAILED_CONCLUSIONS:
        return [job.get("name", "unknown job")]
    return []


def job_route(job: dict) -> str:
    name = job.get("name", "")
    steps = failed_steps(job)
    if not steps:
        return "skip"
    if "macos" in name.lower():
        return "skip"
    route = "skip"
    for step_name in steps:
        if _step_in_commands(step_name, HARD_STEP_COMMANDS):
            return "hard"
        if _step_in_commands(step_name, EASY_STEP_COMMANDS):
            route = "easy"
            continue
        if step_name.startswith("Post ") or step_name in SKIPPABLE_STEP_NAMES:
            continue
        return "hard"
    return route


def _normalize_step_name(step_name: str) -> str:
    """Strip shard suffixes like ' (shard 1/4)' from step names."""
    return re.sub(r"\s*\(shard \d+/\d+\)\s*$", "", step_name)


def _step_in_commands(step_name: str, commands: dict) -> bool:
    return step_name in commands or _normalize_step_name(step_name) in commands


def command_for_step(step_name: str) -> str | None:
    normalized = _normalize_step_name(step_name)
    if normalized in HARD_STEP_COMMANDS:
        return HARD_STEP_COMMANDS[normalized]
    if normalized in EASY_STEP_COMMANDS:
        return EASY_STEP_COMMANDS[normalized]
    return None


def classify_run(run: dict, fix_backend: str = "") -> dict:
    jobs = [job for job in run.get("jobs", []) if job.get("conclusion") in FAILED_CONCLUSIONS]
    commands: list[str] = []
    command_keys: set[str] = set()
    hard_jobs: list[dict] = []
    easy_jobs: list[dict] = []
    skip_jobs: list[dict] = []

    for job in jobs:
        route = job_route(job)
        job["repair_route"] = route
        job["failed_step_names"] = failed_steps(job)
        for step_name in job["failed_step_names"]:
            command = command_for_step(step_name)
            if command and command not in command_keys:
                command_keys.add(command)
                commands.append(command)
        if route == "hard":
            hard_jobs.append(job)
        elif route == "easy":
            easy_jobs.append(job)
        else:
            skip_jobs.append(job)

    # Check if ALL failures are in steps unrelated to cop changes.
    # If so, this is an infra issue the bot can't fix — skip repair.
    all_failed_step_names = {
        step_name
        for job in jobs
        for step_name in job.get("failed_step_names", [])
    }
    def _is_unrelated(step: str) -> bool:
        n = _normalize_step_name(step)
        return n in COP_FIX_UNRELATED_STEPS or n in SKIPPABLE_STEP_NAMES or bool(COP_FIX_UNRELATED_RE.match(n))

    unrelated_failures = {s for s in all_failed_step_names if _is_unrelated(s)}
    non_skippable_failures = all_failed_step_names - unrelated_failures
    if all_failed_step_names and not non_skippable_failures:
        route = "skip"
        backend = ""
        infra_names = sorted(all_failed_step_names - SKIPPABLE_STEP_NAMES)
        reasons = [f"All failures are in infra steps unrelated to cop changes: "
                    f"{', '.join(infra_names)}"]
        return {
            "route": route,
            "backend": backend,
            "guard_profile": "",
            "cop_check_failure": False,
            "jobs": jobs,
            "hard_jobs": [],
            "easy_jobs": [],
            "skip_jobs": jobs,
            "verification_commands": [],
            "reason": "; ".join(reasons),
        }

    # Default to the original fix backend if available, otherwise codex-hard
    default_backend = fix_backend or "codex-hard"

    if hard_jobs:
        route = "hard"
        backend = default_backend
    elif easy_jobs:
        route = "easy"
        backend = default_backend
    else:
        route = "skip"
        backend = ""

    reasons = []
    for job in hard_jobs + easy_jobs + skip_jobs:
        steps = ", ".join(job.get("failed_step_names", [])) or job.get("name", "unknown")
        reasons.append(f"{job.get('name', 'unknown')}: {steps}")

    return {
        "route": route,
        "backend": backend,
        "guard_profile": determine_guard_profile(route, jobs),
        "cop_check_failure": any(
            any(_normalize_step_name(s) in (
                "Check cops against corpus baseline",
                "Comment cop-check summary on PR",
            ) for s in job.get("failed_step_names", []))
            for job in jobs
        ),
        "jobs": jobs,
        "hard_jobs": hard_jobs,
        "easy_jobs": easy_jobs,
        "skip_jobs": skip_jobs,
        "verification_commands": commands,
        "reason": "; ".join(reasons) if reasons else "No repairable failed jobs",
    }


def determine_guard_profile(route: str, jobs: list[dict]) -> str:
    failed_step_names = {
        _normalize_step_name(step_name)
        for job in jobs
        for step_name in job.get("failed_step_names", [])
    }
    if route == "hard":
        if "Check cops against corpus baseline" in failed_step_names:
            return "repair-cop-check"
        return "repair-smoke"
    if route == "easy":
        if failed_step_names & PYTHON_WORKFLOW_STEPS:
            return "repair-python-workflow"
        return "repair-rust-test"
    return ""


def fetch_failed_log(repo: str, run_id: str, job_id: str) -> str:
    return run_gh(
        ["run", "view", run_id, "--job", job_id, "--log-failed"],
        repo=repo,
        check=False,
    )


def build_verification_script(commands: list[str]) -> str:
    parts = ["#!/usr/bin/env bash", "set -euo pipefail", ""]
    for idx, command in enumerate(commands, 1):
        parts.append(f"echo \"=== Verification {idx}/{len(commands)} ===\"")
        parts.append(command)
        parts.append("")
    return "\n".join(parts).rstrip() + "\n"


def excerpt_diff(diff_text: str, max_lines: int = 220) -> str:
    lines = diff_text.splitlines()
    if len(lines) > max_lines:
        lines = lines[:max_lines] + [f"... ({len(diff_text.splitlines()) - max_lines} more lines truncated)"]
    return "\n".join(lines)


def extract_cop_from_title(title: str) -> str | None:
    """Extract cop name from PR title like '[bot] Fix Style/FrozenStringLiteralComment'."""
    m = re.search(r"Fix\s+(\w+/\w+)", title)
    return m.group(1) if m else None


def build_variant_context(cop: str, binary: Path | None = None) -> str:
    """Build variant divergence context for the repair prompt.

    Includes the variant table, fix instructions, and a quick local
    spot-check showing example FP/FN for the worst variant style.

    Returns markdown section or empty string if no variant data.
    """
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from dispatch_cops import load_variant_data_for_cop

    variants = load_variant_data_for_cop(cop)
    diverging = [v for v in variants if v["fp"] + v["fn"] > 0]
    if not diverging:
        return ""

    lines = [
        "## Variant Style Regression",
        "",
        "**This repair was triggered by a variant style regression.** The cop's default config "
        "is correct, but non-default `EnforcedStyle` values produce extra FP/FN:",
        "",
        "| Style | Matches | FP | FN |",
        "|-------|--------:|---:|---:|",
    ]
    for v in sorted(diverging, key=lambda x: x["fp"] + x["fn"], reverse=True):
        lines.append(f"| {v['style_label']} | {v['matches']:,} | {v['fp']:,} | {v['fn']:,} |")

    # Include actual FP/FN code examples from the variant oracle data
    from dispatch_cops import _normalize_example

    max_examples = 5
    for v in sorted(diverging, key=lambda x: x["fp"] + x["fn"], reverse=True):
        fp_ex = v.get("fp_examples", [])
        fn_ex = v.get("fn_examples", [])
        if not fp_ex and not fn_ex:
            continue
        lines.extend(["", f"### Variant `{v['style_label']}` — corpus examples", ""])
        if fp_ex:
            lines.append("**False Positives** (nitrocop flags but RuboCop does not):\n")
            for ex in fp_ex[:max_examples]:
                loc, msg, src = _normalize_example(ex)
                lines.append(f"- `{loc}`")
                if msg:
                    lines.append(f"  Message: {msg}")
                if src:
                    lines.append("  ```ruby")
                    for s in src:
                        lines.append(f"  {s}")
                    lines.append("  ```")
            lines.append("")
        if fn_ex:
            lines.append("**False Negatives** (RuboCop flags but nitrocop misses):\n")
            for ex in fn_ex[:max_examples]:
                loc, msg, src = _normalize_example(ex)
                lines.append(f"- `{loc}`")
                if msg:
                    lines.append(f"  Message: {msg}")
                if src:
                    lines.append("  ```ruby")
                    for s in src:
                        lines.append(f"  {s}")
                    lines.append("  ```")
            lines.append("")

    # Run a quick spot-check for the worst variant to get concrete examples
    worst = sorted(diverging, key=lambda x: x["fp"] + x["fn"], reverse=True)[0]
    worst_style = worst["style_label"]
    lines.extend([
        "",
        f"### Worst variant: `{worst_style}` ({worst['fp']:,} FP, {worst['fn']:,} FN)",
        "",
        "To reproduce locally and see concrete examples:",
        "",
        "```bash",
        "# Quick spot-check: run nitrocop vs rubocop with the variant style",
        f"python3 scripts/check_cop.py {cop} --verbose --rerun --clone --sample 5 \\",
        f"  --style EnforcedStyle={worst_style}",
        "```",
        "",
    ])

    # If we have a binary, try running a quick comparison to get example output
    if binary and binary.exists():
        try:
            result = subprocess.run(
                [
                    sys.executable, "scripts/check_cop.py", cop,
                    "--verbose", "--rerun", "--clone", "--sample", "3",
                    "--style", f"EnforcedStyle={worst_style}",
                ],
                capture_output=True, text=True, timeout=120,
                env={**__import__("os").environ, "NITROCOP_BIN": str(binary)},
            )
            output = result.stdout + result.stderr
            # Extract the useful parts (repos with offenses, excess/missing counts)
            useful_lines = []
            for line in output.splitlines():
                if any(x in line for x in [
                    "Repos with offenses", "Excess", "Missing", "Results:",
                    "Expected", "Actual", "CI nitrocop",
                ]):
                    useful_lines.append(line)
            if useful_lines:
                lines.extend([
                    "### Spot-check output",
                    "",
                    "```",
                    *useful_lines[:20],
                    "```",
                    "",
                ])
        except (subprocess.TimeoutExpired, FileNotFoundError):
            pass

    lines.extend([
        "### How to fix",
        "",
        "1. Find the config read: look for `config.get_str(\"EnforcedStyle\", ...)` in the Rust source",
        "2. Check the RuboCop Ruby source for how each style value changes behavior",
        "3. Add fixture tests with `# nitrocop-config: EnforcedStyle: <value>` directive",
        f"4. Validate: `python3 scripts/check_cop.py {cop} --rerun --clone --sample 15 "
        f"--style EnforcedStyle={worst_style}`",
        "5. All variants must pass — the CI gate runs all variant styles automatically",
        "",
    ])
    return "\n".join(lines)


def prefetch_corpus_context(route: str) -> dict[str, str]:
    if route != "hard":
        return {}

    runtime_paths = agent_runtime.current_paths("agent-pr-repair")
    source_path, run_id, head_sha = _download_corpus()
    target_path = Path(runtime_paths["REPAIR_CORPUS_FILE"])
    shutil.copy2(source_path, target_path)
    return {
        "path": str(target_path),
        "run_id": str(run_id),
        "head_sha": head_sha,
    }


def build_prompt(
    run: dict,
    classification: dict,
    pr_meta: dict,
    diff_stat: str,
    diff_text: str,
    extra_context: str,
    corpus_context: dict[str, dict[str, str]] | None = None,
    variant_context: str = "",
) -> str:
    route = classification["route"]
    backend = classification["backend"] or "none"
    backend_label_map = {
        "codex-normal": "codex / normal",
        "codex-hard": "codex / hard",
        "claude-normal": "claude / normal",
        "claude-hard": "claude / hard",
        "claude-oauth-normal": "claude-oauth / normal",
        "claude-oauth-hard": "claude-oauth / hard",
        "minimax": "minimax / normal",
    }
    backend_label = backend_label_map.get(backend, backend)
    lines = [
        f"# PR Repair Task: PR #{pr_meta['number']}",
        "",
        "Before making changes, read `docs/agent-ci.md`.",
        "",
        "Repair the existing PR branch in place. Do not create a new branch or PR.",
        "Preserve the intent of the current PR and make the smallest changes needed to fix the failing checks.",
        "Do not repair this PR by reverting it back to `origin/main`, deleting the whole diff, or otherwise turning it into an empty/no-op PR.",
        "If the only plausible fix is a full revert of the PR, stop and explain that clearly instead of performing the revert.",
        "Do not edit unrelated files or do broad cleanup.",
        "",
        "## Context",
        "",
        f"- PR: #{pr_meta['number']} {pr_meta['title']}",
        f"- Branch: `{pr_meta['headRefName']}`",
        f"- Checks run: #{run.get('number', '?')} ({run.get('workflowName', 'Checks')})",
        f"- Route: `{route}`",
        f"- Selected backend: `{backend_label}`",
        f"- Failure summary: {classification['reason']}",
        "",
        "## Required Outcome",
        "",
        "Fix the currently failing checks shown below.",
        "Before finishing, run the targeted validations needed to make the workflow's final verification likely to pass.",
        "",
        "## Required Local Verification",
        "",
        "The workflow will rerun these commands before pushing. Your changes should make them pass:",
        "",
        "```bash",
    ]
    for command in classification["verification_commands"]:
        lines.append(command)
        lines.append("")
    lines += [
        "```",
        "",
        "## Current PR Diff Stat",
        "",
        "```",
        diff_stat.strip() or "(no diff stat available)",
        "```",
        "",
        "## Current PR Diff Excerpt",
        "",
        "```diff",
        excerpt_diff(diff_text),
        "```",
        "",
    ]

    if corpus_context:
        lines.extend([
            "## Local Corpus Context",
            "",
            "These corpus oracle artifacts are already downloaded locally by the workflow.",
            "Prefer these local files over re-downloading the same corpus data from GitHub Actions.",
            "If you still need GitHub metadata for debugging, a read-only token is available in `GH_TOKEN`.",
            "",
        ])
        corpus_path = corpus_context.get("path")
        if corpus_path:
            lines.append(
                f"- Corpus JSON (matches the PR `cop-check` gate): `{corpus_path}` "
                f"(corpus oracle run #{corpus_context['run_id']})"
            )
        lines.extend([
            "",
            "Use these files directly with the repo scripts when you need corpus context.",
            "",
            "```bash",
        ])
        if corpus_path:
            lines.append(
                f"python3 scripts/investigate_cop.py Department/CopName --input {corpus_path} --context"
            )
            lines.append(
                f"python3 scripts/check_cop.py Department/CopName --input {corpus_path} --verbose --rerun --clone"
            )
        lines.extend([
            "```",
            "",
        ])

    lines.extend([
        "## Failed Checks Packet",
        "",
    ])

    for job in classification["jobs"]:
        lines.append(f"### {job.get('name', 'unknown job')}")
        lines.append("")
        lines.append(f"- Route: `{job.get('repair_route', 'skip')}`")
        lines.append(f"- Failed steps: {', '.join(job.get('failed_step_names', [])) or '(unknown)'}")
        log_text = normalize_log(job.get("failed_log", ""))
        if log_text:
            lines.extend(["", "```text", log_text, "```"])
        lines.append("")

    if extra_context.strip():
        lines.extend([
            "## Additional Instructions",
            "",
            extra_context.strip(),
            "",
        ])

    if variant_context:
        lines.append(variant_context)

    lines.extend([
        "## Constraints",
        "",
        "- Keep the fix scoped to the failing checks.",
        "- Reuse the existing PR branch and existing tests where possible.",
        "- Prefer the minimal patch that makes the deterministic verification pass.",
        "- A full revert to `origin/main` or an empty PR is treated as a failed repair, not a success.",
        "- If a fix is blocked by missing context, explain that clearly in the final message.",
        "- Do not build a second binary from `origin/main` — use the pre-computed diagnosis packet for baseline data.",
        "- Do not run `check_cop.py --rerun --clone` as your final verification step — it takes 20+ min and will likely time out. Use targeted spot-checks on specific repos instead. The workflow runs the full gate after you exit.",
        "- If you cannot fix the issue, your final message is posted to the PR/issue for future agents. "
        "Include: (1) what you changed and why it didn't work, (2) exact error messages or regression numbers, "
        "(3) why the approach failed, (4) what a correct fix would need. Be specific enough that the next "
        "agent won't repeat your work.",
        "",
    ])
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description="Prepare an automatic PR repair task")
    parser.add_argument("--repo", required=True, help="owner/repo")
    parser.add_argument("--run-id", required=True, help="Failed Checks workflow run ID")
    parser.add_argument("--pr-number", required=True, help="Target PR number")
    parser.add_argument("--pr-title", required=True, help="Target PR title")
    parser.add_argument("--head-branch", required=True, help="Target PR branch")
    parser.add_argument("--diff-stat", type=Path, required=True, help="Path to diff stat text")
    parser.add_argument("--diff", type=Path, required=True, help="Path to PR diff")
    parser.add_argument("--prompt-out", type=Path, required=True, help="Output markdown prompt")
    parser.add_argument("--verify-out", type=Path, required=True, help="Output shell script")
    parser.add_argument("--json-out", type=Path, required=True, help="Output JSON metadata")
    parser.add_argument(
        "--backend-override",
        choices=[
            "auto",
            "minimax",
            "codex-normal",
            "codex-hard",
            "claude-normal",
            "claude-hard",
            "claude-oauth-normal",
            "claude-oauth-hard",
        ],
        default="auto",
    )
    parser.add_argument("--fix-backend", default="", help="Original fix backend from PR model:* label")
    parser.add_argument("--extra-context", default="", help="Additional human instructions")
    args = parser.parse_args()

    run = load_run(args.repo, args.run_id)
    classification = classify_run(run, fix_backend=args.fix_backend)
    if args.backend_override != "auto" and classification["route"] != "skip":
        classification["backend"] = args.backend_override

    for job in classification["jobs"]:
        job["failed_log"] = fetch_failed_log(args.repo, args.run_id, str(job.get("databaseId", "")))

    corpus_context = prefetch_corpus_context(classification["route"])

    # Build variant context for cop-check failures on cop-fix PRs
    vc = ""
    if classification.get("cop_check_failure"):
        cop = extract_cop_from_title(args.pr_title)
        if cop:
            print(f"Building variant context for {cop}...", file=sys.stderr)
            vc = build_variant_context(cop)
            if vc:
                print(f"  Included variant divergence data for {cop}", file=sys.stderr)

    pr_meta = {
        "number": args.pr_number,
        "title": args.pr_title,
        "headRefName": args.head_branch,
    }

    # When the repair was triggered by a variant style regression, the verify
    # script must also check variant styles.  Without this, a repair agent can
    # pass local verification by only fixing the default style while the broken
    # variant remains unverified — then the full CI cop-check-gate fails again.
    if vc:
        classification["verification_commands"] = [
            cmd.replace(
                'scripts/check_cop.py "$cop" --verbose --rerun --clone',
                'scripts/check_cop.py "$cop" --verbose --rerun --clone --check-variants',
            )
            for cmd in classification["verification_commands"]
        ]

    prompt = build_prompt(
        run=run,
        classification=classification,
        pr_meta=pr_meta,
        diff_stat=args.diff_stat.read_text() if args.diff_stat.exists() else "",
        diff_text=args.diff.read_text() if args.diff.exists() else "",
        extra_context=args.extra_context,
        corpus_context=corpus_context,
        variant_context=vc,
    )
    verify_script = build_verification_script(classification["verification_commands"])

    args.prompt_out.write_text(prompt)
    args.verify_out.write_text(verify_script)
    args.verify_out.chmod(0o755)

    metadata = {
        "route": classification["route"],
        "backend": classification["backend"],
        "guard_profile": classification["guard_profile"],
        "cop_check_failure": classification["cop_check_failure"],
        "reason": classification["reason"],
        "verification_commands": classification["verification_commands"],
        "corpus_context": corpus_context,
        "failed_jobs": [
            {
                "name": job.get("name"),
                "route": job.get("repair_route"),
                "failed_steps": job.get("failed_step_names", []),
                "url": job.get("url"),
            }
            for job in classification["jobs"]
        ],
    }
    args.json_out.write_text(json.dumps(metadata, indent=2) + "\n")

    print(f"route={classification['route']}")
    print(f"backend={classification['backend']}")
    print(f"guard_profile={classification['guard_profile']}")
    print(f"cop_check_failure={'true' if classification['cop_check_failure'] else 'false'}")
    print(f"reason={classification['reason']}")
    print(f"failed_jobs={len(classification['jobs'])}")
    if "path" in corpus_context:
        print(f"corpus={corpus_context['path']}")


if __name__ == "__main__":
    main()
