#!/usr/bin/env python3
from __future__ import annotations

"""Gem conformance progress report from corpus oracle results.

Shows per-gem conformance status to help prioritize which gem to bring
to 100% corpus conformance next. Supports both a summary overview and
a deep-dive into a specific gem's cops.

Usage:
    python3 .claude/skills/fix-department/scripts/gem_progress.py --summary
    python3 .claude/skills/fix-department/scripts/gem_progress.py --gem rubocop-performance
    python3 .claude/skills/fix-department/scripts/gem_progress.py --gem rubocop-performance --input corpus-results.json
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

# Allow importing from the main scripts/ directory
_PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent.parent.parent
sys.path.insert(0, str(_PROJECT_ROOT / "scripts"))

# Maps gem names to the cop department prefixes they own.
# Keep this mapping in sync with department ownership in AGENTS.md.
GEM_DEPARTMENTS = {
    "rubocop": [
        "Bundler", "Gemspec", "Layout", "Lint", "Metrics",
        "Migration", "Naming", "Security", "Style",
    ],
    "rubocop-performance": ["Performance"],
    "rubocop-rails": ["Rails"],
    "rubocop-rspec": ["RSpec"],
    "rubocop-rspec_rails": ["RSpecRails"],
    "rubocop-factory_bot": ["FactoryBot"],
}

# Reverse map: department -> gem
DEPT_TO_GEM = {}
for gem, depts in GEM_DEPARTMENTS.items():
    for dept in depts:
        DEPT_TO_GEM[dept] = gem


from shared.corpus_artifacts import download_corpus_results as download_latest_corpus_results
from shared.corpus_artifacts import get_synthetic_results_path


def get_fixed_cops_from_git(oracle_sha: str) -> set[str]:
    """Extract cop names fixed since the corpus oracle run by scanning git history.

    Looks at commit messages between oracle_sha and HEAD for patterns like:
    - "Fix Department/CopName ..."
    - "Fix Department/CopName: ..."

    Returns a set of cop names (e.g., {"Style/RedundantConstantBase", "RSpec/Eq"}).
    """
    if not oracle_sha:
        return set()

    # Verify the SHA exists in our history
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", oracle_sha, "HEAD"],
        capture_output=True,
    )
    if result.returncode != 0:
        print(f"Warning: corpus oracle SHA {oracle_sha[:8]} not found in git history", file=sys.stderr)
        return set()

    # Get commit messages since the oracle SHA
    result = subprocess.run(
        ["git", "log", f"{oracle_sha}..HEAD", "--format=%s"],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        return set()

    # Extract cop names from "Fix Department/CopName" patterns
    cop_pattern = re.compile(r"^Fix (\w+/\w+)")
    fixed = set()
    for line in result.stdout.splitlines():
        m = cop_pattern.match(line.strip())
        if m:
            fixed.add(m.group(1))

    return fixed


def fmt_count(n: int) -> str:
    return f"{n:,}"


def cop_department(cop_name: str) -> str:
    return cop_name.split("/")[0]


def cop_gem(cop_name: str) -> str:
    dept = cop_department(cop_name)
    return DEPT_TO_GEM.get(dept, "unknown")


def classify_cop(cop: dict, synthetic_entry: dict | None = None) -> str:
    """Classify a cop using corpus results first, then synthetic fallback."""
    exercised = cop.get("exercised")
    if exercised is None:
        exercised = cop.get("matches", 0) + cop.get("fp", 0) + cop.get("fn", 0) > 0

    if exercised:
        fp = cop.get("fp", 0)
        fn = cop.get("fn", 0)
        if fp == 0 and fn == 0:
            return "perfect"
        if fp > 0 and fn == 0:
            return "fp_only"
        if fp == 0 and fn > 0:
            return "fn_only"
        return "both"

    if synthetic_entry:
        if synthetic_entry.get("diverging"):
            return "synthetic_diverging"
        if synthetic_entry.get("perfect_match"):
            return "synthetic_perfect"

    return "untested"


def find_project_root() -> Path:
    """Find the git repo root."""
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True, text=True,
    )
    if result.returncode == 0:
        return Path(result.stdout.strip())
    # Fallback: walk up from script location
    return Path(__file__).resolve().parent.parent.parent.parent.parent


def get_registry_cops() -> set[str]:
    """Get all cop names from nitrocop's registry via --list-cops."""
    project_root = find_project_root()
    try:
        result = subprocess.run(
            ["cargo", "run", "--release", "--", "--list-cops"],
            capture_output=True, text=True, cwd=project_root,
        )
    except UnicodeDecodeError:
        print("Warning: could not decode cop list output (wrong-platform binary?), skipping untested cop tracking", file=sys.stderr)
        return set()
    if result.returncode != 0:
        print("Warning: could not get cop list from registry, skipping untested cop tracking", file=sys.stderr)
        return set()
    return {line.strip() for line in result.stdout.strip().splitlines() if "/" in line}


