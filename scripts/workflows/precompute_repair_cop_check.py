#!/usr/bin/env python3
"""Precompute local changed-cop corpus diagnostics for PR repair."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_OFFENSE_RE = re.compile(r"^\s*\d+\s+([^\s]+)\s*$")


def tail_lines(text: str, max_lines: int = 220) -> str:
    lines = [line.rstrip() for line in text.splitlines()]
    if len(lines) <= max_lines:
        return "\n".join(lines)
    kept = lines[-max_lines:]
    return "\n".join(
        [
            f"... (truncated, showing last {max_lines} of {len(lines)} lines) ...",
            *kept,
        ]
    )


def extract_top_repo_ids(output: str, limit: int = 5) -> list[str]:
    repo_ids: list[str] = []
    in_repo_block = False
    for line in output.splitlines():
        stripped = line.strip()
        if stripped.startswith("Repos with offenses "):
            in_repo_block = True
            continue
        if not in_repo_block:
            continue
        if not stripped:
            break
        if stripped.startswith("... and "):
            break
        match = REPO_OFFENSE_RE.match(line)
        if match:
            repo_ids.append(match.group(1))
            if len(repo_ids) >= limit:
                break
    return repo_ids


def used_batch_mode(output: str) -> bool:
    return "used batch --corpus-check mode" in output


def normalize_example(ex) -> tuple[str, str, list[str] | None]:
    if isinstance(ex, dict):
        return ex.get("loc", ""), ex.get("msg", ""), ex.get("src")
    return ex, "", None


def parse_example_loc(loc: str) -> tuple[str, str, int] | None:
    if ": " not in loc:
        return None
    repo_id, rest = loc.split(": ", 1)
    last_colon = rest.rfind(":")
    if last_colon < 0:
        return None
    filepath = rest[:last_colon]
    try:
        line = int(rest[last_colon + 1:])
    except ValueError:
        return None
    return repo_id, filepath, line


def top_repo_breakdown(
    repo_breakdown: dict[str, dict[str, int]],
    *,
    kind: str,
    limit: int = 3,
) -> list[tuple[str, int]]:
    repos = [
        (repo_id, counts.get(kind, 0))
        for repo_id, counts in repo_breakdown.items()
        if counts.get(kind, 0) > 0
    ]
    repos.sort(key=lambda item: (-item[1], item[0]))
    return repos[:limit]


def select_examples_for_kind(
    examples: list,
    *,
    preferred_repos: list[str],
    limit: int = 3,
) -> list[tuple[str, str, list[str] | None]]:
    preferred: list[tuple[str, str, list[str] | None]] = []
    seen: set[str] = set()
    repo_priority = {repo_id: idx for idx, repo_id in enumerate(preferred_repos)}

    parsed_examples: list[tuple[int, str, str, str, list[str] | None]] = []
    for ex in examples:
        loc, msg, src = normalize_example(ex)
        parsed = parse_example_loc(loc)
        if parsed is None:
            priority = len(preferred_repos)
        else:
            priority = repo_priority.get(parsed[0], len(preferred_repos))
        parsed_examples.append((priority, loc, msg, loc, src))

    parsed_examples.sort(key=lambda item: (item[0], item[1]))
    for _priority, _sort_loc, msg, loc, src in parsed_examples:
        if not loc or loc in seen:
            continue
        seen.add(loc)
        preferred.append((loc, msg, src))
        if len(preferred) >= limit:
            break
    return preferred


def render_example_section(
    title: str,
    examples: list[tuple[str, str, list[str] | None]],
) -> list[str]:
    if not examples:
        return []
    lines = [title]
    for loc, msg, src in examples:
        msg_suffix = f" — {msg}" if msg else ""
        lines.append(f"- `{loc}`{msg_suffix}")
        if src:
            lines.append("```ruby")
            lines.extend(src[:8])
            lines.append("```")
    lines.append("")
    return lines


def render_oracle_context(
    cop: str,
    *,
    standard_corpus: Path | None,
    oracle_by_cop: dict[str, dict],
    oracle_repo_breakdown: dict[str, dict[str, dict[str, int]]],
) -> list[str]:
    if standard_corpus is None:
        return []
    cop_entry = oracle_by_cop.get(cop)
    if not cop_entry:
        return []

    lines = [
        "Oracle context from CI corpus artifact:",
        f"- Repos and exact examples: `python3 scripts/investigate_cop.py {cop} --input {standard_corpus} --fn-only --context --limit 10`",
    ]

    repo_breakdown = oracle_repo_breakdown.get(cop, {})
    top_fn = top_repo_breakdown(repo_breakdown, kind="fn")
    top_fp = top_repo_breakdown(repo_breakdown, kind="fp")
    if top_fn:
        lines.append("Oracle FN hotspots:")
        for repo_id, count in top_fn:
            lines.append(f"- `{repo_id}` ({count} FN)")
    if top_fp:
        lines.append("Oracle FP hotspots:")
        for repo_id, count in top_fp:
            lines.append(f"- `{repo_id}` ({count} FP)")
    if top_fn or top_fp:
        lines.append("")

    preferred_repos = [repo_id for repo_id, _count in top_fn] or [repo_id for repo_id, _count in top_fp]
    fn_examples = select_examples_for_kind(
        cop_entry.get("fn_examples", []),
        preferred_repos=preferred_repos,
    )
    fp_examples = select_examples_for_kind(
        cop_entry.get("fp_examples", []),
        preferred_repos=preferred_repos,
        limit=2,
    )
    lines.extend(render_example_section("Representative oracle FN examples:", fn_examples))
    lines.extend(render_example_section("Representative oracle FP examples:", fp_examples))
    return lines


def load_oracle_context(
    standard_corpus: Path | None,
) -> tuple[dict[str, dict], dict[str, dict[str, dict[str, int]]]]:
    if standard_corpus is None or not standard_corpus.exists():
        return {}, {}
    data = json.loads(standard_corpus.read_text())
    by_cop = {entry["cop"]: entry for entry in data.get("by_cop", [])}
    repo_breakdown: dict[str, dict[str, dict[str, int]]] = {}
    for repo_id, cops in data.get("by_repo_cop", {}).items():
        for cop, entry in cops.items():
            repo_breakdown.setdefault(cop, {})[repo_id] = {
                "fp": entry.get("fp", 0),
                "fn": entry.get("fn", 0),
            }
    return by_cop, repo_breakdown


def load_variant_examples(
    cop: str, corpus_path: Path | None,
) -> list[dict]:
    """Load variant FP/FN examples from the oracle style-variant-results.json.

    Returns a list of dicts with {style_label, fp_examples, fn_examples}
    for each variant that has divergence for this cop.
    """
    if corpus_path is None:
        return []
    # The variant results are cached alongside the corpus results
    variant_path = corpus_path.parent / corpus_path.name.replace(
        "corpus", "style-variant-results"
    )
    if not variant_path.exists():
        # Try the standard cache location
        import tempfile
        cache_dir = Path(tempfile.gettempdir()) / "nitrocop-corpus-cache"
        candidates = sorted(cache_dir.glob("style-variant-results-*.json"))
        if candidates:
            variant_path = candidates[-1]
        else:
            return []
    try:
        data = json.loads(variant_path.read_text())
    except (json.JSONDecodeError, OSError):
        return []
    results = []
    for batch in data.get("batches", []):
        for entry in batch.get("by_cop", []):
            if entry.get("cop") != cop:
                continue
            fp_examples = entry.get("fp_examples", [])
            fn_examples = entry.get("fn_examples", [])
            if fp_examples or fn_examples:
                results.append({
                    "style_label": entry.get("style_label", batch.get("name", "")),
                    "fp": entry.get("fp", 0),
                    "fn": entry.get("fn", 0),
                    "fp_examples": fp_examples[:5],
                    "fn_examples": fn_examples[:5],
                })
    return results


def render_variant_context(
    cop: str,
    *,
    standard_corpus: Path | None,
) -> list[str]:
    """Render variant FP/FN examples for a cop into the diagnosis packet."""
    variants = load_variant_examples(cop, standard_corpus)
    if not variants:
        return []
    lines = [
        "**Variant style examples** (from oracle):",
        "",
    ]
    for v in variants:
        lines.append(f"Style `{v['style_label']}` — {v['fp']} FP, {v['fn']} FN:")
        for ex in v["fp_examples"]:
            loc = ex.get("loc", str(ex)) if isinstance(ex, dict) else str(ex)
            lines.append(f"  - FP: `{loc}`")
        for ex in v["fn_examples"]:
            loc = ex.get("loc", str(ex)) if isinstance(ex, dict) else str(ex)
            lines.append(f"  - FN: `{loc}`")
        lines.append("")
    return lines


def render_start_here(
    cop: str,
    top_repos: list[str],
    *,
    standard_corpus: Path | None,
    corpus_dir: Path,
    batch_mode: bool,
) -> list[str]:
    lines = [
        "Start here:",
        f"- Re-run after edits: `python3 scripts/check_cop.py {cop} --verbose --rerun --clone`",
    ]
    if standard_corpus is not None:
        lines.append(
            f"- Baseline corpus context: `python3 scripts/investigate_cop.py {cop} --input {standard_corpus} --repos-only`"
        )
    if batch_mode and top_repos:
        lines.extend(
            [
                f"- Batch sanity check if counts look suspicious: `python3 scripts/check_cop.py {cop} --verbose --rerun --clone --no-batch`",
                "- This local packet used batch `--corpus-check`; compare 1-2 top repos in per-repo mode before inventing a full manual sweep.",
            ]
        )
    for repo_id in top_repos:
        lines.append(f"- Inspect repo: `{corpus_dir / repo_id}`")
    return lines


def render_packet(
    results: list[dict[str, object]],
    *,
    standard_corpus: Path | None = None,
    corpus_dir: Path = Path("vendor/corpus"),
    oracle_by_cop: dict[str, dict] | None = None,
    oracle_repo_breakdown: dict[str, dict[str, dict[str, int]]] | None = None,
) -> str:
    oracle_by_cop = oracle_by_cop or {}
    oracle_repo_breakdown = oracle_repo_breakdown or {}
    lines = [
        "",
        "## Local Cop-Check Diagnosis",
        "",
        "The workflow already reran the changed-cop corpus check locally before agent execution.",
        "Use this packet as the starting point instead of rediscovering the same corpus regression.",
        "Do NOT rebuild baseline binaries from `origin/main` or rerun the full corpus check yourself — this packet already contains that data.",
        "",
    ]

    if not results:
        lines.extend(
            [
                "No changed cops were detected for local corpus diagnosis.",
                "",
            ]
        )
        return "\n".join(lines)

    lines.extend(
        [
            "Changed cops:",
            *[f"- `{result['cop']}`" for result in results],
            "",
        ]
    )

    for result in results:
        top_repos = extract_top_repo_ids(str(result["output"]))
        lines.extend(
            [
                f"### {result['cop']}",
                "",
                *render_start_here(
                    str(result["cop"]),
                    top_repos,
                    standard_corpus=standard_corpus,
                    corpus_dir=corpus_dir,
                    batch_mode=used_batch_mode(str(result["output"])),
                ),
                "",
                *render_oracle_context(
                    str(result["cop"]),
                    standard_corpus=standard_corpus,
                    oracle_by_cop=oracle_by_cop,
                    oracle_repo_breakdown=oracle_repo_breakdown,
                ),
                *render_variant_context(
                    str(result["cop"]),
                    standard_corpus=standard_corpus,
                ),
                "```bash",
                result["command"],
                "```",
                "",
                f"Exit status: `{result['status']}`",
                "",
                "```text",
                result["output"],
                "```",
                "",
            ]
        )

    return "\n".join(lines).rstrip() + "\n"


def run_capture(cmd: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=str(cwd),
        capture_output=True,
        text=True,
        check=False,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Precompute changed-cop corpus diagnostics")
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    parser.add_argument("--changed-cops-out", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    standard_corpus = os.environ.get("REPAIR_CORPUS_FILE")
    standard_corpus_path = Path(standard_corpus) if standard_corpus else None
    oracle_by_cop, oracle_repo_breakdown = load_oracle_context(standard_corpus_path)
    changed_result = run_capture(
        [
            sys.executable,
            "scripts/dispatch_cops.py",
            "changed",
            "--base",
            "origin/main",
            "--head",
            "HEAD",
        ],
        repo_root,
    )

    results: list[dict[str, object]] = []
    if changed_result.returncode == 0:
        cops = [line.strip() for line in changed_result.stdout.splitlines() if line.strip()]
        args.changed_cops_out.parent.mkdir(parents=True, exist_ok=True)
        args.changed_cops_out.write_text("\n".join(cops) + ("\n" if cops else ""))
        for cop in cops:
            cmd = [
                sys.executable,
                "scripts/check_cop.py",
                cop,
                "--verbose",
                "--rerun",
                "--clone",
                "--sample", "15",
            ]
            result = run_capture(cmd, repo_root)
            results.append(
                {
                    "cop": cop,
                    "command": " ".join(cmd),
                    "status": result.returncode,
                    "output": tail_lines((result.stdout + result.stderr).strip()),
                }
            )
    else:
        args.changed_cops_out.parent.mkdir(parents=True, exist_ok=True)
        args.changed_cops_out.write_text("")
        results.append(
            {
                "cop": "(changed-cops detection failed)",
                "command": f"{sys.executable} scripts/dispatch_cops.py changed --base origin/main --head HEAD",
                "status": changed_result.returncode,
                "output": tail_lines((changed_result.stdout + changed_result.stderr).strip()),
            }
        )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        render_packet(
            results,
            standard_corpus=standard_corpus_path,
            corpus_dir=repo_root / "vendor" / "corpus",
            oracle_by_cop=oracle_by_cop,
            oracle_repo_breakdown=oracle_repo_breakdown,
        )
    )
    failed = sum(1 for result in results if isinstance(result["status"], int) and result["status"] != 0)
    print(f"changed_cops={sum(1 for result in results if result['cop'] != '(changed-cops detection failed)')}")
    print(f"failed_cops={failed}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
