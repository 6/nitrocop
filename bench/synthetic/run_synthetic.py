#!/usr/bin/env python3
"""Run nitrocop and RuboCop on the synthetic project and compare results.

Usage:
    python3 bench/synthetic/run_synthetic.py
    python3 bench/synthetic/run_synthetic.py --verbose
    python3 bench/synthetic/run_synthetic.py --output /tmp/results.json
"""
from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import tempfile
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

# Cops that exist in nitrocop's registry but cannot be triggered under
# our corpus config (Ruby 4.0 / Rails 8.0 target). Mirrored from
# bench/corpus/diff_results.py UNSUPPORTED_COPS.
UNSUPPORTED_COPS = frozenset([
    "Lint/ItWithoutArgumentsInBlock",       # max Ruby 3.3 (it is a block param in 3.4+)
    "Lint/NonDeterministicRequireOrder",     # max Ruby 2.7 (Dir sorts since 3.0)
    "Lint/NumberedParameterAssignment",      # syntax error in Ruby 3.4+
    "Lint/UselessElseWithoutRescue",         # syntax error in Ruby 3.4+
    "Security/YAMLLoad",                    # max Ruby 3.0 (YAML.load safe in 3.1+)
])

# Cops with no or very low corpus data, excluding UNSUPPORTED_COPS.
# Includes zero-activity cops (synthetic-only validation) and cops with
# <= 5 total corpus occurrences (safety net against corpus churn).
TARGET_COPS = sorted([
    "FactoryBot/IdSequence",
    "Lint/ArrayLiteralInRegexp",
    "Lint/BigDecimalNew",
    "Lint/CircularArgumentReference",
    "Lint/ConstantOverwrittenInRescue",
    "Lint/DuplicateMagicComment",
    "Lint/DuplicateMatchPattern",
    "Lint/DuplicateRescueException",
    "Lint/EachWithObjectArgument",
    "Lint/NextWithoutAccumulator",
    "Lint/NumericOperationWithConstantResult",
    "Lint/PercentSymbolArray",
    "Lint/RedundantRegexpQuantifiers",
    "Lint/RefinementImportMethods",
    "Lint/RegexpAsCondition",
    "Lint/RequireRangeParentheses",
    "Lint/RequireRelativeSelfPath",
    "Lint/RescueType",
    "Lint/SafeNavigationWithEmpty",
    "Lint/TrailingCommaInAttributeDeclaration",
    "Lint/UnescapedBracketInRegexp",
    "Lint/UselessRuby2Keywords",
    "Performance/RedundantSortBlock",
    "RSpec/AroundBlock",
    "RSpec/DuplicatedMetadata",
    "RSpec/EmptyMetadata",
    "RSpec/InstanceSpy",
    "RSpec/IsExpectedSpecify",
    "RSpec/RedundantAround",
    "RSpec/RepeatedIncludeExample",
    "RSpec/SkipBlockInsideExample",
    "RSpecRails/HttpStatusNameConsistency",
    "Rails/ActionControllerTestCase",
    "Rails/ActiveSupportAliases",
    "Rails/AddColumnIndex",
    "Rails/AfterCommitOverride",
    "Rails/ApplicationJob",
    "Rails/ApplicationMailer",
    "Rails/ApplicationRecord",
    "Rails/BelongsTo",
    "Rails/BulkChangeTable",
    "Rails/CompactBlank",
    "Rails/ContentTag",
    "Rails/CreateTableWithTimestamps",
    "Rails/DangerousColumnNames",
    "Rails/DelegateAllowBlank",
    "Rails/EnumSyntax",
    "Rails/EnumUniqueness",
    "Rails/EnvLocal",
    "Rails/ExpandedDateRange",
    "Rails/FreezeTime",
    "Rails/HttpPositionalArguments",
    "Rails/HttpStatusNameConsistency",
    "Rails/I18nLocaleAssignment",
    "Rails/IgnoredSkipActionFilterOption",
    "Rails/IndexWith",
    "Rails/MigrationClassName",
    "Rails/MultipleRoutePaths",
    "Rails/NotNullColumn",
    "Rails/Pick",
    "Rails/Pluck",
    "Rails/RedirectBackOrTo",
    "Rails/RedundantAllowNil",
    "Rails/RedundantPresenceValidationOnBelongsTo",
    "Rails/RedundantTravelBack",
    "Rails/RequireDependency",
    "Rails/ResponseParsedBody",
    "Rails/ReversibleMigration",
    "Rails/RootJoinChain",
    "Rails/StrongParametersExpect",
    "Rails/ReversibleMigrationMethodDefinition",
    "Rails/SafeNavigationWithBlank",
    "Rails/ThreeStateBooleanColumn",
    "Rails/TimeZoneAssignment",
    "Rails/ToFormattedS",
    "Rails/ToSWithArgument",
    "Rails/TopLevelHashWithIndifferentAccess",
    "Rails/UniqueValidationWithoutIndex",
    "Rails/UnusedIgnoredColumns",
    "Rails/WhereMissing",
    "Rails/WhereRange",
    "Style/AmbiguousEndlessMethodDefinition",
    "Style/BeginBlock",
    "Style/Copyright",
    "Style/DataInheritance",
    "Style/DirEmpty",
    "Style/DoubleCopDisableDirective",
    "Style/ExactRegexpMatch",
    "Style/FileTouch",
    "Style/InPatternThen",
    "Style/MinMax",
    "Style/MultilineInPatternThen",
    "Style/RedundantConstantBase",
    "Style/ReverseFind",
    "Style/Strip",
    "Style/SwapValues",
    "Style/TrailingBodyOnClass",
])