def build_gem_stats(by_cop: list[dict], registry_cops: set[str] | None = None,
                    fixed_cops: set[str] | None = None,
                    synthetic: dict[str, dict] | None = None) -> dict[str, dict]:
    """Aggregate per-cop data into per-gem stats.

    If registry_cops is provided, also tracks cops that exist in the registry
    but have no corpus data (never triggered on the corpus).
    If fixed_cops is provided, cops listed there are moved from diverging to
    "fixed (pending confirmation)" — they still show in the data but don't count
    as diverging.
    If synthetic is provided, cops with no corpus data are reclassified using
    their synthetic status (perfect_match or diverging).
    """
    corpus_cop_names = {c["cop"] for c in by_cop}
    fixed = fixed_cops or set()
    synthetic = synthetic or {}

    gems = {}
    for gem_name in GEM_DEPARTMENTS:
        gems[gem_name] = {
            "total_in_corpus": 0,
            "total_in_registry": 0,
            "untested": 0,       # in registry but not in corpus (never triggered)
            "perfect": 0,        # in corpus, matches>0, 0 FP, 0 FN
            "fixed": 0,          # was diverging but fixed since corpus oracle (git-detected)
            "diverging": 0,
            "fp_only": 0,
            "fn_only": 0,
            "both": 0,
            "total_fp": 0,
            "total_fn": 0,
            "total_matches": 0,
            "syn_fp": 0,         # synthetic-only FP (cops with 0 corpus activity)
            "syn_fn": 0,         # synthetic-only FN (cops with 0 corpus activity)
            "syn_diverging": 0,  # synthetic-only diverging cops
            "cops": [],          # all cops in this gem from corpus data
            "fixed_cops": [],    # cop names treated as fixed
            "untested_cops": [],  # cop names missing from corpus
        }

    # Count registry cops per gem.
    if registry_cops:
        for cop in registry_cops:
            gem = cop_gem(cop)
            if gem not in gems:
                continue
            gems[gem]["total_in_registry"] += 1

    # Cops missing from the oracle payload are rare, but classify them the same
    # way as inactive corpus cops: synthetic coverage counts, otherwise untested.
    if registry_cops:
        for cop in sorted(registry_cops - corpus_cop_names):
            gem = cop_gem(cop)
            if gem not in gems:
                continue
            syn = synthetic.get(cop)
            status = classify_cop({"cop": cop, "matches": 0, "fp": 0, "fn": 0}, syn)
            if status == "synthetic_perfect":
                gems[gem]["perfect"] += 1
            elif status == "synthetic_diverging":
                gems[gem]["syn_fp"] += syn.get("fp", 0)
                gems[gem]["syn_fn"] += syn.get("fn", 0)
                gems[gem]["syn_diverging"] += 1
            else:
                gems[gem]["untested"] += 1
                gems[gem]["untested_cops"].append(cop)

    for c in by_cop:
        gem = cop_gem(c["cop"])
        if gem not in gems:
            continue
        g = gems[gem]
        status = classify_cop(c, synthetic.get(c["cop"]))
        if c.get("exercised"):
            g["total_in_corpus"] += 1
        g["total_fp"] += c["fp"]
        g["total_fn"] += c["fn"]
        g["total_matches"] += c["matches"]
        g["cops"].append(c)

        if status in {"perfect", "synthetic_perfect"}:
            g["perfect"] += 1
        elif status == "synthetic_diverging":
            syn = synthetic.get(c["cop"], {})
            g["syn_fp"] += syn.get("fp", 0)
            g["syn_fn"] += syn.get("fn", 0)
            g["syn_diverging"] += 1
        elif status == "untested":
            g["untested"] += 1
            g["untested_cops"].append(c["cop"])
        elif c["cop"] in fixed:
            g["fixed"] += 1
            g["fixed_cops"].append(c["cop"])
        elif status == "fp_only":
            g["fp_only"] += 1
            g["diverging"] += 1
        elif status == "fn_only":
            g["fn_only"] += 1
            g["diverging"] += 1
        else:
            g["both"] += 1
            g["diverging"] += 1

    # Sort lists for stable output
    for g in gems.values():
        g["untested_cops"].sort()
        g["fixed_cops"].sort()

    return gems


