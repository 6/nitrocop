#!/usr/bin/env python3
"""Render cop-check PR comment from shard summary files.

Aggregates variant rows across shards before rendering, since variant
baselines are global but each shard only runs a subset of repos.

Usage:
    python3 scripts/workflows/render_cop_check_comment.py \
        --summaries-dir summaries/ \
        --result pass
"""
from __future__ import annotations

import argparse
import re
from pathlib import Path


def parse_summary_lines(summaries_dir: Path) -> list[dict]:
    """Parse all shard summary files into structured rows."""
    rows = []
    for f in sorted(summaries_dir.glob("*.txt")):
        for line in f.read_text().splitlines():
            if not line.strip():
                continue
            parts = line.split("|")
            if len(parts) < 6:
                continue
            row = {
                "cop": parts[0].strip(),
                "bl_fp": int(parts[1].strip()) if parts[1].strip() else 0,
                "bl_fn": int(parts[2].strip()) if parts[2].strip() else 0,
                "local_fp": int(parts[3].strip()) if parts[3].strip() else 0,
                "local_fn": int(parts[4].strip()) if parts[4].strip() else 0,
                "result": parts[5].strip() if len(parts) > 5 else "pass",
                "count_bl_fp": int(parts[6].strip()) if len(parts) > 6 and parts[6].strip() else None,
                "count_bl_fn": int(parts[7].strip()) if len(parts) > 7 and parts[7].strip() else None,
                "detail": parts[8].strip() if len(parts) > 8 and parts[8].strip() else "",
            }
            rows.append(row)
    return rows


def is_variant_row(cop: str) -> bool:
    """Check if a cop name is a variant row (contains parenthesized style label)."""
    return bool(re.search(r"\(.*\)$", cop))


def aggregate_rows(rows: list[dict]) -> list[dict]:
    """Aggregate variant rows across shards, keep default rows as-is.

    Default cop rows (no parentheses) are kept individually per shard —
    the default gate already handles per-repo regression correctly.

    Variant rows are summed across shards — both baselines and local
    FP/FN are per-repo scoped, so summing across shards gives the
    correct totals for the sampled repos.
    """
    default_rows = []
    variant_agg: dict[str, dict] = {}  # cop name -> aggregated data

    for row in rows:
        if is_variant_row(row["cop"]):
            key = row["cop"]
            if key not in variant_agg:
                variant_agg[key] = {
                    "cop": key,
                    "bl_fp": 0,
                    "bl_fn": 0,
                    "local_fp": 0,
                    "local_fn": 0,
                    "result": "pass",
                    "count_bl_fp": None,
                    "count_bl_fn": None,
                    "details": [],
                }
            agg = variant_agg[key]
            agg["bl_fp"] += row["bl_fp"]
            agg["bl_fn"] += row["bl_fn"]
            agg["local_fp"] += row["local_fp"]
            agg["local_fn"] += row["local_fn"]
            if row.get("detail"):
                agg["details"].append(row["detail"])
            # If any shard errored, mark as error
            if row["result"] == "error":
                agg["result"] = "error"
        else:
            default_rows.append(row)

    # Determine variant pass/fail based on aggregated delta.
    # Regression takes priority over error — a regression with some
    # rubocop errors is still a regression that should block.
    for agg in variant_agg.values():
        fp_delta = agg["local_fp"] - agg["bl_fp"]
        fn_delta = agg["local_fn"] - agg["bl_fn"]
        if fp_delta > 0 or fn_delta > 0:
            agg["result"] = "regression"
        elif agg["result"] != "error":
            agg["result"] = "pass"

    return default_rows + sorted(variant_agg.values(), key=lambda r: r["cop"])


def render_comment(rows: list[dict], overall_result: str) -> str:
    """Render the PR comment markdown."""
    icon = "✅" if overall_result == "pass" else "❌"
    lines = [
        f"{icon} **Cop-check results** (12 shards)\n",
        "| Cop | Baseline FP | Baseline FN | Local FP | Local FN | FP Δ | FN Δ | Result |",
        "|-----|-----------|-----------|--------|--------|------|------|--------|",
    ]

    for row in rows:
        status = "✅"
        if row["result"] == "fail" or row["result"] == "regression":
            status = "❌"

        fp_delta = row["local_fp"] - row["bl_fp"]
        fn_delta = row["local_fn"] - row["bl_fn"]
        fp_str = f"+{fp_delta}" if fp_delta > 0 else str(fp_delta)
        fn_str = f"+{fn_delta}" if fn_delta > 0 else str(fn_delta)

        # Sanity check annotation for default rows
        note = ""
        if row.get("count_bl_fp") is not None and row["result"] == "fail":
            count_fp_delta = row["local_fp"] - row["count_bl_fp"]
            if count_fp_delta <= 0 and fp_delta > 0:
                note = f" ⚠️ count Δ={count_fp_delta}"

        lines.append(
            f"| `{row['cop']}` | {row['bl_fp']} | {row['bl_fn']} "
            f"| {row['local_fp']} | {row['local_fn']} "
            f"| {fp_str} | {fn_str} | {status}{note} |"
        )

        # Show diverging repos for variant regressions so agents know
        # exactly where to look instead of re-running the entire corpus.
        details = row.get("details", [])
        if details and row["result"] in ("fail", "regression"):
            combined = " ".join(details)
            if combined:
                lines.append(f"| | | | | | | | Repos: `{combined}` |")

    lines.append("")
    if overall_result == "pass":
        lines.append("All shards passed — ready to merge.")
    else:
        lines.append("Regressions detected. The repair workflow will attempt an automatic fix.")

    return "\n".join(lines)


def has_variant_regressions(rows: list[dict]) -> bool:
    """Check if any aggregated variant row has a regression."""
    return any(r["result"] == "regression" for r in rows if is_variant_row(r["cop"]))


def main():
    parser = argparse.ArgumentParser(description="Render cop-check PR comment")
    parser.add_argument("--summaries-dir", required=True, type=Path)
    parser.add_argument("--result", required=True, choices=["pass", "fail", "skip", "cancelled"])
    parser.add_argument("--check-variant-regressions", action="store_true",
                        help="Exit non-zero if any variant regression is detected")
    args = parser.parse_args()

    if not args.summaries_dir.exists():
        rows = []
    else:
        raw_rows = parse_summary_lines(args.summaries_dir)
        rows = aggregate_rows(raw_rows)

    # If default gate passed but variants regressed, override result
    effective_result = args.result
    if args.result == "pass" and has_variant_regressions(rows):
        effective_result = "fail"

    comment = render_comment(rows, effective_result)
    print(comment)

    if args.check_variant_regressions and has_variant_regressions(rows):
        import sys
        print(
            "ERROR: variant style regressions detected",
            file=sys.stderr,
        )
        sys.exit(1)


if __name__ == "__main__":
    main()
