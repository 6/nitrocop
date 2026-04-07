#!/usr/bin/env python3
from __future__ import annotations

"""Diff nitrocop vs RuboCop JSON results and produce a corpus report.

Usage:
    python3 bench/corpus/diff_results.py \
        --nitrocop-dir results/nitrocop \
        --rubocop-dir results/rubocop \
        --manifest bench/corpus/manifest.jsonl \
        --output-json corpus-results.json \
        --output-md corpus-results.md
"""

import argparse
import json
import math
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

# Cops that exist in nitrocop's registry but cannot be triggered under
# our corpus config (Ruby 4.0 / Rails 8.0 target). They are excluded from
# registered_cops counts so they don't inflate "no corpus data" numbers.
UNSUPPORTED_COPS = frozenset([
    "Lint/ItWithoutArgumentsInBlock",       # max Ruby 3.3 (it is a block param in 3.4+)
    "Lint/NonDeterministicRequireOrder",     # max Ruby 2.7 (Dir sorts since 3.0)
    "Lint/NumberedParameterAssignment",      # syntax error in Ruby 3.4+
    "Lint/UselessElseWithoutRescue",         # syntax error in Ruby 3.4+
    "Security/YAMLLoad",                    # max Ruby 3.0 (YAML.load safe in 3.1+)
])


def fmt_pct(rate: float) -> str:
    """Format rate as percentage, floored to 0.1% (never rounds up to 100%)."""
    return f"{math.floor(rate * 1000) / 10:.1f}%"


def trunc4(rate: float) -> float:
    """Truncate rate to 4 decimal places (never rounds up to 1.0)."""
    return math.floor(rate * 10000) / 10000


def strip_repo_prefix(filepath: str) -> str:
    """Strip the repos/<id>/ prefix to get a path relative to the repo root."""
    # Paths may look like: repos/mastodon__mastodon__c1f398a/app/models/user.rb
    # or /full/path/repos/mastodon__mastodon__c1f398a/app/models/user.rb
    # We want: app/models/user.rb
    parts = filepath.replace("\\", "/").split("/")
    # Find "repos" in the path and skip it + the repo id
    for i, part in enumerate(parts):
        if part == "repos" and i + 1 < len(parts):
            return "/".join(parts[i + 2:])
    return filepath


def read_err_snippet(json_path: Path, tool: str) -> str:
    """Read the .err file next to a .json file and return the first meaningful error line.
    Returns empty string if no .err file or no meaningful content."""
    err_path = json_path.with_suffix(".err")
    try:
        text = err_path.read_text(errors="replace").strip()
    except FileNotFoundError:
        return ""
    if not text:
        return ""
    # Return first non-trivial line (skip blanks, "N files inspected" summaries,
    # and rubocop progress dots)
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        # Skip rubocop progress indicators (lines of just dots/letters like "..CC.W.")
        if all(c in ".CWEF" for c in line):
            continue
        # Skip summary lines like "1234 files inspected, ..."
        if "files inspected" in line or "offenses detected" in line:
            continue
        # Truncate long lines
        if len(line) > 200:
            line = line[:200] + "..."
        return line
    return ""


def parse_nitrocop_json(path: Path) -> tuple[set, dict] | None:
    """Parse nitrocop JSON output. Format: {"offenses": [...]}
    Returns (offenses_set, messages_dict) or None if the file is missing/empty/unparseable.
    messages_dict maps (filepath, line, cop) -> message."""
    try:
        text = path.read_text()
    except FileNotFoundError:
        return None
    if not text.strip():
        return None
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        return None

    offenses = set()
    messages = {}
    for o in data.get("offenses", []):
        filepath = strip_repo_prefix(o.get("path", ""))
        line = o.get("line", 0)
        cop = o.get("cop_name", "")
        if filepath and cop:
            key = (filepath, line, cop)
            offenses.add(key)
            msg = o.get("message", "")
            if msg:
                messages[key] = msg
    return offenses, messages


def parse_rubocop_json(path: Path) -> tuple[set, dict, set, int, int] | None:
    """Parse RuboCop JSON output. Format: {"files": [{"path": ..., "offenses": [...]}]}
    Returns (offenses, messages, inspected_files, target_file_count, inspected_file_count)
    or None if the file is missing/empty/unparseable.
    messages maps (filepath, line, cop) -> message.
    inspected_files is the set of relative file paths that RuboCop actually reported on.
    This is needed because RuboCop silently drops files when its parser crashes mid-batch."""
    try:
        text = path.read_text()
    except FileNotFoundError:
        return None
    if not text.strip():
        return None
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        return None

    offenses = set()
    messages = {}
    inspected_files = set()
    zero_offense_files = set()
    for f in data.get("files", []):
        filepath = strip_repo_prefix(f.get("path", ""))
        if filepath:
            inspected_files.add(filepath)
            if not f.get("offenses"):
                zero_offense_files.add(filepath)
        for o in f.get("offenses", []):
            line = o.get("location", {}).get("line", 0)
            cop = o.get("cop_name", "")
            if filepath and cop:
                key = (filepath, line, cop)
                offenses.add(key)
                msg = o.get("message", "")
                if msg:
                    messages[key] = msg

    # Detect parser crashes: when inspected_file_count < target_file_count,
    # RuboCop's parser crashed mid-batch and dropped files. Files that appear
    # in the output with 0 offenses may have been listed but not actually
    # analyzed (the crash causes them to be emitted with empty offense lists).
    # Exclude these suspicious zero-offense files from the inspected set so
    # nitrocop's correct offenses on them don't count as false positives.
    summary = data.get("summary", {})
    target = summary.get("target_file_count", 0)
    inspected = summary.get("inspected_file_count", 0)
    if target > 0 and inspected < target and zero_offense_files:
        inspected_files -= zero_offense_files

    return offenses, messages, inspected_files, target, inspected