def print_summary(gems: dict[str, dict], run_date: str, summary: dict, has_registry: bool):
    """Print the overview scoreboard of all gems."""
    print(f"Gem Conformance Progress — {run_date}")
    print(f"{summary['total_repos']} repos, {fmt_count(summary['total_offenses_compared'])} offenses compared")
    print()

    # Sort by diverging count (ascending = closest to done first), then untested
    sorted_gems = sorted(gems.items(), key=lambda kv: (kv[1]["diverging"], kv[1]["untested"], kv[1]["total_fp"]))

    # Column widths
    gem_w = max(len(g) for g, _ in sorted_gems)
    gem_w = max(gem_w, 3)

    has_fixed = any(g["fixed"] > 0 for _, g in sorted_gems)
    has_syn = any(g["syn_diverging"] > 0 for _, g in sorted_gems)

    # Adapt columns based on whether we have registry data
    if has_registry:
        hdr = f"  {'Gem':<{gem_w}}  {'Reg':>4}  {'Corpus':>6}  {'Untest':>6}  {'Perf':>5}"
        sep = f"  {'':->{gem_w}}  {'':->4}  {'':->6}  {'':->6}  {'':->5}"
        if has_fixed:
            hdr += f"  {'Fixed':>5}"
            sep += f"  {'':->5}"
        hdr += f"  {'Dvrg':>5}  {'Total FP':>9}  {'Total FN':>9}"
        sep += f"  {'':->5}  {'':->9}  {'':->9}"
        if has_syn:
            hdr += f"  {'SynFP':>5}  {'SynFN':>5}"
            sep += f"  {'':->5}  {'':->5}"
        hdr += "  Status"
        sep += f"  {'':->30}"
        print(hdr)
        print(sep)
    else:
        hdr = f"  {'Gem':<{gem_w}}  {'Corpus':>6}  {'Perf':>5}"
        sep = f"  {'':->{gem_w}}  {'':->6}  {'':->5}"
        if has_fixed:
            hdr += f"  {'Fixed':>5}"
            sep += f"  {'':->5}"
        hdr += f"  {'Dvrg':>5}  {'Total FP':>9}  {'Total FN':>9}"
        sep += f"  {'':->5}  {'':->9}  {'':->9}"
        if has_syn:
            hdr += f"  {'SynFP':>5}  {'SynFN':>5}"
            sep += f"  {'':->5}  {'':->5}"
        hdr += "  Status"
        sep += f"  {'':->30}"
        print(hdr)
        print(sep)

    for gem, g in sorted_gems:
        if g["total_in_corpus"] == 0 and g["total_in_registry"] == 0:
            continue

        # Determine status
        syn_dvrg = g["syn_diverging"]
        if g["diverging"] == 0 and g["untested"] == 0 and g["fixed"] == 0 and syn_dvrg == 0:
            status = "100% conformance"
        elif g["diverging"] == 0 and g["untested"] == 0 and g["fixed"] == 0 and syn_dvrg > 0:
            status = f"corpus 100%, {syn_dvrg} syn diverge"
        elif g["diverging"] == 0 and g["fixed"] > 0 and g["untested"] == 0:
            status = "done (pending corpus confirmation)"
        elif g["diverging"] == 0 and g["untested"] > 0:
            status = f"0 FP/FN but {g['untested']} untested"
        elif g["total_fp"] == 0:
            status = f"FP-free! {g['diverging']} FN-only cops"
        else:
            parts = [f"{g['diverging']} to fix"]
            if g["fixed"] > 0:
                parts.append(f"{g['fixed']} fixed")
            if g["untested"] > 0:
                parts.append(f"{g['untested']} untested")
            if syn_dvrg > 0:
                parts.append(f"{syn_dvrg} syn diverge")
            status = ", ".join(parts)

        if has_registry:
            row = f"  {gem:<{gem_w}}  {g['total_in_registry']:>4}  {g['total_in_corpus']:>6}  {g['untested']:>6}  {g['perfect']:>5}"
            if has_fixed:
                row += f"  {g['fixed']:>5}"
            row += f"  {g['diverging']:>5}  {fmt_count(g['total_fp']):>9}  {fmt_count(g['total_fn']):>9}"
            if has_syn:
                row += f"  {g['syn_fp']:>5}  {g['syn_fn']:>5}"
            row += f"  {status}"
        else:
            row = f"  {gem:<{gem_w}}  {g['total_in_corpus']:>6}  {g['perfect']:>5}"
            if has_fixed:
                row += f"  {g['fixed']:>5}"
            row += f"  {g['diverging']:>5}  {fmt_count(g['total_fp']):>9}  {fmt_count(g['total_fn']):>9}"
            if has_syn:
                row += f"  {g['syn_fp']:>5}  {g['syn_fn']:>5}"
            row += f"  {status}"
        print(row)

    print()

    # Legend
    if has_registry:
        legend = "  Reg=registry cops  Corpus=triggered on corpus  Untest=no corpus or synthetic coverage  Perf=0 FP+FN"
        if has_fixed:
            legend += "  Fixed=pending confirm"
        legend += "  Dvrg=FP or FN >0"
        if has_syn:
            legend += "  SynFP/SynFN=synthetic-only divergence"
        print(legend)
        print()

    # Summary stats
    total_diverging = sum(g["diverging"] for g in gems.values())
    total_perfect = sum(g["perfect"] for g in gems.values())
    total_fixed = sum(g["fixed"] for g in gems.values())
    total_untested = sum(g["untested"] for g in gems.values())
    gems_at_100 = sum(1 for g in gems.values()
                      if g["diverging"] == 0 and g["untested"] == 0 and g["syn_diverging"] == 0
                      and (g["total_in_corpus"] > 0 or g["total_in_registry"] > 0))
    total_gems = sum(1 for g in gems.values()
                     if g["total_in_corpus"] > 0 or g["total_in_registry"] > 0)
    parts = [f"{gems_at_100}/{total_gems} gems at 100% conformance",
             f"{total_perfect} verified perfect"]
    if total_fixed:
        parts.append(f"{total_fixed} fixed (pending)")
    parts.extend([f"{total_diverging} diverging", f"{total_untested} untested"])
    print(f"Overall: {', '.join(parts)}")

    # Recommendation: pick the best next target
    candidates = [(name, g) for name, g in sorted_gems
                  if g["diverging"] > 0 and (g["total_in_corpus"] > 0 or g["total_in_registry"] > 0)]
    if not candidates:
        return

    print()
    print("Recommendation:")

    # Prefer gems with 0 untested (can claim true 100%)
    full_coverage = [(n, g) for n, g in candidates if g["untested"] == 0]
    if full_coverage:
        best_name, best = min(full_coverage, key=lambda x: x[1]["diverging"])
        print(f"  Best target: {best_name} ({best['diverging']} diverging, 0 untested = clean 100% claim)")
    else:
        # No gem has full corpus coverage; recommend by adoption value since all have asterisks
        # Adoption value: performance (most added plugin) > rspec_rails (small) > rails > rspec > core
        adoption_rank = {
            "rubocop-performance": (0, "most commonly added plugin"),
            "rubocop-rspec_rails": (1, "smallest, easiest to complete"),
            "rubocop-rails": (2, "large Rails ecosystem"),
            "rubocop-rspec": (3, "widely used"),
            "rubocop": (4, "too large — use /fix-cops instead"),
        }
        best_name, best = min(candidates,
                              key=lambda x: adoption_rank.get(x[0], (99, ""))[0])
        reason = adoption_rank.get(best_name, (99, ""))[1]
        print(f"  Best target: {best_name} ({best['diverging']} diverging, {best['untested']} untested) — {reason}")
        # Also mention the quickest win
        quickest_name, quickest = min(candidates, key=lambda x: x[1]["diverging"])
        if quickest_name != best_name:
            print(f"  Quickest win: {quickest_name} ({quickest['diverging']} diverging) — least work to complete")
        print("  Note: No remaining gem has 0 untested cops — true 100% needs all cops to trigger on corpus.")

    # Show quick-win info
    if best["fp_only"] > 0:
        print(f"  Quick wins: {best['fp_only']} FP-only cops (fix first, no risk of introducing FNs)")


