#!/usr/bin/env python3
"""Patch variant conformance stats into the final corpus-results.json.

Must run AFTER the include-gated merge so that variant department rates
are computed from the final (merged) default FP/FN numbers. Without this
ordering, variant rates can appear higher than default rates for
departments where the IG merge improved the default numbers.

When --synthetic is provided, cops with no corpus data but a passing
synthetic result are treated as default-perfect (matching update_readme.py
logic). Synthetic variant data (from run_synthetic.py) is also merged
into variant results so that synth-only cops with EnforcedStyle options
get proper variant coverage.

Usage:
    python3 bench/corpus/patch_variant_stats.py \
        --corpus corpus-results.json \
        --variants style-variant-results.json \
        --synthetic bench/synthetic/synthetic-results.json
"""
from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path


def trunc4(rate: float) -> float:
    return math.floor(rate * 10000) / 10000


def load_synthetic(path: Path | None) -> tuple[dict[str, dict], dict[str, list[dict]]]:
    """Load synthetic results.

    Returns (synth_by_cop, synth_variant_by_cop) where synth_by_cop maps
    cop name to its default-config entry and synth_variant_by_cop maps
    cop name to its variant entries (from run_synthetic.py variant runs).
    """
    if not path or not path.exists():
        return {}, {}
    data = json.loads(path.read_text())
    by_cop = {e["cop"]: e for e in data.get("by_cop", [])}
    variant_by_cop: dict[str, list[dict]] = {}
    for batch in data.get("variants", {}).get("batches", []):
        for cop_entry in batch.get("by_cop", []):
            cop_name = cop_entry.get("cop", "")
            if cop_name:
                variant_by_cop.setdefault(cop_name, []).append(cop_entry)
    return by_cop, variant_by_cop


def patch(corpus: dict, variant_path: Path,
          synthetic_path: Path | None = None) -> bool:
    """Patch variant stats into corpus dict. Returns True if patched."""
    data = json.loads(variant_path.read_text())
    variant_by_cop: dict[str, list[dict]] = {}
    for batch in data.get("batches", []):
        for cop_entry in batch.get("by_cop", []):
            cop_name = cop_entry.get("cop", "")
            if cop_name:
                variant_by_cop.setdefault(cop_name, []).append(cop_entry)

    # Merge synthetic variant data into variant_by_cop so that synth-only
    # cops with EnforcedStyle options get variant coverage.
    synth_by_cop, synth_variant_by_cop = load_synthetic(synthetic_path)
    for cop_name, entries in synth_variant_by_cop.items():
        variant_by_cop.setdefault(cop_name, []).extend(entries)

    if not variant_by_cop:
        return False

    summary = corpus["summary"]

    # Variant totals = final default totals + all variant offenses
    v_matches = summary["matches"]
    v_fp = summary["fp"]
    v_fn = summary["fn"]
    dept_extra: dict[str, dict[str, int]] = {}

    for cop_name, variants in variant_by_cop.items():
        dept = cop_name.split("/")[0]
        if dept not in dept_extra:
            dept_extra[dept] = {"matches": 0, "fp": 0, "fn": 0}
        for v in variants:
            vm, vfp, vfn = v.get("matches", 0), v.get("fp", 0), v.get("fn", 0)
            v_matches += vm
            v_fp += vfp
            v_fn += vfn
            dept_extra[dept]["matches"] += vm
            dept_extra[dept]["fp"] += vfp
            dept_extra[dept]["fn"] += vfn

    v_total = v_matches + v_fp + v_fn
    summary["variant_overall_match_rate"] = trunc4(v_matches / v_total) if v_total > 0 else 1.0
    summary["variant_matches"] = v_matches
    summary["variant_fp"] = v_fp
    summary["variant_fn"] = v_fn

    # Per-cop variant-perfect status: a cop is variant-diverging if any
    # of its variant entries have FP+FN > 0. Used by update_readme.py
    # to compute per-department "variant matched exactly" counts.
    variant_diverging_cops: set[str] = set()
    for cop_name, variants in variant_by_cop.items():
        if any(v.get("fp", 0) + v.get("fn", 0) > 0 for v in variants):
            variant_diverging_cops.add(cop_name)

    # Per-department: count variant-perfect and variant-diverging cops
    dept_variant_perfect: dict[str, int] = {}
    dept_variant_diverging: dict[str, int] = {}
    for cop_entry in corpus.get("by_cop", []):
        cop_name = cop_entry["cop"]
        dept = cop_name.split("/")[0]
        dept_variant_perfect.setdefault(dept, 0)
        dept_variant_diverging.setdefault(dept, 0)
        # A cop is variant-perfect if it's default-perfect AND not variant-diverging.
        # Fall back to synthetic results for cops with no corpus data.
        default_perfect = cop_entry.get("perfect_match", False)
        if not default_perfect and not cop_entry.get("exercised", True):
            syn = synth_by_cop.get(cop_name)
            if syn and syn.get("perfect_match"):
                default_perfect = True
        if cop_name in variant_diverging_cops:
            dept_variant_diverging[dept] += 1
        elif default_perfect:
            dept_variant_perfect[dept] += 1
        elif cop_entry.get("diverging", False):
            # Already diverging in default — still variant-diverging
            dept_variant_diverging[dept] += 1

    for dept_entry in corpus.get("by_department", []):
        dept = dept_entry["department"]
        extra = dept_extra.get(dept)
        if extra:
            d_total = dept_entry["matches"] + dept_entry["fp"] + dept_entry["fn"]
            combined_total = d_total + extra["matches"] + extra["fp"] + extra["fn"]
            combined_matches = dept_entry["matches"] + extra["matches"]
            dept_entry["variant_match_rate"] = trunc4(
                combined_matches / combined_total) if combined_total > 0 else 1.0
        dept_entry["variant_perfect_cops"] = dept_variant_perfect.get(dept, 0)
        dept_entry["variant_diverging_cops"] = dept_variant_diverging.get(dept, 0)

    return True


def main():
    parser = argparse.ArgumentParser(
        description="Patch variant stats into corpus-results.json")
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--variants", required=True, type=Path)
    parser.add_argument("--synthetic", type=Path, default=None,
                        help="Path to synthetic-results.json (fills in cops with no corpus data)")
    args = parser.parse_args()

    if not args.variants.exists():
        print("No variant results — skipping", file=sys.stderr)
        return

    corpus = json.loads(args.corpus.read_text())
    if patch(corpus, args.variants, args.synthetic):
        args.corpus.write_text(json.dumps(corpus, indent=2) + "\n")
        rate = corpus["summary"].get("variant_overall_match_rate", 0)
        print(f"Patched variant stats: {math.floor(rate * 1000) / 10:.1f}%", file=sys.stderr)
    else:
        print("No variant data to patch", file=sys.stderr)


if __name__ == "__main__":
    main()