def load_manifest(path: Path) -> list:
    """Load JSONL manifest."""
    repos = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                repos.append(json.loads(line))
    return repos


def load_context(context_dir: Path | None, repo_id: str) -> dict:
    """Load context snippets for a repo. Returns {filepath:line -> {context: [...]}}.
    Returns empty dict if no context available."""
    if not context_dir:
        return {}
    path = context_dir / f"{repo_id}.json"
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def main():
    parser = argparse.ArgumentParser(description="Diff corpus oracle results")
    parser.add_argument("--nitrocop-dir", required=True, type=Path)
    parser.add_argument("--rubocop-dir", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output-json", required=True, type=Path)
    parser.add_argument("--output-md", required=True, type=Path)
    parser.add_argument("--cop-list", type=Path, help="File with one cop name per line (filter RuboCop to these)")
    parser.add_argument("--context-dir", type=Path, help="Directory with per-repo context JSON files from extract_context.py")
    parser.add_argument("--style-variant-results", type=Path, default=None,
                        help="Path to style-variant-results.json (from variant batch runs). "
                             "When present, cops with variant data get per-variant rows in the "
                             "Diverging Cops table.")
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    {r["id"] for r in manifest}

    # Load cop filter (only compare offenses from cops nitrocop knows about)
    covered_cops = None
    if args.cop_list and args.cop_list.exists():
        covered_cops = {line.strip() for line in args.cop_list.read_text().splitlines() if line.strip()}
        print(f"Filtering to {len(covered_cops)} covered cops", file=sys.stderr)

    # Collect all repo IDs that have results
    tc_files = {f.stem: f for f in args.nitrocop_dir.glob("*.json")} if args.nitrocop_dir.exists() else {}
    rc_files = {f.stem: f for f in args.rubocop_dir.glob("*.json")} if args.rubocop_dir.exists() else {}
    all_ids = sorted(set(tc_files.keys()) | set(rc_files.keys()))

    multi_repo = len(all_ids) > 1

    # Per-repo results
    repo_results = []
    by_cop_matches = defaultdict(int)
    by_cop_fp = defaultdict(int)  # nitrocop-only
    by_cop_fn = defaultdict(int)  # rubocop-only
    by_cop_fp_examples = defaultdict(list)  # (filepath, line) per cop
    by_cop_fn_examples = defaultdict(list)
    by_cop_unfiltered: dict[str, int] = {}  # nitrocop count BEFORE RuboCop file-filtering
    by_repo_cop = defaultdict(lambda: defaultdict(lambda: {"matches": 0, "fp": 0, "fn": 0, "nitro_unfiltered": 0}))
    total_matches = 0
    total_fp = 0
    total_fn = 0
    repos_perfect = 0
    repos_error = 0
    total_files = 0
    total_files_dropped = 0
    cop_src_counts: dict[str, int] = {}  # tracks context examples per cop for global cap
    warning_repos = []  # repos with partial RuboCop crashes (file drops)

    for repo_id in all_ids:
        tc_path = tc_files.get(repo_id)
        rc_path = rc_files.get(repo_id)

        if not tc_path or not rc_path:
            side = "nitrocop" if not tc_path else "rubocop"
            repo_results.append({
                "repo": repo_id,
                "status": "missing_results",
                "error_message": f"No {side} JSON output file",
                "match_rate": 0,
                "matches": 0,
                "fp": 0,
                "fn": 0,
            })
            repos_error += 1
            continue

        tc_result = parse_nitrocop_json(tc_path)
        rc_result = parse_rubocop_json(rc_path)

        # Detect crashed/empty output — don't compare against phantom zero offenses
        if tc_result is None or rc_result is None:
            side = "nitrocop" if tc_result is None else "rubocop"
            err_path = tc_path if tc_result is None else rc_path
            err_msg = read_err_snippet(err_path, side)
            repo_results.append({
                "repo": repo_id,
                "status": f"crashed_{side}",
                "error_message": err_msg,
                "match_rate": 0,
                "matches": 0,
                "fp": 0,
                "fn": 0,
            })
            repos_error += 1
            continue

        tc_offenses, tc_messages = tc_result
        rc_offenses, rc_messages, rc_inspected_files, rc_target, rc_inspected = rc_result
        total_files += len(rc_inspected_files)

        # Filter to covered cops only (drop offenses from cops nitrocop doesn't implement)
        if covered_cops is not None:
            tc_offenses = {o for o in tc_offenses if o[2] in covered_cops}
            rc_offenses = {o for o in rc_offenses if o[2] in covered_cops}

        # Only compare files RuboCop actually inspected. RuboCop silently drops
        # files when its parser crashes mid-batch, producing phantom FPs for every
        # nitrocop offense on those dropped files.
        #
        # Root cause: Prism::Translation::Parser crashes on files with invalid
        # multibyte regex escapes (e.g., /\x9F/ in jruby's test_regexp.rb).
        # The unrescued RegexpError kills the worker, and all subsequent files in
        # that batch are silently omitted from the JSON output. Only 2-3 files
        # actually crash, but ~1000 are lost as collateral.
        #
        # Alternatives considered:
        # - Exclude crashing files in baseline_rubocop.yml: too repo-specific,
        #   and new crashing files could appear in future corpus additions.
        # - Re-run RuboCop file-by-file on dropped files: would recover ~1000
        #   files but adds significant CI complexity and runtime. Worth doing
        #   if we need coverage on those files for tier decisions.
        # - Fix upstream in parser gem / Prism translation layer: the crash is
        #   in Parser::Builders::Default#static_regexp which doesn't rescue
        #   RegexpError. Not something we control.
        # Count unfiltered nitrocop offenses per-cop before file filtering.
        # check_cop.py doesn't run RuboCop, so it can't replicate the
        # file-filtering step. The unfiltered count is stored in the
        # artifact so check-cop can compare against it instead.
        for _, _, cop in tc_offenses:
            by_cop_unfiltered[cop] = by_cop_unfiltered.get(cop, 0) + 1
            if multi_repo:
                by_repo_cop[repo_id][cop]["nitro_unfiltered"] += 1

        if rc_inspected_files:
            tc_offenses = {o for o in tc_offenses if o[0] in rc_inspected_files}

        matches = tc_offenses & rc_offenses
        fp = tc_offenses - rc_offenses  # nitrocop-only (false positives)
        fn = rc_offenses - tc_offenses  # rubocop-only (false negatives)

        n_matches = len(matches)
        n_fp = len(fp)
        n_fn = len(fn)
        total_count = len(tc_offenses | rc_offenses)
        match_rate = n_matches / total_count if total_count > 0 else 1.0

        total_matches += n_matches
        total_fp += n_fp
        total_fn += n_fn

        if n_fp == 0 and n_fn == 0:
            repos_perfect += 1

        # Load source context for this repo (if available)
        repo_context = load_context(args.context_dir, repo_id) if args.context_dir else {}

        def _make_example(loc: str, msg: str, filepath: str, line: int, cop: str) -> dict | str:
            """Build an example entry with optional message and source context.
            Context is capped at SRC_CAP_PER_COP examples per cop to keep JSON size manageable."""
            SRC_CAP_PER_COP = 50
            ctx = None
            if repo_context:
                ctx = repo_context.get(f"{filepath}:{line}", {}).get("context")
                if ctx and cop_src_counts.get(cop, 0) >= SRC_CAP_PER_COP:
                    ctx = None  # over cap, drop context for this cop
            if msg or ctx:
                entry = {"loc": loc}
                if msg:
                    entry["msg"] = msg
                if ctx:
                    entry["src"] = ctx
                    cop_src_counts[cop] = cop_src_counts.get(cop, 0) + 1
                return entry
            return loc

        # Per-cop aggregation
        for _, _, cop in matches:
            by_cop_matches[cop] += 1
            if multi_repo:
                by_repo_cop[repo_id][cop]["matches"] += 1
        for filepath, line, cop in sorted(fp):
            by_cop_fp[cop] += 1
            key = (filepath, line, cop)
            loc = f"{repo_id}: {filepath}:{line}" if multi_repo else f"{filepath}:{line}"
            # FP = nitrocop fires but RuboCop doesn't → message comes from nitrocop
            msg = tc_messages.get(key, "")
            by_cop_fp_examples[cop].append(_make_example(loc, msg, filepath, line, cop))
            if multi_repo:
                by_repo_cop[repo_id][cop]["fp"] += 1
        for filepath, line, cop in sorted(fn):
            by_cop_fn[cop] += 1
            key = (filepath, line, cop)
            loc = f"{repo_id}: {filepath}:{line}" if multi_repo else f"{filepath}:{line}"
            # FN = RuboCop fires but nitrocop doesn't → message comes from RuboCop
            msg = rc_messages.get(key, "")
            by_cop_fn_examples[cop].append(_make_example(loc, msg, filepath, line, cop))
            if multi_repo:
                by_repo_cop[repo_id][cop]["fn"] += 1

        result = {
            "repo": repo_id,
            "status": "ok",
            "match_rate": trunc4(match_rate),
            "matches": n_matches,
            "fp": n_fp,
            "fn": n_fn,
            "nitrocop_total": len(tc_offenses),
            "rubocop_total": len(rc_offenses),
            "files_inspected": len(rc_inspected_files),
        }

        # Track partial RuboCop crashes (parser errors that drop files)
        files_dropped = rc_target - rc_inspected if rc_target > rc_inspected > 0 else 0
        if files_dropped > 0:
            total_files_dropped += files_dropped
            err_msg = read_err_snippet(rc_path, "rubocop")
            result["rubocop_files_dropped"] = files_dropped
            result["rubocop_target_files"] = rc_target
            result["rubocop_inspected_files"] = rc_inspected
            if err_msg:
                result["rubocop_error"] = err_msg
            warning_repos.append(result)

        repo_results.append(result)

    # Compute unique repo counts per cop (repos where RuboCop fires at least once)
    cop_repo_counts = defaultdict(int)
    cop_activity_repos = defaultdict(list)
    for repo_id, cops in by_repo_cop.items():
        for cop_name, stats in cops.items():
            if stats["matches"] + stats["fn"] > 0:
                cop_repo_counts[cop_name] += 1
                cop_activity_repos[cop_name].append(repo_id)

    # Build per-cop table. When a full cop list is available from `--list-cops`,
    # include zero-activity cops so downstream reports can distinguish
    # "perfect in corpus" from "never exercised by the corpus".
    observed_cops = set(by_cop_matches) | set(by_cop_fp) | set(by_cop_fn)
    all_cops = sorted(((covered_cops or set()) | observed_cops) - UNSUPPORTED_COPS)
    by_cop = []
    EXAMPLE_CAP = 100  # cap per-cop examples to keep JSON size manageable
    for cop in all_cops:
        m = by_cop_matches.get(cop, 0)
        fp = by_cop_fp.get(cop, 0)
        fn = by_cop_fn.get(cop, 0)
        total = m + fp + fn
        rate = m / total if total > 0 else 1.0
        diverging = fp + fn > 0
        exercised = total > 0
        entry = {
            "cop": cop,
            "matches": m,
            "fp": fp,
            "fn": fn,
            "rubocop_total": m + fn,
            "nitro_total_unfiltered": by_cop_unfiltered.get(cop, m + fp),
            "unique_repos": cop_repo_counts.get(cop, 0),
            "match_rate": trunc4(rate),
            "exercised": exercised,
            "perfect_match": exercised and not diverging,
            "diverging": diverging,
            "fp_examples": by_cop_fp_examples.get(cop, [])[:EXAMPLE_CAP],
            "fn_examples": by_cop_fn_examples.get(cop, [])[:EXAMPLE_CAP],
        }
        by_cop.append(entry)
    by_cop.sort(key=lambda x: x["fp"] + x["fn"], reverse=True)

    # Aggregate by department
    dept_stats = defaultdict(lambda: {
        "matches": 0,
        "fp": 0,
        "fn": 0,
        "cops": 0,
        "exercised_cops": 0,
        "perfect_cops": 0,
        "diverging_cops": 0,
        "inactive_cops": 0,
    })
    for c in by_cop:
        dept = c["cop"].split("/")[0]
        dept_stats[dept]["matches"] += c["matches"]
        dept_stats[dept]["fp"] += c["fp"]
        dept_stats[dept]["fn"] += c["fn"]
        dept_stats[dept]["cops"] += 1
        if c["diverging"]:
            dept_stats[dept]["diverging_cops"] += 1
        elif c["exercised"]:
            dept_stats[dept]["perfect_cops"] += 1
        else:
            dept_stats[dept]["inactive_cops"] += 1
        if c["exercised"]:
            dept_stats[dept]["exercised_cops"] += 1
    by_department = []
    for dept in sorted(dept_stats):
        s = dept_stats[dept]
        total = s["matches"] + s["fp"] + s["fn"]
        rate = s["matches"] / total if total > 0 else 1.0
        by_department.append({
            "department": dept,
            "matches": s["matches"],
            "fp": s["fp"],
            "fn": s["fn"],
            "match_rate": trunc4(rate),
            "cops": s["cops"],
            "exercised_cops": s["exercised_cops"],
            "perfect_cops": s["perfect_cops"],
            "diverging_cops": s["diverging_cops"],
            "inactive_cops": s["inactive_cops"],
        })

    registered_cops = len(by_cop)
    exercised_cops = sum(1 for c in by_cop if c["exercised"])
    perfect_cops = sum(1 for c in by_cop if c["perfect_match"])
    diverging_cops = sum(1 for c in by_cop if c["diverging"])
    inactive_cops = registered_cops - exercised_cops

    # Overall stats
    oracle_total = total_matches + total_fp + total_fn
    overall_rate = total_matches / oracle_total if oracle_total > 0 else 1.0

    # Load style-variant results if available (before JSON write so stats are embedded)
    variant_by_cop: dict[str, list[dict]] = {}  # cop -> [{style_label, matches, fp, fn}]
    if args.style_variant_results and args.style_variant_results.exists():
        try:
            variant_data = json.loads(args.style_variant_results.read_text())
            for batch in variant_data.get("batches", []):
                for cop_entry in batch.get("by_cop", []):
                    cop_name = cop_entry.get("cop", "")
                    if cop_name:
                        variant_by_cop.setdefault(cop_name, []).append({
                            "style_label": cop_entry.get("style_label", batch.get("name", "?")),
                            "matches": cop_entry.get("matches", 0),
                            "fp": cop_entry.get("fp", 0),
                            "fn": cop_entry.get("fn", 0),
                            "fp_examples": cop_entry.get("fp_examples", []),
                            "fn_examples": cop_entry.get("fn_examples", []),
                        })
        except (json.JSONDecodeError, KeyError) as e:
            print(f"Warning: could not load style-variant results: {e}", file=sys.stderr)

    # Compute variant-inclusive totals for summary/department sections
    variant_total_matches = total_matches
    variant_total_fp = total_fp
    variant_total_fn = total_fn
    variant_dept_stats: dict[str, dict[str, int]] = {}
    for cop_name, variants in variant_by_cop.items():
        dept = cop_name.split("/")[0]
        if dept not in variant_dept_stats:
            variant_dept_stats[dept] = {"matches": 0, "fp": 0, "fn": 0}
        for v in variants:
            variant_total_matches += v["matches"]
            variant_total_fp += v["fp"]
            variant_total_fn += v["fn"]
            variant_dept_stats[dept]["matches"] += v["matches"]
            variant_dept_stats[dept]["fp"] += v["fp"]
            variant_dept_stats[dept]["fn"] += v["fn"]
    variant_oracle_total = variant_total_matches + variant_total_fp + variant_total_fn
    variant_overall_rate = variant_total_matches / variant_oracle_total if variant_oracle_total > 0 else 1.0

    # ── Write JSON ──
    # NOTE: variant stats (variant_overall_match_rate, variant_match_rate per
    # department) are NOT embedded here. They are patched in by
    # patch_variant_stats.py AFTER the include-gated merge, so that variant
    # rates are computed from the final default FP/FN numbers.
    json_output = {
        "schema": 1,
        "run_date": datetime.now(timezone.utc).isoformat(),
        "baseline": {
            "rubocop": "1.84.2",
            "rubocop-rails": "2.34.3",
            "rubocop-performance": "1.26.1",
            "rubocop-rspec": "3.9.0",
            "rubocop-rspec_rails": "2.32.0",
            "rubocop-factory_bot": "2.28.0",
            "rubocop-rake": "0.7.1",
        },
        "summary": {
            "total_repos": len(all_ids),
            "repos_perfect": repos_perfect,
            "repos_error": repos_error,
            "repos_with_rubocop_warnings": len(warning_repos),
            "total_offenses_compared": oracle_total,
            "matches": total_matches,
            "fp": total_fp,
            "fn": total_fn,
            "registered_cops": registered_cops,
            "exercised_cops": exercised_cops,
            "perfect_cops": perfect_cops,
            "diverging_cops": diverging_cops,
            "inactive_cops": inactive_cops,
            "overall_match_rate": trunc4(overall_rate),
            "total_files_inspected": total_files,
            "rubocop_files_dropped": total_files_dropped,
        },
        "by_department": by_department,
        "by_cop": by_cop,  # all cops (gen_tiers.py needs the full list)
        "by_repo": repo_results,
        "cop_activity_repos": {
            cop: sorted(repos)
            for cop, repos in cop_activity_repos.items()
        },
        "by_repo_cop": {
            repo: {cop: stats for cop, stats in cops.items() if stats["fp"] + stats["fn"] > 0}
            for repo, cops in by_repo_cop.items()
            if any(s["fp"] + s["fn"] > 0 for s in cops.values())
        },
    }
    args.output_json.write_text(json.dumps(json_output, indent=2) + "\n")

    def _sanitize_for_md(s: str) -> str:
        """Replace C0 control chars with ASCII escape sequences.

        NUL bytes and other control characters in RuboCop messages (e.g. from
        symbols containing \\x00) break GitHub's markdown renderer entirely.
        """
        return "".join(
            repr(c)[1:-1] if ord(c) < 0x20 and c not in '\n\t' else c
            for c in s
        )

    def _format_example_md(ex) -> str:
        """Format an example for markdown (handles both string and dict format)."""
        if isinstance(ex, dict):
            loc = ex.get("loc", "")
            msg = ex.get("msg", "")
            msg = _sanitize_for_md(msg)
            return f"{loc}  [{msg}]" if msg else loc
        return _sanitize_for_md(ex)

    def _emit_examples(md_out: list[str], fp_list: list, fn_list: list, limit: int) -> None:
        """Append FP/FN example bullets to *md_out*."""
        if fp_list:
            for ex in fp_list[:limit]:
                md_out.append(f"- FP: `{_format_example_md(ex)}`")
            if len(fp_list) > limit:
                md_out.append(f"- ... and {len(fp_list) - limit:,} more FP")
        if fn_list:
            for ex in fn_list[:limit]:
                md_out.append(f"- FN: `{_format_example_md(ex)}`")
            if len(fn_list) > limit:
                md_out.append(f"- ... and {len(fn_list) - limit:,} more FN")
        md_out.append("")

    # ── Write Markdown ──
    md = []
    md.append("# Corpus Oracle Results")
    md.append("")
    md.append("> Auto-generated by the [corpus oracle workflow](../.github/workflows/corpus-oracle.yml).")
    md.append(f"> Last updated: {datetime.now(timezone.utc).strftime('%Y-%m-%d')}")
    md.append("")
    md.append(f"Compares nitrocop against RuboCop on {len(all_ids):,} open-source Ruby repos.")
    md.append("Every offense is compared by file path, line number, and cop name.")
    md.append("")

    md.append("## Summary")
    md.append("")
    md.append("| Metric | Value |")
    md.append("|--------|------:|")
    md.append(f"| Repos | {len(all_ids)} |")
    md.append(f"| Repos with 100% match | {repos_perfect} |")
    md.append(f"| Files inspected | {total_files:,} |")
    md.append(f"| Offenses compared | {oracle_total:,} |")
    md.append(f"| Matches (both agree) | {total_matches:,} |")
    md.append(f"| FP (nitrocop extra) | {total_fp:,} |")
    md.append(f"| FN (nitrocop missing) | {total_fn:,} |")
    md.append(f"| Registered cops | {registered_cops:,} |")
    md.append(f"| Cops with exact match | {perfect_cops:,} |")
    md.append(f"| Cops with divergence | {diverging_cops:,} |")
    md.append(f"| Cops with no corpus data | {inactive_cops:,} |")
    md.append(f"| **Match rate (default config)** | **{fmt_pct(overall_rate)}** |")
    if variant_by_cop:
        md.append(f"| **Match rate (all variants)** | **{fmt_pct(variant_overall_rate)}** |")
    if repos_error > 0 or warning_repos:
        md.append(f"| Repos with errors | {repos_error} |")
    if warning_repos:
        md.append(f"| Repos with RuboCop parser crashes | {len(warning_repos)} |")
        md.append(f"| RuboCop files dropped (parser crash) | {total_files_dropped:,} |")
    md.append("")

    # ── Department breakdown ──
    if by_department:
        md.append("## Department Breakdown")
        md.append("")
        if variant_by_cop:
            md.append("| Department | Total cops | Exact match | Diverging | No corpus data | Matches | FP | FN | Default variant % | All variants % |")
            md.append("|------------|-----------:|------------:|----------:|---------------:|--------:|---:|---:|---------:|---------------:|")
        else:
            md.append("| Department | Total cops | Exact match | Diverging | No corpus data | Matches | FP | FN | Match % |")
            md.append("|------------|-----------:|------------:|----------:|---------------:|--------:|---:|---:|--------:|")
        for d in by_department:
            total = d["matches"] + d["fp"] + d["fn"]
            pct = fmt_pct(d['match_rate']) if total > 0 else "N/A"
            row = (
                f"| {d['department']} | {d['cops']:,} | "
                f"{d['perfect_cops']:,} | {d['diverging_cops']:,} | {d['inactive_cops']:,} | "
                f"{d['matches']:,} | {d['fp']:,} | {d['fn']:,} | {pct}"
            )
            if variant_by_cop:
                vd = variant_dept_stats.get(d["department"], {"matches": 0, "fp": 0, "fn": 0})
                v_total = total + vd["matches"] + vd["fp"] + vd["fn"]
                v_matches = d["matches"] + vd["matches"]
                v_pct = fmt_pct(v_matches / v_total) if v_total > 0 else "N/A"
                row += f" | {v_pct} |"
            else:
                row += " |"
            md.append(row)
        md.append("")

    # ── Compute ok/error repo lists (used by multiple sections below) ──
    ok_repos = [r for r in repo_results if r["status"] == "ok"]
    err_repos = [r for r in repo_results if r["status"] != "ok"]

    # ── Top divergence contributor repos (by number of diverging cops) ──
    if multi_repo and by_repo_cop:
        repo_diverging_cops: dict[str, int] = {}
        repo_fp_cops: dict[str, int] = {}
        repo_fn_cops: dict[str, int] = {}
        for repo_id_key, cops in by_repo_cop.items():
            fp_cops = sum(1 for c in cops.values() if c["fp"] > 0)
            fn_cops = sum(1 for c in cops.values() if c["fn"] > 0)
            diverging = sum(1 for c in cops.values() if c["fp"] + c["fn"] > 0)
            if diverging > 0:
                repo_diverging_cops[repo_id_key] = diverging
                repo_fp_cops[repo_id_key] = fp_cops
                repo_fn_cops[repo_id_key] = fn_cops

        if repo_diverging_cops:
            top = sorted(repo_diverging_cops.items(), key=lambda x: x[1], reverse=True)[:20]
            md.append("## Top Divergence Contributors (repos with most diverging cops)")
            md.append("")
            md.append("| Repo | Diverging Cops | FP Cops | FN Cops | Total FP | Total FN |")
            md.append("|------|---------------:|--------:|--------:|---------:|---------:|")
            for repo_id_key, div_count in top:
                r = next((x for x in ok_repos if x["repo"] == repo_id_key), None)
                total_fp_count = r["fp"] if r else 0
                total_fn_count = r["fn"] if r else 0
                md.append(
                    f"| {repo_id_key} | {div_count} | "
                    f"{repo_fp_cops.get(repo_id_key, 0)} | {repo_fn_cops.get(repo_id_key, 0)} | "
                    f"{total_fp_count:,} | {total_fn_count:,} |"
                )
            md.append("")

    # ── RuboCop warnings (parser crashes, errors) ──
    warn_and_err = warning_repos + err_repos
    if warn_and_err:
        md.append("## RuboCop Warnings")
        md.append("")
        md.append(f"{len(warn_and_err)} repos had RuboCop issues (parser crashes or errors).")
        md.append("")
        md.append("| Repo | Issue | Files Dropped | Error |")
        md.append("|------|-------|--------------|-------|")
        for r in warning_repos:
            dropped = r.get("rubocop_files_dropped", 0)
            err = r.get("rubocop_error", "")
            err_cell = f"`{err}`" if err else ""
            md.append(f"| {r['repo']} | parser crash | {dropped:,} | {err_cell} |")
        for r in err_repos:
            err = r.get("error_message", "")
            err_cell = f"`{err}`" if err else ""
            md.append(f"| {r['repo']} | {r['status']} | all | {err_cell} |")
        md.append("")

    # ── Diverging cops with <details> for examples ──
    diverging = [c for c in by_cop if c["diverging"]]
    perfect_cop_list = [c for c in by_cop if c["perfect_match"]]

    # Cops that are perfect in default config but diverge in a variant
    variant_only_diverging = []
    if variant_by_cop:
        default_diverging_names = {c["cop"] for c in diverging}
        for cop_name, variants in sorted(variant_by_cop.items()):
            if cop_name not in default_diverging_names:
                if any(v["fp"] + v["fn"] > 0 for v in variants):
                    # Find the cop's default stats from by_cop
                    cop_entry = next((c for c in by_cop if c["cop"] == cop_name), None)
                    if cop_entry:
                        variant_only_diverging.append(cop_entry)

    all_diverging = diverging + variant_only_diverging
    if all_diverging:
        md.append("## Diverging Cops")
        md.append("")
        md.append(
            f"{len(diverging)} cops diverge from RuboCop on the corpus (default config). "
            f"{len(perfect_cop_list)} cops match RuboCop exactly. "
            f"{inactive_cops} cops have no corpus data."
        )
        if variant_only_diverging:
            md.append(
                f" {len(variant_only_diverging)} additional cops diverge only in non-default style variants."
            )
        md.append("")
        md.append("| Cop | Matches | FP | FN | Match % |")
        md.append("|-----|--------:|---:|---:|--------:|")
        for c in diverging:
            total = c["matches"] + c["fp"] + c["fn"]
            pct = fmt_pct(c['match_rate']) if total > 0 else "N/A"
            if c["cop"] in variant_by_cop:
                md.append(f"| {c['cop']} (default) | {c['matches']:,} | {c['fp']:,} | {c['fn']:,} | {pct} |")
                for v in variant_by_cop[c["cop"]]:
                    v_total = v["matches"] + v["fp"] + v["fn"]
                    if v_total == 0:
                        continue  # skip variants with no data (batch errored)
                    v_pct = fmt_pct(v["matches"] / v_total)
                    md.append(f"| ↳ {c['cop']} ({v['style_label']}) | {v['matches']:,} | {v['fp']:,} | {v['fn']:,} | {v_pct} |")
            else:
                md.append(f"| {c['cop']} | {c['matches']:,} | {c['fp']:,} | {c['fn']:,} | {pct} |")

        # Show cops that only diverge in non-default variants
        if variant_only_diverging:
            md.append("")
            md.append("**Variant-only divergence** (perfect in default config, diverge in non-default styles):")
            md.append("")
            md.append("| Cop | Matches | FP | FN | Match % |")
            md.append("|-----|--------:|---:|---:|--------:|")
            for c in sorted(variant_only_diverging, key=lambda x: sum(v["fp"] + v["fn"] for v in variant_by_cop.get(x["cop"], [])), reverse=True):
                total = c["matches"] + c["fp"] + c["fn"]
                pct = fmt_pct(c['match_rate']) if total > 0 else "N/A"
                md.append(f"| {c['cop']} (default) | {c['matches']:,} | {c['fp']:,} | {c['fn']:,} | {pct} |")
                for v in variant_by_cop.get(c["cop"], []):
                    v_total = v["matches"] + v["fp"] + v["fn"]
                    v_pct = fmt_pct(v["matches"] / v_total) if v_total > 0 else "N/A"
                    if v["fp"] + v["fn"] > 0:
                        md.append(f"| ↳ {c['cop']} ({v['style_label']}) | {v['matches']:,} | {v['fp']:,} | {v['fn']:,} | {v_pct} |")
        md.append("")

        # Expandable details per cop (show up to 3 examples in markdown; full list in JSON)
        MD_EXAMPLE_LIMIT = 3
        for c in all_diverging:
            cop_name = c["cop"]
            default_fp = c.get("fp_examples", [])
            default_fn = c.get("fn_examples", [])
            variants = variant_by_cop.get(cop_name, [])
            variant_fp = [ex for v in variants for ex in v.get("fp_examples", [])]
            variant_fn = [ex for v in variants for ex in v.get("fn_examples", [])]
            if not default_fp and not default_fn and not variant_fp and not variant_fn:
                continue
            total = c["matches"] + c["fp"] + c["fn"]
            pct = fmt_pct(c['match_rate']) if total > 0 else "N/A"
            all_fp = c["fp"] + sum(v["fp"] for v in variants)
            all_fn = c["fn"] + sum(v["fn"] for v in variants)
            md.append("<details>")
            md.append(f"<summary><strong>{cop_name}</strong> — {c['matches']:,} matches, {all_fp:,} FP, {all_fn:,} FN ({pct})</summary>")
            md.append("")
            if default_fp or default_fn:
                if c["fp"] + c["fn"] > 0:
                    md.append(f"**Default config** ({c['fp']:,} FP, {c['fn']:,} FN):")
                    md.append("")
                    _emit_examples(md, default_fp, default_fn, MD_EXAMPLE_LIMIT)
            for v in variants:
                v_fp = v.get("fp_examples", [])
                v_fn = v.get("fn_examples", [])
                if not v_fp and not v_fn:
                    continue
                md.append(f"**{v['style_label']}** ({v['fp']:,} FP, {v['fn']:,} FN):")
                md.append("")
                _emit_examples(md, v_fp, v_fn, MD_EXAMPLE_LIMIT)
            md.append("</details>")
            md.append("")

    # ── Per-repo table ──
    md.append("## Per-Repo Results")
    md.append("")
    md.append(f"{len(ok_repos)} repos completed successfully, {len(err_repos)} had errors.")
    md.append("")
    md.append("| Repo | Files | Match Rate | Matches | FP | FN |")
    md.append("|------|------:|----------:|--------:|---:|---:|")
    for r in sorted(ok_repos, key=lambda x: x.get("match_rate", 0)):
        files = r.get("files_inspected", 0)
        rate = fmt_pct(r['match_rate'])
        md.append(f"| {r['repo']} | {files:,} | {rate} | {r['matches']:,} | {r['fp']:,} | {r['fn']:,} |")
    md.append("")

    if err_repos:
        md.append("<details>")
        md.append(f"<summary>Repos with errors ({len(err_repos)})</summary>")
        md.append("")
        md.append("| Repo | Status | Error |")
        md.append("|------|--------|-------|")
        for r in err_repos:
            err = r.get("error_message", "")
            err_cell = f"`{err}`" if err else ""
            md.append(f"| {r['repo']} | {r['status']} | {err_cell} |")
        md.append("")
        md.append("</details>")
        md.append("")

    # ── Perfect cops ──
    if perfect_cop_list:
        md.append("<details>")
        md.append(f"<summary>Perfect cops ({len(perfect_cop_list)} cops with 100% match rate)</summary>")
        md.append("")
        md.append("| Cop | Matches |")
        md.append("|-----|--------:|")
        for c in sorted(perfect_cop_list, key=lambda x: x["matches"], reverse=True):
            md.append(f"| {c['cop']} | {c['matches']:,} |")
        md.append("")
        md.append("</details>")
        md.append("")

    args.output_md.write_text("\n".join(md) + "\n")

    # Print summary to stderr
    print(f"\nCorpus: {len(all_ids)} repos, {repos_perfect} perfect, {repos_error} errors", file=sys.stderr)
    print(f"Offenses: {oracle_total:,} compared, {total_matches:,} match, {total_fp:,} FP, {total_fn:,} FN", file=sys.stderr)
    print(f"Overall match rate: {fmt_pct(overall_rate)}", file=sys.stderr)
    if warning_repos:
        print(f"RuboCop parser crashes: {len(warning_repos)} repos, {total_files_dropped:,} files dropped", file=sys.stderr)
        for r in warning_repos:
            err = r.get("rubocop_error", "unknown")
            print(f"  - {r['repo']}: {r.get('rubocop_files_dropped', 0)} files dropped ({err})", file=sys.stderr)

    # Exit 0 always for now — CI gating can be added later via --strict flag
    sys.exit(0)


if __name__ == "__main__":
    main()