def trunc4(rate: float) -> float:
    """Truncate rate to 4 decimal places (never rounds up to 1.0)."""
    return math.floor(rate * 10000) / 10000


def normalize_path(filepath: str, project_dir: str) -> str:
    """Normalize a file path to be relative to the project directory."""
    project_prefix = os.path.abspath(project_dir) + "/"
    if filepath.startswith(project_prefix):
        filepath = filepath[len(project_prefix):]
    elif filepath.startswith("project/"):
        filepath = filepath[len("project/"):]
    # Strip leading ./ from relative paths
    if filepath.startswith("./"):
        filepath = filepath[2:]
    return filepath


def parse_nitrocop_json(data: dict, project_dir: str) -> set[tuple[str, int, str]]:
    """Parse nitrocop JSON output into offense tuples."""
    offenses = set()
    for o in data.get("offenses", []):
        filepath = normalize_path(o.get("path", ""), project_dir)
        line = o.get("line", 0)
        cop = o.get("cop_name", "")
        if filepath and cop:
            offenses.add((filepath, line, cop))
    return offenses


def parse_rubocop_json(data: dict, project_dir: str) -> set[tuple[str, int, str]]:
    """Parse RuboCop JSON output into offense tuples."""
    offenses = set()
    for f in data.get("files", []):
        filepath = normalize_path(f.get("path", ""), project_dir)
        for o in f.get("offenses", []):
            line = o.get("location", {}).get("line", 0)
            cop = o.get("cop_name", "")
            if filepath and cop:
                offenses.add((filepath, line, cop))
    return offenses


def find_variant_cops(target_set: set[str]) -> list[dict]:
    """Find TARGET_COPS that have EnforcedStyle alternatives.

    Returns a list of dicts with keys: cop, key, default, alternatives.
    """
    project_root = Path(__file__).resolve().parent.parent.parent
    sys.path.insert(0, str(project_root / "scripts"))
    from corpus_stress import VENDOR_CONFIGS, parse_enforced_styles  # noqa: E402

    results = []
    for config_path, _plugin in VENDOR_CONFIGS:
        full_path = str(project_root / config_path)
        if not Path(full_path).exists():
            continue
        for s in parse_enforced_styles(full_path):
            if s["cop"] in target_set and s["alternatives"]:
                results.append(s)
    return results