def print_gem_detail(gem_name: str, gems: dict[str, dict], run_date: str,
                     synthetic: dict[str, dict] | None = None):
    """Print deep-dive for a specific gem."""
    if gem_name not in gems:
        print(f"Unknown gem: {gem_name}", file=sys.stderr)
        print(f"Available gems: {', '.join(sorted(GEM_DEPARTMENTS.keys()))}", file=sys.stderr)
        sys.exit(1)

    g = gems[gem_name]
    cops = g["cops"]

    if not cops:
        print(f"No corpus data for {gem_name} (0 cops found in corpus results)")
        return

    fixed_set = set(g["fixed_cops"])

    # Categorize cops (exclude fixed cops from diverging categories)
    perfect = sorted(
        [
            c for c in cops
            if classify_cop(c, synthetic.get(c["cop"]) if synthetic else None)
            in {"perfect", "synthetic_perfect"}
        ],
        key=lambda c: c["cop"],
    )
    fixed = sorted([c for c in cops if c["cop"] in fixed_set],
                   key=lambda c: c["cop"])
    fp_only = sorted(
        [
            c for c in cops
            if classify_cop(c, synthetic.get(c["cop"]) if synthetic else None) == "fp_only"
            and c["cop"] not in fixed_set
        ],
        key=lambda c: c["fp"],
        reverse=True,
    )
    fn_only = sorted(
        [
            c for c in cops
            if classify_cop(c, synthetic.get(c["cop"]) if synthetic else None) == "fn_only"
            and c["cop"] not in fixed_set
        ],
        key=lambda c: c["fn"],
        reverse=True,
    )
    both = sorted(
        [
            c for c in cops
            if classify_cop(c, synthetic.get(c["cop"]) if synthetic else None) == "both"
            and c["cop"] not in fixed_set
        ],
        key=lambda c: c["fp"],
        reverse=True,
    )

    print(f"{gem_name} — Conformance Deep Dive ({run_date})")
    print(f"Departments: {', '.join(GEM_DEPARTMENTS[gem_name])}")
    reg = g["total_in_registry"]
    corpus = g["total_in_corpus"]
    untested = g["untested"]
    if reg > 0:
        print(f"{reg} cops in registry, {corpus} exercised on corpus, {untested} untested (no corpus or synthetic coverage)")
    else:
        print(f"{corpus} cops exercised on corpus")
    print(f"{g['perfect']} verified perfect, {g['diverging']} diverging "
          f"({g['fp_only']} FP-only, {g['fn_only']} FN-only, {g['both']} both)")
    print()

    # Perfect cops (compact list)
    if perfect:
        names = [c["cop"].split("/")[1] for c in perfect]
        print(f"Perfect ({len(perfect)}):")
        # Wrap at ~100 chars
        line = "  "
        for i, name in enumerate(names):
            addition = name + (", " if i < len(names) - 1 else "")
            if len(line) + len(addition) > 100:
                print(line)
                line = "  " + addition
            else:
                line += addition
        if line.strip():
            print(line)
        print()

    # Fixed cops (pending corpus confirmation)
    if fixed:
        names = [c["cop"].split("/")[1] for c in fixed]
        print(f"Fixed — pending corpus confirmation ({len(fixed)}):")
        line = "  "
        for i, name in enumerate(names):
            addition = name + (", " if i < len(names) - 1 else "")
            if len(line) + len(addition) > 100:
                print(line)
                line = "  " + addition
            else:
                line += addition
        if line.strip():
            print(line)
        print()

    # FP-only cops (fix these first!)
    if fp_only:
        print(f"FP-only ({len(fp_only)} — fix these first!):")
        cop_w = max(len(c["cop"]) for c in fp_only)
        for i, c in enumerate(fp_only, 1):
            match_pct = f"{c['match_rate']:.1%}" if c["matches"] > 0 else "N/A"
            print(f"  #{i:<3} {c['cop']:<{cop_w}}  FP={fmt_count(c['fp']):>7}  "
                  f"matches={fmt_count(c['matches']):>7}  ({match_pct})")
        print()

    # Both FP+FN cops
    if both:
        print(f"Both FP+FN ({len(both)} — fix FPs first):")
        cop_w = max(len(c["cop"]) for c in both)
        for i, c in enumerate(both, 1):
            match_pct = f"{c['match_rate']:.1%}" if (c["matches"] + c["fn"]) > 0 else "N/A"
            print(f"  #{i:<3} {c['cop']:<{cop_w}}  FP={fmt_count(c['fp']):>7}  "
                  f"FN={fmt_count(c['fn']):>7}  matches={fmt_count(c['matches']):>7}  ({match_pct})")
        print()

    # FN-only cops
    if fn_only:
        print(f"FN-only ({len(fn_only)} — lower priority, missing detections):")
        cop_w = max(len(c["cop"]) for c in fn_only)
        for i, c in enumerate(fn_only, 1):
            match_pct = f"{c['match_rate']:.1%}" if (c["matches"] + c["fn"]) > 0 else "N/A"
            print(f"  #{i:<3} {c['cop']:<{cop_w}}  FN={fmt_count(c['fn']):>7}  "
                  f"matches={fmt_count(c['matches']):>7}  ({match_pct})")
        print()

    # Untested cops (in registry but never triggered on corpus)
    if g["untested_cops"]:
        print(f"Untested ({g['untested']} — no corpus or synthetic coverage):")
        for cop in g["untested_cops"]:
            print(f"  {cop}")
        print()

    # Synthetic-only divergence (cops with 0 corpus activity but FP/FN in synthetic)
    if synthetic:
        syn_diverging = []
        for c in cops:
            if classify_cop(c, synthetic.get(c["cop"])) == "synthetic_diverging":
                syn_diverging.append(synthetic[c["cop"]])
        if syn_diverging:
            syn_diverging.sort(key=lambda s: s.get("fp", 0) + s.get("fn", 0), reverse=True)
            print(f"Synthetic-only divergence ({len(syn_diverging)} — no corpus activity, but diverge on synthetic tests):")
            cop_w = max(len(s["cop"]) for s in syn_diverging)
            for s in syn_diverging:
                parts = []
                if s.get("fp", 0) > 0:
                    parts.append(f"FP={s['fp']}")
                if s.get("fn", 0) > 0:
                    parts.append(f"FN={s['fn']}")
                print(f"  {s['cop']:<{cop_w}}  {' '.join(parts)}")
            print("  (Investigate via bench/synthetic/synthetic-results.json, source at bench/synthetic/project/)")
            print()

    # Strategy recommendation
    if g["diverging"] > 0 or g["untested"] > 0:
        print("Strategy:")
        step = 0
        if fp_only:
            step += 1
            print(f"  {step}. Fix {len(fp_only)} FP-only cops to eliminate all false alarms from these cops")
        if both:
            step += 1
            print(f"  {step}. Fix FP side of {len(both)} both-FP+FN cops")
        if fn_only:
            step += 1
            print(f"  {step}. Fix {len(fn_only)} FN-only cops for full 100% conformance")
        if g["diverging"] > 0:
            fp_cops = len(fp_only) + len(both)
            print(f"  Total FP-producing cops: {fp_cops} ({fmt_count(g['total_fp'])} false positives)")
            print(f"  Total FN-producing cops: {len(fn_only) + len(both)} ({fmt_count(g['total_fn'])} false negatives)")
        if g["untested"] > 0:
            print()
            can_claim = g["diverging"] == 0
            if can_claim:
                print(f"  Note: All corpus-tested cops are perfect, but {g['untested']} cops never triggered.")
            else:
                print(f"  Note: {g['untested']} cops have no corpus data — cannot claim full 100% until they're exercised.")
            print("  These cops may be correct but are unverified against real-world code.")
    else:
        print("This gem is at 100% corpus conformance! All cops tested and verified.")