def run_variant_batches(
    variant_styles: list[dict],
    target_set: set[str],
    nitrocop_binary: str,
    rubocop_yml: str,
    project_dir: str,
    gemfile: str,
    verbose: bool,
) -> dict:
    """Run nitrocop + RuboCop with variant configs and return results.

    Groups alternatives by index (batch 1 = first alt for each cop, etc.)
    to match the corpus variant batch structure.
    """
    # Group by alternative index
    max_alts = max((len(s["alternatives"]) for s in variant_styles), default=0)
    if max_alts == 0:
        return {"batches": []}

    batches_result = []
    for alt_idx in range(max_alts):
        overrides: list[tuple[str, str, str]] = []  # (cop, key, value)
        for s in variant_styles:
            if alt_idx < len(s["alternatives"]):
                overrides.append((s["cop"], s["key"], s["alternatives"][alt_idx]))

        if not overrides:
            continue

        # Generate a temporary .rubocop.yml with style overrides
        lines = [
            f"# Auto-generated variant batch {alt_idx + 1} for synthetic bench",
            f"inherit_from: {rubocop_yml}",
            "",
        ]
        for cop, key, value in overrides:
            lines.append(f"{cop}:")
            lines.append(f"  {key}: {value}")
            lines.append("")

        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".yml", dir=project_dir,
            prefix=".rubocop_variant_", delete=False,
        ) as tmp:
            tmp.write("\n".join(lines) + "\n")
            variant_config = tmp.name

        override_cops = {cop for cop, _, _ in overrides}
        style_label = f"variant_batch_{alt_idx + 1}"
        try:
            batch = _run_single_variant(
                variant_config, override_cops, target_set,
                nitrocop_binary, project_dir, gemfile, style_label, verbose,
            )
            batches_result.append(batch)
        finally:
            os.unlink(variant_config)

    return {"batches": batches_result}


def _run_single_variant(
    config_path: str,
    override_cops: set[str],
    target_set: set[str],
    nitrocop_binary: str,
    project_dir: str,
    gemfile: str,
    style_label: str,
    verbose: bool,
) -> dict:
    """Run a single variant config and return a batch result dict."""
    # Only diff the cops whose style was overridden
    filter_set = override_cops & target_set

    print(f"Running variant {style_label} ({len(filter_set)} cops)...",
          end="", file=sys.stderr, flush=True)

    # nitrocop
    nc_result = subprocess.run(
        [nitrocop_binary, "--preview", "--no-cache", "--format", "json",
         "--config", config_path, "."],
        capture_output=True, text=True, cwd=project_dir,
    )
    nc_data = json.loads(nc_result.stdout) if nc_result.stdout.strip() else {"offenses": []}
    nc_offenses = {o for o in parse_nitrocop_json(nc_data, project_dir) if o[2] in filter_set}

    # RuboCop
    rc_env = os.environ.copy()
    rc_env["BUNDLE_GEMFILE"] = gemfile
    rc_result = subprocess.run(
        ["bundle", "exec", "rubocop", "--config", config_path, "--format", "json", "."],
        capture_output=True, text=True, env=rc_env, cwd=project_dir,
    )
    rc_data = json.loads(rc_result.stdout) if rc_result.stdout.strip() else {"files": []}
    rc_offenses = {o for o in parse_rubocop_json(rc_data, project_dir) if o[2] in filter_set}

    # Diff
    matches = nc_offenses & rc_offenses
    fp = nc_offenses - rc_offenses
    fn = rc_offenses - nc_offenses

    by_cop_m: dict[str, int] = defaultdict(int)
    by_cop_fp: dict[str, int] = defaultdict(int)
    by_cop_fn: dict[str, int] = defaultdict(int)
    for _, _, cop in matches:
        by_cop_m[cop] += 1
    for _, _, cop in fp:
        by_cop_fp[cop] += 1
    for _, _, cop in fn:
        by_cop_fn[cop] += 1

    by_cop_list = []
    for cop in sorted(filter_set | set(by_cop_m) | set(by_cop_fp) | set(by_cop_fn)):
        m = by_cop_m.get(cop, 0)
        f = by_cop_fp.get(cop, 0)
        n = by_cop_fn.get(cop, 0)
        total = m + f + n
        rate = m / total if total > 0 else 1.0
        exercised = total > 0
        diverging = f + n > 0
        by_cop_list.append({
            "cop": cop,
            "matches": m,
            "fp": f,
            "fn": n,
            "match_rate": trunc4(rate),
            "exercised": exercised,
            "perfect_match": exercised and not diverging,
            "diverging": diverging,
            "style_label": style_label,
        })

    total_fp = len(fp)
    total_fn = len(fn)
    print(f" {len(matches)} matches, {total_fp} FP, {total_fn} FN", file=sys.stderr)

    if verbose:
        for c in by_cop_list:
            if c["diverging"]:
                print(f"    {c['cop']}: FP={c['fp']} FN={c['fn']}", file=sys.stderr)

    return {"style_label": style_label, "by_cop": by_cop_list}