def main():
    parser = argparse.ArgumentParser(description="Gem conformance progress report")
    parser.add_argument("--input", type=Path,
                        help="Path to corpus-results.json (default: download from CI)")
    parser.add_argument("--summary", action="store_true",
                        help="Show overview scoreboard of all gems")
    parser.add_argument("--gem", type=str,
                        help="Deep-dive into a specific gem (e.g., rubocop-performance)")
    parser.add_argument("--exclude-cops-file", type=Path,
                        help="(deprecated, use git-based detection) File with cop names to treat as fixed")
    parser.add_argument("--synthetic", type=Path, default=None,
                        help="Path to synthetic-results.json (reclassifies untested cops)")
    parser.add_argument("--no-git-exclude", action="store_true",
                        help="Disable automatic git-based exclusion of already-fixed cops")
    args = parser.parse_args()

    # Default to --summary when no args given
    if not args.summary and not args.gem:
        args.summary = True

    # Load corpus results
    oracle_sha = ""
    _run_id = 0
    if args.input:
        input_path = args.input
    else:
        input_path, _run_id, oracle_sha = download_latest_corpus_results()

    data = json.loads(input_path.read_text())
    summary = data["summary"]
    by_cop = data["by_cop"]
    run_date = data.get("run_date", "unknown")[:10]

    # Get registry cops for untested detection (requires cargo build)
    print("Loading cop registry...", file=sys.stderr)
    registry_cops = get_registry_cops()
    has_registry = len(registry_cops) > 0
    if not has_registry:
        print("Warning: running without registry data — untested cops won't be shown", file=sys.stderr)

    # Detect cops fixed since the corpus oracle run via git history
    fixed_cops: set[str] = set()
    if not args.no_git_exclude and oracle_sha:
        git_fixed = get_fixed_cops_from_git(oracle_sha)
        if git_fixed:
            fixed_cops |= git_fixed
            print(f"Found {len(git_fixed)} cops fixed since corpus oracle ({oracle_sha[:8]})", file=sys.stderr)

    # Also support legacy --exclude-cops-file for manual exclusions
    if args.exclude_cops_file and args.exclude_cops_file.exists():
        file_cops = {line.strip() for line in args.exclude_cops_file.read_text().splitlines()
                     if line.strip() and not line.startswith("#")}
        fixed_cops |= file_cops

    if fixed_cops:
        print(f"Treating {len(fixed_cops)} cops as fixed (pending corpus confirmation)", file=sys.stderr)

    # Load synthetic results: explicit path > local file > cached from CI artifact
    synthetic = None
    syn_path = args.synthetic or Path("bench/synthetic/synthetic-results.json")
    if syn_path.exists():
        syn_data = json.loads(syn_path.read_text())
        synthetic = {entry["cop"]: entry for entry in syn_data.get("by_cop", [])}
        print(f"Loaded {len(synthetic)} synthetic results from {syn_path}", file=sys.stderr)
    elif _run_id:
        cached_syn = get_synthetic_results_path(_run_id)
        if cached_syn:
            syn_data = json.loads(cached_syn.read_text())
            synthetic = {entry["cop"]: entry for entry in syn_data.get("by_cop", [])}
            print(f"Loaded {len(synthetic)} synthetic results from CI cache", file=sys.stderr)

    gems = build_gem_stats(by_cop, registry_cops if has_registry else None, fixed_cops, synthetic)

    if args.summary:
        print_summary(gems, run_date, summary, has_registry)

    if args.gem:
        if args.summary:
            print()
            print("=" * 80)
            print()
        print_gem_detail(args.gem, gems, run_date, synthetic)


if __name__ == "__main__":
    main()