def main():
    parser = argparse.ArgumentParser(
        description="Run nitrocop and RuboCop on the synthetic project and compare results."
    )
    parser.add_argument("--verbose", action="store_true", help="Print per-cop detail table")
    parser.add_argument("--output", type=str, default=None,
                        help="Output JSON path (default: synthetic-results.json in script dir)")
    args = parser.parse_args()

    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_dir = os.path.join(script_dir, "project")
    rubocop_yml = os.path.join(project_dir, ".rubocop.yml")
    gemfile = os.path.join(script_dir, "Gemfile")
    nitrocop_binary = os.path.join(script_dir, "..", "..", "target", "release", "nitrocop")
    output_path = args.output or os.path.join(script_dir, "synthetic-results.json")

    if not os.path.isfile(nitrocop_binary):
        print(f"Error: nitrocop binary not found at {nitrocop_binary}\n"
              f"Run 'cargo build --release' first.", file=sys.stderr)
        sys.exit(1)

    target_cops = TARGET_COPS
    target_set = set(target_cops)
    print(f"Synthetic corpus: {len(target_cops)} cops targeted", file=sys.stderr)

    # ── Run nitrocop ──
    print("Running nitrocop...", end="", file=sys.stderr, flush=True)
    nc_result = subprocess.run(
        [nitrocop_binary, "--preview", "--no-cache", "--format", "json",
         "--config", rubocop_yml, "."],
        capture_output=True, text=True, cwd=project_dir,
    )
    if not nc_result.stdout.strip():
        print(f"\nError: nitrocop produced no output.\nstderr: {nc_result.stderr}", file=sys.stderr)
        sys.exit(1)
    try:
        nc_data = json.loads(nc_result.stdout)
    except json.JSONDecodeError as e:
        print(f"\nError: failed to parse nitrocop JSON: {e}", file=sys.stderr)
        sys.exit(1)
    nc_all = parse_nitrocop_json(nc_data, project_dir)
    nc_offenses = {o for o in nc_all if o[2] in target_set}
    print(f" {len(nc_offenses)} offenses (of {len(nc_all)} total)", file=sys.stderr)

    # ── Run RuboCop ──
    print("Running RuboCop...", end="", file=sys.stderr, flush=True)
    rc_env = os.environ.copy()
    rc_env["BUNDLE_GEMFILE"] = gemfile
    rc_result = subprocess.run(
        ["bundle", "exec", "rubocop", "--config", rubocop_yml, "--format", "json", "."],
        capture_output=True, text=True, env=rc_env, cwd=project_dir,
    )
    if not rc_result.stdout.strip():
        print(f"\nError: RuboCop produced no output.\nstderr: {rc_result.stderr}", file=sys.stderr)
        sys.exit(1)
    try:
        rc_data = json.loads(rc_result.stdout)
    except json.JSONDecodeError as e:
        print(f"\nError: failed to parse RuboCop JSON: {e}", file=sys.stderr)
        sys.exit(1)
    rc_all = parse_rubocop_json(rc_data, project_dir)
    rc_offenses = {o for o in rc_all if o[2] in target_set}
    print(f" {len(rc_offenses)} offenses (of {len(rc_all)} total)", file=sys.stderr)

    # ── Compare ──
    matches_set = nc_offenses & rc_offenses
    fp_set = nc_offenses - rc_offenses
    fn_set = rc_offenses - nc_offenses

    by_cop_matches = defaultdict(int)
    by_cop_fp = defaultdict(int)
    by_cop_fn = defaultdict(int)
    by_cop_fp_examples = defaultdict(list)
    by_cop_fn_examples = defaultdict(list)

    for filepath, line, cop in matches_set:
        by_cop_matches[cop] += 1
    for filepath, line, cop in fp_set:
        by_cop_fp[cop] += 1
        by_cop_fp_examples[cop].append(f"{filepath}:{line}")
    for filepath, line, cop in fn_set:
        by_cop_fn[cop] += 1
        by_cop_fn_examples[cop].append(f"{filepath}:{line}")

    by_cop = []
    for cop in sorted(target_set | set(by_cop_matches) | set(by_cop_fp) | set(by_cop_fn)):
        m = by_cop_matches.get(cop, 0)
        fp = by_cop_fp.get(cop, 0)
        fn = by_cop_fn.get(cop, 0)
        total = m + fp + fn
        rate = m / total if total > 0 else 1.0
        exercised = total > 0
        diverging = fp + fn > 0
        by_cop.append({
            "cop": cop,
            "matches": m,
            "fp": fp,
            "fn": fn,
            "match_rate": trunc4(rate),
            "exercised": exercised,
            "perfect_match": exercised and not diverging,
            "diverging": diverging,
            "fp_examples": sorted(by_cop_fp_examples.get(cop, [])),
            "fn_examples": sorted(by_cop_fn_examples.get(cop, [])),
        })

    by_cop.sort(key=lambda x: (-x["fp"] - x["fn"], x["cop"]))

    total_matches = len(matches_set)
    total_fp = len(fp_set)
    total_fn = len(fn_set)
    total_compared = total_matches + total_fp + total_fn
    overall_rate = total_matches / total_compared if total_compared > 0 else 1.0
    exercised_count = sum(1 for c in by_cop if c["exercised"])

    print(file=sys.stderr)
    print(f"Results: {len(by_cop)} cops, {exercised_count} exercised, "
          f"{total_fp} FP, {total_fn} FN", file=sys.stderr)
    print(file=sys.stderr)

    if args.verbose:
        print(f"  {'Cop':<45s} {'Matches':>7s} {'FP':>5s} {'FN':>5s}  Status", file=sys.stderr)
        print("  " + "-" * 44 + "  " + "-" * 7 + "  " + "-" * 4 + "  " + "-" * 4 + "  " + "-" * 15,
              file=sys.stderr)
        for c in sorted(by_cop, key=lambda x: x["cop"]):
            if c["diverging"]:
                status = "FAIL" if c["fp"] > 0 else "FN"
            elif c["exercised"]:
                status = "ok"
            else:
                status = "-- not exercised"
            print(f"  {c['cop']:<45s} {c['matches']:>7d} {c['fp']:>5d} {c['fn']:>5d}  {status}",
                  file=sys.stderr)
        print(file=sys.stderr)
    elif total_fp > 0 or total_fn > 0:
        diverging = [c for c in by_cop if c["diverging"]]
        if diverging:
            print("  Diverging cops:", file=sys.stderr)
            for c in diverging:
                status = "FAIL" if c["fp"] > 0 else "FN"
                print(f"    {c['cop']:<45s} matches={c['matches']} FP={c['fp']} FN={c['fn']}  {status}",
                      file=sys.stderr)
                for ex in c.get("fp_examples", [])[:5]:
                    print(f"      FP: {ex}", file=sys.stderr)
                for ex in c.get("fn_examples", [])[:5]:
                    print(f"      FN: {ex}", file=sys.stderr)
            print(file=sys.stderr)

    # ── Run variant batches ──
    variant_styles = find_variant_cops(target_set)
    if variant_styles:
        print(f"\nVariant runs: {len(variant_styles)} cops with EnforcedStyle alternatives",
              file=sys.stderr)
        variants = run_variant_batches(
            variant_styles, target_set,
            nitrocop_binary, rubocop_yml, project_dir, gemfile, args.verbose,
        )
    else:
        variants = {"batches": []}

    json_output = {
        "source": "synthetic",
        "run_date": datetime.now(timezone.utc).isoformat(),
        "summary": {
            "target_cops": len(target_cops),
            "exercised_cops": exercised_count,
            "total_offenses_compared": total_compared,
            "matches": total_matches,
            "fp": total_fp,
            "fn": total_fn,
            "overall_match_rate": trunc4(overall_rate),
        },
        "by_cop": by_cop,
        "variants": variants,
    }
    Path(output_path).write_text(json.dumps(json_output, indent=2) + "\n")
    print(f"Wrote {output_path}", file=sys.stderr)


if __name__ == "__main__":
    main()
