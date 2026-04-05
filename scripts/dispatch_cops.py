#!/usr/bin/env python3
from __future__ import annotations

"""Dispatch-related corpus tooling.

Public subcommands:
- `task` generates a self-contained agent prompt for a single cop
- `changed` maps a git diff to changed cops
- `tiers` groups diverging cops into dispatch difficulty tiers
- `rank` finds cops that look fixable by agents
- `prior-attempts` collects failed PR attempts for a cop
- `backend` selects a recommended backend for a cop
- `issues-sync` syncs one tracker issue per diverging cop
Usage:
    python3 scripts/dispatch_cops.py task Style/NegatedWhile
    python3 scripts/dispatch_cops.py changed --base origin/main --head HEAD
    python3 scripts/dispatch_cops.py tiers --tier 1 --names
    python3 scripts/dispatch_cops.py rank --json
    python3 scripts/dispatch_cops.py prior-attempts --cop Style/NegatedWhile
    python3 scripts/dispatch_cops.py backend --cop Style/NegatedWhile --binary target/debug/nitrocop
    python3 scripts/dispatch_cops.py issues-sync --binary target/debug/nitrocop
    python3 scripts/dispatch_cops.py issues-sync --department Rails --binary target/debug/nitrocop
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

# Allow importing shared helpers from scripts/
SCRIPTS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS_DIR))
from shared.corpus_artifacts import download_corpus_results as _download_corpus

PROJECT_ROOT = SCRIPTS_DIR.parent
TIERS_JSON = PROJECT_ROOT / "src" / "resources" / "tiers.json"


def _cop_tier(cop: str) -> str:
    """Return 'stable' or 'preview' for the given cop name."""
    try:
        data = json.loads(TIERS_JSON.read_text())
        return data.get("overrides", {}).get(cop, data.get("default_tier", "preview"))
    except Exception:
        return "preview"


# Department → vendor gem directory mapping
DEPT_TO_VENDOR = {
    "Bundler": "vendor/rubocop",
    "FactoryBot": "vendor/rubocop",
    "Gemspec": "vendor/rubocop",
    "Layout": "vendor/rubocop",
    "Lint": "vendor/rubocop",
    "Metrics": "vendor/rubocop",
    "Migration": "vendor/rubocop",
    "Naming": "vendor/rubocop",
    "Security": "vendor/rubocop",
    "Style": "vendor/rubocop",
    "Performance": "vendor/rubocop-performance",
    "Rails": "vendor/rubocop-rails",
    "RSpec": "vendor/rubocop-rspec",
    "RSpecRails": "vendor/rubocop-rspec_rails",
    "Rake": "vendor/rubocop-rake",
}

# Department name → directory name in vendor gem (usually lowercase)
DEPT_TO_DIR = {
    "FactoryBot": "factory_bot",
    "RSpec": "rspec",
    "RSpecRails": "rspec_rails",
}

# Cops that consume the shared VariableForce engine. Changes to
# src/cop/variable_force/ should trigger corpus checks for all of these.
VARIABLE_FORCE_CONSUMERS = {
    "Lint/ShadowedArgument",
    "Lint/UnusedMethodArgument",
    "Lint/UnusedBlockArgument",
    "Lint/UnderscorePrefixedVariableName",
    "Lint/ShadowingOuterLocalVariable",
    "Lint/UselessAssignment",
    "Style/InfiniteLoop",
    "Style/MapIntoArray",
    "RSpec/LeakyLocalVariable",
    "Rails/SaveBang",
}

# Shared modules consumed by multiple cops. When a shared module file changes,
# corpus checks trigger for all its consumers. Keyed by module filename (without .rs).
# Matches against src/cop/shared/{key}.rs and src/cop/{dept}/{key}.rs.
# The "variable_force" key is special — it matches the entire directory.
SHARED_MODULE_CONSUMERS: dict[str, set[str]] = {
    "method_complexity": {
        "Metrics/CyclomaticComplexity",
        "Metrics/PerceivedComplexity",
    },
    "hash_subset": {
        "Style/HashExcept",
        "Style/HashSlice",
    },
    "hash_transform_method": {
        "Style/HashTransformKeys",
        "Style/HashTransformValues",
    },
    "multiline_literal_brace_layout": {
        "Layout/MultilineArrayBraceLayout",
        "Layout/MultilineHashBraceLayout",
        "Layout/MultilineMethodCallBraceLayout",
        "Layout/MultilineMethodDefinitionBraceLayout",
    },
    "trailing_comma": {
        "Style/TrailingCommaInArguments",
        "Style/TrailingCommaInArrayLiteral",
        "Style/TrailingCommaInHashLiteral",
    },
    "method_identifier_predicates": {
        "Layout/FirstArgumentIndentation",
        "Layout/MultilineMethodCallIndentation",
        "Layout/SpaceBeforeFirstArg",
        "Lint/AmbiguousBlockAssociation",
        "Lint/AmbiguousRange",
        "Lint/ParenthesesAsGroupedExpression",
        "Lint/RequireParentheses",
        "Lint/SymbolConversion",
        "Lint/UnreachableLoop",
        "Lint/UselessSetterCall",
        "Metrics/AbcSize",
        "Naming/MethodName",
        "Naming/PredicateMethod",
        "Performance/IoReadlines",
        "Rails/SaveBang",
        "Style/MethodCallWithArgsParentheses",
        "Style/MultilineTernaryOperator",
        "Style/Next",
        "Style/RedundantParentheses",
    },
    "literal_predicates": {
        "Lint/LiteralAsCondition",
        "Style/InfiniteLoop",
    },
    "access_modifier_predicates": {
        "Layout/AccessModifierIndentation",
        "Layout/ClassStructure",
        "Layout/EmptyLinesAroundAccessModifier",
        "Layout/IndentationConsistency",
        "Layout/IndentationWidth",
        "Lint/IneffectiveAccessModifier",
        "Lint/UselessAccessModifier",
        "Lint/UselessConstantScoping",
        "Lint/UselessMethodDefinition",
        "Rails/I18nLazyLookup",
        "Style/AccessModifierDeclarations",
        "Style/DocumentationMethod",
        "Style/ModuleFunction",
    },
    "method_dispatch_predicates": {
        "Bundler/DuplicatedGroup",
        "FactoryBot/AssociationStyle",
        "Layout/ClassStructure",
        "Lint/NumberConversion",
        "Lint/ParenthesesAsGroupedExpression",
        "Lint/RedundantRequireStatement",
        "Lint/UselessAccessModifier",
        "Lint/UselessConstantScoping",
        "Lint/UselessRuby2Keywords",
        "Rails/ActionControllerFlashBeforeRender",
        "Rails/BulkChangeTable",
        "Rails/DefaultScope",
        "Rails/FilePath",
        "Rails/HttpPositionalArguments",
        "Rails/PluckId",
        "Rails/StrongParametersExpect",
        "Rails/ThreeStateBooleanColumn",
        "Rails/TransactionExitStatement",
        "RSpec/AroundBlock",
        "RSpec/ExpectActual",
        "RSpec/MessageExpectation",
        "RSpec/MessageSpies",
        "RSpec/ReceiveNever",
        "RSpec/SkipBlockInsideExample",
        "RSpec/StubbedMock",
        "RSpec/SubjectStub",
        "RSpec/VoidExpect",
        "Security/Eval",
        "Style/ArrayFirstLast",
        "Style/HashExcept",
        "Style/HashSlice",
        "Style/SafeNavigation",
        "Style/StaticClass",
    },
    "predicate_operator_predicates": {
        "Lint/AmbiguousOperatorPrecedence",
        "Style/RedundantParentheses",
        "Style/UnlessLogicalOperators",
    },
}

# Department PascalCase → snake_case directory name in src/cop/ and tests/fixtures/
# Only needed for departments where pascal_to_snake() gives the wrong result
DEPT_TO_SRC_DIR = {
    "RSpec": "rspec",
    "RSpecRails": "rspec_rails",
    "FactoryBot": "factory_bot",
}

# Known Prism pitfalls: keyword → note
PRISM_PITFALLS = {
    "as_hash_node": (
        "hash splits into HashNode (literal `{}`) and KeywordHashNode "
        "(keyword args `foo(a: 1)`). If you handle one, check if you need the other."
    ),
    "as_constant_read_node": (
        "const splits into ConstantReadNode (simple `Foo`) and ConstantPathNode "
        "(qualified `Foo::Bar`). If you handle one, check if you need the other."
    ),
    "as_begin_node": (
        "begin is overloaded: explicit `begin..end` is BeginNode, implicit method "
        "body is StatementsNode. Check which one(s) your cop needs."
    ),
}

MAX_DIAGNOSTIC_DETAILS_PER_KIND = 8
MAX_ADDITIONAL_EXAMPLES_PER_KIND = 5

COP_TRACKER_MARKER = "nitrocop-cop-tracker"
PR_ISSUE_MARKER = "nitrocop-cop-issue"
ISSUE_TITLE_PREFIX = "[cop] "
TRACKER_LABEL = "type:cop-issue"
STATE_BACKLOG = "state:backlog"
STATE_PR_OPEN = "state:pr-open"
STATE_BLOCKED = "state:blocked"
STATE_LABELS = [STATE_BACKLOG, STATE_PR_OPEN, STATE_BLOCKED]
DIFFICULTY_LABELS = {
    "simple": "difficulty:simple",
    "medium": "difficulty:medium",
    "complex": "difficulty:complex",
    "config-only": "difficulty:config-only",
}
LABEL_COLORS = {
    TRACKER_LABEL: "1d76db",
    STATE_BACKLOG: "fbca04",
    STATE_PR_OPEN: "0e8a16",
    STATE_BLOCKED: "b60205",
    "difficulty:simple": "0e8a16",
    "difficulty:medium": "fbca04",
    "difficulty:complex": "d73a4a",
    "difficulty:config-only": "c5def5",
}
TITLE_RE = re.compile(r"^\[bot\] Fix (?P<cop>.+?)(?: \(retry\))?$")
TRACKER_RE = re.compile(r"<!--\s*" + re.escape(COP_TRACKER_MARKER) + r":\s*(.*?)\s*-->")
PR_ISSUE_RE = re.compile(r"<!--\s*" + re.escape(PR_ISSUE_MARKER) + r":\s*(.*?)\s*-->")
MAX_GH_PAGE = 500


def pascal_to_snake(name: str) -> str:
    """Convert PascalCase to snake_case. E.g. NegatedWhile -> negated_while."""
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


def parse_cop_name(cop: str) -> tuple[str, str, str]:
    """Parse 'Department/CopName' into (department, cop_name, snake_name)."""
    if "/" not in cop:
        print(f"Error: cop must be in Department/CopName format, got: {cop}",
              file=sys.stderr)
        sys.exit(1)
    dept, name = cop.split("/", 1)
    return dept, name, pascal_to_snake(name)


def make_issue_title(cop: str) -> str:
    return f"{ISSUE_TITLE_PREFIX}{cop}"


def parse_marker_fields(body: str, pattern: re.Pattern[str]) -> dict[str, str]:
    match = pattern.search(body or "")
    if not match:
        return {}
    fields: dict[str, str] = {}
    for token in match.group(1).split():
        if "=" not in token:
            continue
        key, value = token.split("=", 1)
        fields[key.strip()] = value.strip()
    return fields


def extract_cop_from_issue(issue: dict) -> str | None:
    body_fields = parse_marker_fields(issue.get("body", ""), TRACKER_RE)
    cop = body_fields.get("cop")
    if cop:
        return cop
    title = issue.get("title", "")
    if title.startswith(ISSUE_TITLE_PREFIX):
        return title[len(ISSUE_TITLE_PREFIX):].strip()
    return None


def extract_cop_from_pr(pr: dict) -> str | None:
    body_fields = parse_marker_fields(pr.get("body", ""), PR_ISSUE_RE)
    cop = body_fields.get("cop")
    if cop:
        return cop
    match = TITLE_RE.match(pr.get("title", "").strip())
    if match:
        return match.group("cop")
    return None


def extract_issue_number_from_pr(pr: dict) -> int | None:
    body_fields = parse_marker_fields(pr.get("body", ""), PR_ISSUE_RE)
    issue_number = body_fields.get("number")
    if issue_number and issue_number.isdigit():
        return int(issue_number)
    return None


def read_file_safe(path: Path) -> str | None:
    """Read a file, return None if it doesn't exist."""
    if path.exists():
        return path.read_text(errors="replace")
    return None


def dept_dir_name(dept: str) -> str:
    """Get the directory name for a department in src/cop/ and tests/fixtures/."""
    return DEPT_TO_SRC_DIR.get(dept, pascal_to_snake(dept))


def find_rust_source(dept: str, snake: str) -> Path:
    """Find the cop's Rust source file."""
    return PROJECT_ROOT / "src" / "cop" / dept_dir_name(dept) / f"{snake}.rs"


def find_vendor_ruby_source(dept: str, snake: str) -> Path | None:
    """Find the RuboCop Ruby implementation."""
    vendor_gem = DEPT_TO_VENDOR.get(dept)
    if not vendor_gem:
        return None
    dept_dir = DEPT_TO_DIR.get(dept, dept.lower())
    path = PROJECT_ROOT / vendor_gem / "lib" / "rubocop" / "cop" / dept_dir / f"{snake}.rb"
    return path if path.exists() else None


def find_vendor_spec(dept: str, snake: str) -> Path | None:
    """Find the RuboCop spec file."""
    vendor_gem = DEPT_TO_VENDOR.get(dept)
    if not vendor_gem:
        return None
    dept_dir = DEPT_TO_DIR.get(dept, dept.lower())
    path = PROJECT_ROOT / vendor_gem / "spec" / "rubocop" / "cop" / dept_dir / f"{snake}_spec.rb"
    return path if path.exists() else None


def extract_spec_excerpts(spec_text: str, max_blocks: int = 20) -> str:
    """Extract expect_offense and expect_no_offenses blocks from a spec file.

    Returns a condensed version with just the key test cases."""
    blocks = []
    lines = spec_text.splitlines()
    i = 0
    while i < len(lines) and len(blocks) < max_blocks:
        line = lines[i]
        # Match expect_offense or expect_no_offenses with heredoc
        if re.search(r"expect_(offense|no_offenses)\s*\(\s*<<~?\s*['\"]?(\w+)", line):
            # Find the heredoc delimiter
            m = re.search(r"<<~?\s*['\"]?(\w+)", line)
            if m:
                delimiter = m.group(1)
                block_lines = [line]
                i += 1
                while i < len(lines):
                    block_lines.append(lines[i])
                    if lines[i].strip() == delimiter:
                        break
                    i += 1
                blocks.append("\n".join(block_lines))
        # Also capture it/context/describe blocks for context
        elif re.match(r"\s*(it|context|describe)\s+['\"]", line):
            blocks.append(line)
        i += 1
    return "\n\n".join(blocks) if blocks else "(no expect_offense blocks found)"


def find_fixtures(dept: str, snake: str) -> tuple[str | None, str | None]:
    """Find offense.rb and no_offense.rb fixture files."""
    fixture_dir = PROJECT_ROOT / "tests" / "fixtures" / "cops" / dept_dir_name(dept) / snake
    offense = read_file_safe(fixture_dir / "offense.rb")
    no_offense = read_file_safe(fixture_dir / "no_offense.rb")
    # Check scenario layout
    if offense is None:
        offense_dir = fixture_dir / "offense"
        if offense_dir.is_dir():
            parts = []
            for f in sorted(offense_dir.glob("*.rb")):
                parts.append(f"# --- {f.name} ---\n{f.read_text(errors='replace')}")
            if parts:
                offense = "\n\n".join(parts)
    return offense, no_offense


def get_corpus_data(
    cop: str,
    input_path: Path | None,
    *,
    require_examples: bool = False,
) -> dict:
    """Get FP/FN data from corpus-results.json.

    Returns dict with counts and raw example lists.

    If require_examples is True, fails hard when only summary data is available
    (e.g. docs/corpus.md fallback) — used in CI task generation where the
    pre-diagnostic gate needs real per-file examples.
    """
    if input_path is None:
        try:
            input_path, _, _ = _download_corpus()
        except Exception as e:
            print(f"Warning: could not download corpus data: {e}", file=sys.stderr)
            if require_examples:
                print(
                    "Error: task generation requires full corpus-results.json with "
                    "per-file examples, but download failed.",
                    file=sys.stderr,
                )
                sys.exit(1)
            return {
                "fp": 0,
                "fn": 0,
                "matches": 0,
                "fp_examples": [],
                "fn_examples": [],
                "repo_breakdown": {},
                "available": False,
            }

    data = json.loads(input_path.read_text())
    source = data.get("_source", "")
    if require_examples and "summary only" in source:
        print(
            f"Error: corpus data is summary-only (from {source}). "
            "Task generation requires full corpus-results.json with per-file "
            "examples for pre-diagnostic. Check that GH_TOKEN has actions:read "
            "scope and corpus-oracle artifacts are not expired.",
            file=sys.stderr,
        )
        sys.exit(1)

    data = json.loads(input_path.read_text())
    by_cop = data.get("by_cop", [])
    by_repo_cop = data.get("by_repo_cop", {})
    cop_entry = next((e for e in by_cop if e["cop"] == cop), None)

    if cop_entry is None:
        return {
            "fp": 0,
            "fn": 0,
            "matches": 0,
            "fp_examples": [],
            "fn_examples": [],
            "repo_breakdown": {},
            "available": True,
        }

    repo_breakdown = {}
    for repo_id, cops in by_repo_cop.items():
        if cop not in cops:
            continue
        entry = cops[cop]
        repo_breakdown[repo_id] = {
            "fp": entry.get("fp", 0),
            "fn": entry.get("fn", 0),
        }

    return {
        "fp": cop_entry.get("fp", 0),
        "fn": cop_entry.get("fn", 0),
        "matches": cop_entry.get("matches", 0),
        "fp_examples": cop_entry.get("fp_examples", []),
        "fn_examples": cop_entry.get("fn_examples", []),
        "repo_breakdown": repo_breakdown,
        "available": True,
        "variants": [],  # populated later by caller if needed
    }


def load_variant_data_for_cop(cop: str, run_id: int | str | None = None) -> list[dict]:
    """Load per-style variant divergence data for a cop.

    Returns list of {style_label, matches, fp, fn} for each variant
    that includes the cop. Empty list if no variant data available.
    """
    from shared.corpus_artifacts import get_variant_results_path
    if run_id is None:
        # Try to get run_id from the latest download
        try:
            _, rid, _ = _download_corpus()
            run_id = rid
        except Exception:
            return []
    path = get_variant_results_path(int(run_id)) if run_id else None
    if not path:
        return []
    try:
        data = json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return []
    variants = []
    for batch in data.get("batches", []):
        for cop_entry in batch.get("by_cop", []):
            if cop_entry.get("cop") == cop:
                variants.append({
                    "style_label": cop_entry.get("style_label", batch.get("name", "?")),
                    "matches": cop_entry.get("matches", 0),
                    "fp": cop_entry.get("fp", 0),
                    "fn": cop_entry.get("fn", 0),
                })
    return variants


def load_variant_only_candidates(
    data: dict,
    department: str | None,
    already_ranked: set[str],
) -> list[dict]:
    """Find cops that are perfect in default config but diverge in variants.

    Returns rank-compatible dicts for cops not already in the ranked set.
    These cops have code_bugs=0 (no default examples to diagnose) but
    variant_fp/variant_fn > 0.
    """
    by_cop_index = {e["cop"]: e for e in data.get("by_cop", [])}
    candidates = []

    # Load variant data from the latest corpus run
    try:
        _, run_id, _ = _download_corpus()
    except Exception:
        return []
    from shared.corpus_artifacts import get_variant_results_path
    variant_path = get_variant_results_path(int(run_id)) if run_id else None
    if not variant_path:
        return []

    try:
        vdata = json.loads(variant_path.read_text())
    except (json.JSONDecodeError, OSError):
        return []

    # Aggregate variant FP/FN per cop
    variant_totals: dict[str, dict] = {}
    for batch in vdata.get("batches", []):
        for cop_entry in batch.get("by_cop", []):
            cn = cop_entry.get("cop", "")
            if not cn:
                continue
            vt = variant_totals.setdefault(cn, {"fp": 0, "fn": 0})
            vt["fp"] += cop_entry.get("fp", 0)
            vt["fn"] += cop_entry.get("fn", 0)

    for cop_name, vt in sorted(variant_totals.items()):
        if cop_name in already_ranked:
            continue
        if department and not cop_name.startswith(department + "/"):
            continue
        if vt["fp"] + vt["fn"] == 0:
            continue
        # Only include cops that are perfect in default (otherwise they'd
        # already be in the ranked set from the default loop)
        default_entry = by_cop_index.get(cop_name, {})
        if default_entry.get("fp", 0) + default_entry.get("fn", 0) > 0:
            continue

        candidates.append({
            "cop": cop_name,
            "fp": 0,
            "fn": 0,
            "variant_fp": vt["fp"],
            "variant_fn": vt["fn"],
            "code_bugs": 0,
            "config_issues": 0,
            "matches": default_entry.get("matches", 0),
        })

    return candidates


def safe_get_corpus_data(
    cop: str,
    *,
    input_path: Path | None = None,
) -> dict | None:
    try:
        return get_corpus_data(cop, input_path)
    except Exception as exc:
        print(f"Warning: could not load corpus data: {exc}", file=sys.stderr)
        return None


def _normalize_example(ex) -> tuple[str, str, list[str] | None]:
    """Normalize an example to (loc_string, message, embedded_context)."""
    if isinstance(ex, dict):
        return ex.get("loc", ""), ex.get("msg", ""), ex.get("src")
    return ex, "", None


def _parse_example_loc(loc: str) -> tuple[str, str, int] | None:
    """Parse `repo_id: path/to/file.rb:line` into components."""
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


def _top_repos(
    repo_breakdown: dict[str, dict[str, int]], kind: str, limit: int = 3,
) -> list[tuple[str, int]]:
    repos = [
        (repo_id, counts.get(kind, 0))
        for repo_id, counts in repo_breakdown.items()
        if counts.get(kind, 0) > 0
    ]
    repos.sort(key=lambda item: (-item[1], item[0]))
    return repos[:limit]


def _sample_examples_for_repo(examples: list, repo_id: str, limit: int = 1) -> list[str]:
    matches = []
    for ex in examples:
        loc, _msg, _src = _normalize_example(ex)
        parsed = _parse_example_loc(loc)
        if parsed is None:
            continue
        ex_repo_id, filepath, line = parsed
        if ex_repo_id != repo_id:
            continue
        matches.append(f"{filepath}:{line}")
        if len(matches) >= limit:
            break
    return matches


def build_start_here_section(cop: str, corpus: dict) -> str:
    """Build a compact investigation guide from corpus hotspots."""
    repo_breakdown = corpus.get("repo_breakdown", {})
    fp_examples = corpus.get("fp_examples", [])
    fn_examples = corpus.get("fn_examples", [])

    top_fp_repos = _top_repos(repo_breakdown, "fp")
    top_fn_repos = _top_repos(repo_breakdown, "fn")

    if not top_fp_repos and not top_fn_repos and not fp_examples and not fn_examples:
        return ""

    lines = [
        "## Start Here",
        "",
        "Use the existing corpus data to focus on the most concentrated regressions first.",
        "",
        "Helpful local commands:",
        f"- `python3 scripts/investigate_cop.py {cop} --repos-only`",
        f"- `python3 scripts/investigate_cop.py {cop} --context`",
        f"- `python3 scripts/verify_cop_locations.py {cop}`",
        "",
    ]

    if top_fp_repos:
        lines.append("Top FP repos:")
        for repo_id, count in top_fp_repos:
            samples = _sample_examples_for_repo(fp_examples, repo_id)
            suffix = f" — example `{samples[0]}`" if samples else ""
            lines.append(f"- `{repo_id}` ({count} FP){suffix}")
        lines.append("")

    if top_fn_repos:
        lines.append("Top FN repos:")
        for repo_id, count in top_fn_repos:
            samples = _sample_examples_for_repo(fn_examples, repo_id)
            suffix = f" — example `{samples[0]}`" if samples else ""
            lines.append(f"- `{repo_id}` ({count} FN){suffix}")
        lines.append("")

    if fp_examples:
        lines.append("Representative FP examples:")
        for ex in fp_examples[:3]:
            loc, msg, _src = _normalize_example(ex)
            msg_suffix = f" — {msg}" if msg else ""
            lines.append(f"- `{loc}`{msg_suffix}")
        lines.append("")

    if fn_examples:
        lines.append("Representative FN examples:")
        for ex in fn_examples[:3]:
            loc, msg, _src = _normalize_example(ex)
            msg_suffix = f" — {msg}" if msg else ""
            lines.append(f"- `{loc}`{msg_suffix}")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def corpus_total(corpus: dict | None) -> int:
    if not corpus:
        return 0
    return corpus.get("fp", 0) + corpus.get("fn", 0)


def affected_repo_count(corpus: dict | None) -> int:
    if not corpus:
        return 0
    return sum(
        1
        for counts in corpus.get("repo_breakdown", {}).values()
        if counts.get("fp", 0) > 0 or counts.get("fn", 0) > 0
    )



def _extract_source_lines(src: list[str]) -> tuple[list[str], str | None, int | None]:
    """Extract clean source lines from corpus src context.

    Returns (all_source_lines, offense_line, offense_line_index)."""
    source_lines = []
    offense_line = None
    offense_line_idx = None
    for i, s in enumerate(src):
        is_offense = s.strip().startswith(">>>")
        cleaned = re.sub(r"^(>>>\s*)?\s*\d+:\s?", "", s)
        source_lines.append(cleaned)
        if is_offense:
            offense_line = cleaned
            offense_line_idx = i
    return source_lines, offense_line, offense_line_idx


# Ruby block-opening keywords/patterns and their AST significance
_ENCLOSING_PATTERNS = [
    (r'^\s*BEGIN\s*\{', "BEGIN {} block (Prism: PreExecutionNode)"),
    (r'^\s*END\s*\{', "END {} block (Prism: PostExecutionNode)"),
    (r'^\s*class\s+', "class body"),
    (r'^\s*module\s+', "module body"),
    (r'^\s*def\s+', "method body"),
    (r'^\s*if\s+', "if branch"),
    (r'^\s*unless\s+', "unless branch"),
    (r'^\s*while\s+', "while loop"),
    (r'^\s*until\s+', "until loop"),
    (r'^\s*begin\b', "begin block"),
    (r'^\s*rescue\b', "rescue block"),
    (r'^\s*ensure\b', "ensure block"),
    (r'^\s*case\s+', "case expression"),
    (r'.*\bdo\s*(\|.*\|)?\s*$', "block (do..end)"),
    (r'.*\{\s*(\|.*\|)?\s*$', "block ({..})"),
]


def _find_enclosing_structure(
    source_lines: list[str], offense_line_idx: int | None,
) -> str | None:
    """Identify the enclosing Ruby structure around the offense line.

    Scans backwards from the offense line looking for block-opening keywords
    to help the agent understand what context makes a FP/FN different."""
    chain = _find_enclosing_structures(source_lines, offense_line_idx)
    return chain[0] if chain else None


def _find_enclosing_structures(
    source_lines: list[str], offense_line_idx: int | None,
    *, real_line_offset: int = 0,
) -> list[str]:
    """Identify ALL enclosing Ruby structures around the offense line.

    Returns a list from innermost to outermost.  *real_line_offset* is added
    to the 0-based index when building the ``(line N: ...)`` label so that
    callers using a full-file slice can report real line numbers."""
    if offense_line_idx is None or offense_line_idx == 0:
        return []

    results: list[str] = []
    current_indent = len(source_lines[offense_line_idx]) - len(
        source_lines[offense_line_idx].lstrip()
    )

    # Scan backwards, collecting every enclosing block at decreasing indent
    for i in range(offense_line_idx - 1, -1, -1):
        line = source_lines[i]
        stripped = line.lstrip()
        if not stripped or stripped.startswith("#"):
            continue
        line_indent = len(line) - len(stripped)
        if line_indent < current_indent:
            real_line = i + 1 + real_line_offset  # 1-indexed
            matched = False
            for pattern, desc in _ENCLOSING_PATTERNS:
                if re.match(pattern, line):
                    results.append(f"{desc} (line {real_line}: `{stripped.rstrip()}`)")
                    matched = True
                    break
            if not matched:
                results.append(
                    f"enclosing line {real_line}: `{stripped.rstrip()}`"
                )
            current_indent = line_indent
            if line_indent == 0:
                break  # reached top-level

    return results


def _get_prism_node_type(source: str, line: int) -> str | None:
    """Use Ruby's Prism to identify the AST node type at a given line.

    Returns a string like 'RangeNode > IntegerNode' or None if Ruby/Prism
    is not available."""
    ruby_script = f"""
require 'prism'
result = Prism.parse({source!r})
target_line = {line}

# Walk the AST to find the deepest node at the target line
def find_at_line(node, line, path=[])
  return nil unless node.respond_to?(:child_nodes)
  loc = node.location rescue nil
  if loc && loc.start_line == line
    path << node.class.name.split('::').last
  end
  node.child_nodes.compact.each {{ |c| find_at_line(c, line, path) }}
  path
end

types = find_at_line(result.value, target_line)
puts types.join(' > ') unless types.empty?
"""
    try:
        proc = subprocess.run(
            ["ruby", "-e", ruby_script],
            capture_output=True, text=True, timeout=10,
        )
        out = proc.stdout.strip()
        return out if out else None
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None


_MANIFEST_CACHE: dict[str, dict] | None = None


def _load_manifest() -> dict[str, dict]:
    """Load bench/corpus/manifest.jsonl into {repo_id: {repo_url, sha}}."""
    global _MANIFEST_CACHE
    if _MANIFEST_CACHE is not None:
        return _MANIFEST_CACHE
    manifest_path = PROJECT_ROOT / "bench" / "corpus" / "manifest.jsonl"
    result: dict[str, dict] = {}
    try:
        for line in manifest_path.read_text().splitlines():
            line = line.strip()
            if not line:
                continue
            entry = json.loads(line)
            rid = entry.get("id", "")
            if rid:
                result[rid] = {
                    "repo_url": entry.get("repo_url", ""),
                    "sha": entry.get("sha", ""),
                }
    except (FileNotFoundError, json.JSONDecodeError):
        pass
    _MANIFEST_CACHE = result
    return result


def _fetch_full_file(repo_url: str, sha: str, filepath: str) -> str | None:
    """Fetch a single file from GitHub via raw.githubusercontent.com.

    Returns file content or None on failure."""
    # repo_url is like https://github.com/owner/repo
    parts = repo_url.rstrip("/").split("/")
    if len(parts) < 2:
        return None
    owner, repo = parts[-2], parts[-1]
    url = f"https://raw.githubusercontent.com/{owner}/{repo}/{sha}/{filepath}"
    try:
        import urllib.request
        with urllib.request.urlopen(url, timeout=10) as resp:
            return resp.read().decode("utf-8", errors="replace")
    except Exception:
        return None


def _run_nitrocop_on_file(
    binary_path: Path, file_content: str, cop: str, filename: str = "test.rb",
) -> list[dict]:
    """Run nitrocop on arbitrary file content, return offenses list.

    ``filename`` can be a relative path (e.g. ``spec/models/foo_spec.rb``);
    intermediate directories are created so that cops with Include patterns
    (like ``**/*_spec.rb`` or ``**/spec/**/*.rb``) can match.
    """
    tmp_dir = tempfile.mkdtemp(prefix="nitrocop_diag_ff_")
    tmp_path = os.path.join(tmp_dir, filename)
    os.makedirs(os.path.dirname(tmp_path), exist_ok=True)
    try:
        with open(tmp_path, "w") as f:
            f.write(file_content)
        return _run_nitrocop(binary_path, tmp_dir, cop, filename)
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


BASELINE_CONFIG = PROJECT_ROOT / "bench" / "corpus" / "baseline_rubocop.yml"


def _run_nitrocop(
    binary_path: Path, cwd: str, cop: str = "", filename: str = "test.rb",
) -> list[dict]:
    """Run nitrocop on a file in the given directory, return offenses list.

    Uses the corpus baseline config (same as the corpus oracle) instead of
    --force-default-config. This ensures pre-diagnostics match the actual
    corpus environment: all cops enabled, plugins loaded, TargetRubyVersion
    set, etc. Without this, cops whose behavior depends on these settings
    are misclassified as "config-only" because the pre-diagnostic can't
    reproduce the FP/FN.
    """
    cmd = [str(binary_path), "--preview", "--no-cache", "--format", "json"]
    if BASELINE_CONFIG.exists():
        cmd.extend(["--config", str(BASELINE_CONFIG)])
    else:
        cmd.append("--force-default-config")
    if cop:
        cmd.extend(["--only", cop])
    cmd.append(filename)
    proc = subprocess.run(
        cmd, capture_output=True, text=True, timeout=15, cwd=cwd,
    )
    output_text = proc.stdout.strip()
    if output_text:
        try:
            output = json.loads(output_text)
            return output.get("offenses", [])
        except json.JSONDecodeError:
            pass
    return []


def run_diagnostic(
    binary_path: Path, cop: str,
    fp_examples: list, fn_examples: list,
) -> list[dict]:
    """Run nitrocop on extracted FP/FN source to classify each as code bug vs config issue.

    For each example with source context, creates a temp .rb file from the
    extracted source lines and runs nitrocop with --force-default-config to
    test detection in isolation.

    For FN: detected → config/context issue; not detected → code bug.
    For FP: flagged → confirmed code bug; not flagged → context-dependent.
    """
    results = []
    for kind, examples in [("fn", fn_examples), ("fp", fp_examples)]:
        for ex in examples[:15]:
            loc, msg, src = _normalize_example(ex)
            if not src:
                results.append({
                    "kind": kind, "loc": loc, "msg": msg,
                    "diagnosed": False, "reason": "no source context",
                })
                continue

            source_lines, offense_line, offense_line_idx = _extract_source_lines(src)
            if not source_lines:
                results.append({
                    "kind": kind, "loc": loc, "msg": msg,
                    "diagnosed": False, "reason": "empty source",
                })
                continue

            # Write temp file in its own directory (nitrocop needs a project root).
            # Preserve the original repo-relative path so that cops with
            # Include patterns (e.g., Rails cops matching **/test/**/*.rb or
            # RSpec cops matching **/*_spec.rb) can match the file.
            tmp_dir = tempfile.mkdtemp(prefix="nitrocop_diag_")
            parsed_loc = _parse_example_loc(loc)
            snippet_rel_path = parsed_loc[1] if parsed_loc else "test.rb"
            tmp_path = os.path.join(tmp_dir, snippet_rel_path)
            os.makedirs(os.path.dirname(tmp_path), exist_ok=True)
            with open(tmp_path, "w") as f:
                f.write("\n".join(source_lines) + "\n")

            try:
                offenses = _run_nitrocop(
                    binary_path, tmp_dir, cop, snippet_rel_path,
                )

                # If no offenses with full context (may have parse errors from
                # truncated source), retry with just the offense line
                if not offenses and offense_line is not None:
                    with open(tmp_path, "w") as f:
                        f.write(offense_line + "\n")
                    offenses = _run_nitrocop(
                        binary_path, tmp_dir, cop, snippet_rel_path,
                    )

                detected = len(offenses) > 0

                # Generate test snippet from offense data
                test_snippet = None
                if offense_line is not None:
                    expected_line_num = (offense_line_idx + 1) if offense_line_idx is not None else 1
                    if detected and offenses:
                        # Find offense on the expected line
                        matching = [o for o in offenses if o.get("line") == expected_line_num]
                        o = matching[0] if matching else offenses[0]
                        col = o.get("column", 1) - 1  # 1-indexed → 0-indexed
                        omsg = o.get("message", msg)
                        test_snippet = f"{offense_line}\n{' ' * col}^ {cop}: {omsg}"
                    else:
                        test_snippet = f"{offense_line}\n^ {cop}: {msg}"

                # Identify enclosing structure for context
                enclosing = _find_enclosing_structure(source_lines, offense_line_idx)

                # Identify Prism node type at the offense location
                node_type = None
                if offense_line_idx is not None:
                    node_type = _get_prism_node_type(
                        "\n".join(source_lines), offense_line_idx + 1,
                    )

                # --- Full-file fallback ---
                # Fetch the real file and re-test.  This serves two purposes:
                # 1. When the snippet doesn't detect: distinguish "snippet too
                #    narrow" from "genuine code bug".
                # 2. When the snippet DOES detect an FN: confirm whether the
                #    full file also detects.  If not, the full-file context
                #    (e.g., rubocop:disable comments, begin/rescue wrappers)
                #    suppresses detection — this is a real bug in how nitrocop
                #    handles that context, not just a config issue.
                full_file_detected: bool | None = None
                full_file_enclosing: str | None = None
                full_file_context: str | None = None
                diagnosis_note: str | None = None

                parsed = _parse_example_loc(loc)
                if parsed:
                    repo_id, filepath, real_line = parsed
                    manifest = _load_manifest()
                    entry = manifest.get(repo_id)
                    if entry and entry["repo_url"] and entry["sha"]:
                        content = _fetch_full_file(
                            entry["repo_url"], entry["sha"], filepath,
                        )
                        if content is not None:
                            ff_offenses = _run_nitrocop_on_file(
                                binary_path, content, cop,
                                filename=filepath,
                            )
                            ff_hit = any(
                                o.get("line") == real_line
                                for o in ff_offenses
                            )
                            full_file_detected = ff_hit

                            # Build enclosing chain from the full file
                            full_lines = content.splitlines()
                            if 0 < real_line <= len(full_lines):
                                chain = _find_enclosing_structures(
                                    full_lines, real_line - 1,
                                    real_line_offset=0,
                                )
                                if chain:
                                    full_file_enclosing = " > ".join(chain)

                                # Provide broader context (30 lines before)
                                ctx_start = max(0, real_line - 1 - 30)
                                ctx_end = min(len(full_lines), real_line + 7)
                                ctx_lines = []
                                for ci in range(ctx_start, ctx_end):
                                    marker = ">>> " if ci == real_line - 1 else "    "
                                    ctx_lines.append(
                                        f"{marker}{ci + 1:>5}: {full_lines[ci]}"
                                    )
                                full_file_context = "\n".join(ctx_lines)

                            if not detected and ff_hit and kind == "fn":
                                diagnosis_note = (
                                    "Snippet too narrow — offense is detected "
                                    "in the full file but not in the ±7-line "
                                    "extract. The enclosing structure chain "
                                    "shows the missing context."
                                )
                            elif not detected and ff_hit and kind == "fp":
                                diagnosis_note = (
                                    "Snippet too narrow — FP reproduces in "
                                    "the full file but not in the ±7-line "
                                    "extract. This is a real code/config bug, "
                                    "not just context-dependent."
                                )
                            elif detected and not ff_hit and kind == "fn":
                                diagnosis_note = (
                                    "Snippet detects but full file does not — "
                                    "something in the full-file context "
                                    "(rubocop:disable comment, begin/rescue "
                                    "wrapper, config interaction) suppresses "
                                    "detection. This is a real code bug, not "
                                    "a project config issue."
                                )

                results.append({
                    "kind": kind, "loc": loc, "msg": msg,
                    "diagnosed": True, "detected": detected,
                    "offense_line": offense_line,
                    "test_snippet": test_snippet,
                    "enclosing": enclosing,
                    "node_type": node_type,
                    "source_context": "\n".join(source_lines),
                    "full_file_detected": full_file_detected,
                    "full_file_enclosing": full_file_enclosing,
                    "full_file_context": full_file_context,
                    "diagnosis_note": diagnosis_note,
                })
            except Exception as e:
                results.append({
                    "kind": kind, "loc": loc, "msg": msg,
                    "diagnosed": False, "reason": str(e),
                })
            finally:
                shutil.rmtree(tmp_dir, ignore_errors=True)

    return results


def format_corpus_section(
    cop: str, corpus: dict, diagnostics: list[dict] | None,
) -> str:
    """Format the corpus FP/FN section, optionally enriched with diagnostic results."""
    fp_examples = corpus["fp_examples"]
    fn_examples = corpus["fn_examples"]

    if not fp_examples and not fn_examples:
        return "(no FP/FN examples in corpus data)"

    if diagnostics:
        return _format_with_diagnostics(cop, diagnostics, fp_examples, fn_examples)
    return _format_without_diagnostics(cop, fp_examples, fn_examples)


def _format_with_diagnostics(
    cop: str, diagnostics: list[dict],
    fp_examples: list, fn_examples: list,
) -> str:
    lines = []

    fn_diags = [d for d in diagnostics if d["kind"] == "fn"]
    fp_diags = [d for d in diagnostics if d["kind"] == "fp"]
    fn_diagnosed = [d for d in fn_diags if d.get("diagnosed")]
    fp_diagnosed = [d for d in fp_diags if d.get("diagnosed")]
    fn_undiagnosed = [d for d in fn_diags if not d.get("diagnosed")]
    fp_undiagnosed = [d for d in fp_diags if not d.get("diagnosed")]

    # Summary counts — full-file fallback can reclassify snippet misses.
    # A FN is a "code bug" when:
    #   - Neither snippet nor full-file detects it (classic code bug), OR
    #   - Snippet detects but full-file does NOT (context-sensitive bug:
    #     something in the real file suppresses detection, e.g. a
    #     rubocop:disable comment nitrocop mishandles, a begin/rescue
    #     wrapper, or a config interaction bug).
    fn_code_bugs = sum(
        1 for d in fn_diagnosed
        if (not d.get("detected") and not d.get("full_file_detected"))
        or (d.get("detected") and d.get("full_file_detected") is False)
    )
    fn_context_narrow = sum(
        1 for d in fn_diagnosed
        if not d.get("detected") and d.get("full_file_detected")
    )
    fn_config = sum(
        1 for d in fn_diagnosed
        if d.get("detected") and d.get("full_file_detected") is not False
    )
    fp_code_bugs = sum(
        1 for d in fp_diagnosed
        if d.get("detected") or d.get("full_file_detected")
    )
    fp_context_narrow = sum(
        1 for d in fp_diagnosed
        if not d.get("detected") and d.get("full_file_detected")
    )
    fp_config = sum(
        1 for d in fp_diagnosed
        if not d.get("detected") and not d.get("full_file_detected")
    )

    lines.append("### Diagnosis Summary")
    lines.append("Each example was tested by running nitrocop on the extracted source in isolation")
    lines.append("and against the full file fetched from GitHub to determine if the issue is a")
    lines.append("code bug or config issue.")
    lines.append("Note: source context is truncated and may not parse perfectly. If a diagnosis")
    lines.append("seems wrong (e.g., your test passes immediately for a 'CODE BUG'), treat it as")
    lines.append("a config/context issue instead.\n")
    if fn_diags:
        parts = []
        if fn_code_bugs:
            parts.append(f"{fn_code_bugs} code bug(s)")
        if fn_context_narrow:
            parts.append(f"{fn_context_narrow} context-dependent (detected in full file only)")
        if fn_config:
            parts.append(f"{fn_config} config/context issue(s)")
        lines.append(f"- **FN:** {', '.join(parts)}" if parts else "- **FN:** 0 issues")
    if fp_diags:
        fp_parts = []
        if fp_code_bugs:
            fp_parts.append(f"{fp_code_bugs} confirmed code bug(s)")
        if fp_context_narrow:
            fp_parts.append(f"{fp_context_narrow} context-dependent (detected in full file only)")
        if fp_config:
            fp_parts.append(f"{fp_config} context-dependent")
        lines.append(f"- **FP:** {', '.join(fp_parts)}" if fp_parts else "- **FP:** 0 issues")
    if fn_diagnosed and fn_undiagnosed:
        no_context = sum(1 for d in fn_undiagnosed if d.get("reason") == "no source context")
        extra = len(fn_undiagnosed) - no_context
        if no_context:
            lines.append(
                f"- Omitted {no_context} pre-diagnostic FN example(s) with no source context "
                "because diagnosed FN examples were available"
            )
        if extra:
            lines.append(f"- Omitted {extra} additional undiagnosed FN example(s) for brevity")
    if fp_diagnosed and fp_undiagnosed:
        no_context = sum(1 for d in fp_undiagnosed if d.get("reason") == "no source context")
        extra = len(fp_undiagnosed) - no_context
        if no_context:
            lines.append(
                f"- Omitted {no_context} pre-diagnostic FP example(s) with no source context "
                "because diagnosed FP examples were available"
            )
        if extra:
            lines.append(f"- Omitted {extra} additional undiagnosed FP example(s) for brevity")
    lines.append("")

    # FN details
    fn_display = fn_diagnosed[:MAX_DIAGNOSTIC_DETAILS_PER_KIND]
    if not fn_display:
        fn_display = fn_undiagnosed[:MAX_ADDITIONAL_EXAMPLES_PER_KIND]
    for i, d in enumerate(fn_display, 1):
        lines.append(f"### FN #{i}: `{d['loc']}`")
        if d.get("diagnosed"):
            if d.get("detected") and d.get("full_file_detected") is False:
                lines.append("**DETECTED in snippet but NOT in full file — CODE BUG**")
                lines.append("The cop detects the pattern in a small snippet but fails")
                lines.append("when the full file is present. Something in the file context")
                lines.append("(rubocop:disable comment, begin/rescue wrapper, or config")
                lines.append("interaction) incorrectly suppresses detection.")
                if d.get("diagnosis_note"):
                    lines.append(f"\n> {d['diagnosis_note']}")
                if d.get("full_file_enclosing"):
                    lines.append(f"\n**Full-file enclosing chain:** {d['full_file_enclosing']}")
            elif d.get("detected"):
                lines.append("**DETECTED in isolation — CONFIG/CONTEXT issue**")
                lines.append("The cop correctly detects this pattern with default config.")
                lines.append("The corpus FN is caused by the target repo's configuration")
                lines.append("(Include/Exclude patterns, cop disabled, file outside scope,")
                lines.append("or `rubocop:disable` comment). Investigate config resolution.")
            elif d.get("full_file_detected"):
                lines.append("**DETECTED in full file only — CONTEXT-DEPENDENT**")
                lines.append("The ±7-line snippet is too narrow to reproduce this offense.")
                lines.append("The offense depends on file-level structure (e.g., an enclosing")
                lines.append("`if`/`while`/`unless` far above the offense line).")
                if d.get("diagnosis_note"):
                    lines.append(f"\n> {d['diagnosis_note']}")
                if d.get("full_file_enclosing"):
                    lines.append(f"\n**Full-file enclosing chain:** {d['full_file_enclosing']}")
                    lines.append("Read the chain from left (innermost) to right (outermost).")
                    lines.append("The outermost structure is likely the condition that makes")
                    lines.append("this assignment an offense. Your test fixture must include")
                    lines.append("that enclosing structure.")
            else:
                lines.append("**NOT DETECTED — CODE BUG**")
                lines.append("The cop fails to detect this pattern. Fix the detection logic.")
                if d.get("full_file_enclosing"):
                    lines.append(f"\n**Full-file enclosing chain:** {d['full_file_enclosing']}")
                elif d.get("enclosing"):
                    lines.append(f"\n**Enclosing structure:** {d['enclosing']}")
                    lines.append("The offense is inside this structure — the cop may need")
                    lines.append("to handle this context to detect the pattern.")
                if d.get("node_type"):
                    lines.append(f"\n**Prism AST at offense line:** `{d['node_type']}`")
            lines.append(f"\nMessage: `{d['msg']}`")
            if d.get("test_snippet"):
                lines.append("\nReady-made test snippet (add to offense.rb, adjust `^` count):")
                lines.append(f"```ruby\n{d['test_snippet']}\n```")
            # Prefer full-file context when available (broader view)
            if d.get("full_file_context"):
                lines.append("\nFull file context (30 lines before offense):")
                lines.append(f"```\n{d['full_file_context']}\n```")
            elif d.get("source_context"):
                lines.append("\nFull source context:")
                lines.append(f"```ruby\n{d['source_context']}\n```")
        else:
            lines.append(f"(could not diagnose: {d.get('reason', 'unknown')})")
            lines.append(f"Message: `{d['msg']}`")
        lines.append("")
    if len(fn_diagnosed) > len(fn_display):
        omitted = len(fn_diagnosed) - len(fn_display)
        lines.append(f"_Omitted {omitted} additional diagnosed FN example(s) for brevity._")
        lines.append("")
    elif not fn_diagnosed and len(fn_undiagnosed) > len(fn_display):
        omitted = len(fn_undiagnosed) - len(fn_display)
        lines.append(f"_Omitted {omitted} additional undiagnosed FN example(s) for brevity._")
        lines.append("")

    # FP details
    fp_display = fp_diagnosed[:MAX_DIAGNOSTIC_DETAILS_PER_KIND]
    if not fp_display:
        fp_display = fp_undiagnosed[:MAX_ADDITIONAL_EXAMPLES_PER_KIND]
    for i, d in enumerate(fp_display, 1):
        lines.append(f"### FP #{i}: `{d['loc']}`")
        if d.get("diagnosed"):
            if d.get("detected"):
                lines.append("**CONFIRMED false positive — CODE BUG**")
                lines.append("nitrocop incorrectly flags this pattern in isolation.")
                lines.append("Fix the detection logic to not flag this.")
                if d.get("enclosing"):
                    lines.append(f"\n**Enclosing structure:** {d['enclosing']}")
                    lines.append("The offense is inside this structure — this is likely WHY")
                    lines.append("RuboCop does not flag it. Your fix should detect this context.")
                if d.get("node_type"):
                    lines.append(f"\n**Prism AST at offense line:** `{d['node_type']}`")
                    lines.append("This shows the Prism node types at the flagged location.")
                if d.get("source_context"):
                    lines.append("\nFull source context (add relevant parts to no_offense.rb):")
                    lines.append(f"```ruby\n{d['source_context']}\n```")
                elif d.get("offense_line"):
                    lines.append("\nAdd to no_offense.rb:")
                    lines.append(f"```ruby\n{d['offense_line']}\n```")
            elif d.get("full_file_detected"):
                lines.append("**DETECTED in full file only — CODE BUG (snippet too narrow)**")
                lines.append("The ±7-line snippet is too narrow to reproduce this FP.")
                lines.append("nitrocop flags this in the full file but RuboCop does not.")
                lines.append("This is a real FP that needs a code or config fix.")
                if d.get("diagnosis_note"):
                    lines.append(f"\n> {d['diagnosis_note']}")
                if d.get("full_file_enclosing"):
                    lines.append(f"\n**Full-file enclosing chain:** {d['full_file_enclosing']}")
            else:
                lines.append("**NOT REPRODUCED — CONFIG/CONTEXT issue**")
                lines.append("nitrocop does not flag this in isolation or in the full file")
                lines.append("(with default config). The FP is caused by the target repo's")
                lines.append("config (e.g., different Max value, Include/Exclude patterns).")
                if d.get("source_context"):
                    lines.append("\nSource context:")
                    lines.append(f"```ruby\n{d['source_context']}\n```")
            lines.append(f"\nMessage: `{d['msg']}`")
            # Prefer full-file context when available (broader view)
            if d.get("full_file_context"):
                lines.append("\nFull file context (30 lines before offense):")
                lines.append(f"```\n{d['full_file_context']}\n```")
        else:
            lines.append(f"(could not diagnose: {d.get('reason', 'unknown')})")
            lines.append(f"Message: `{d['msg']}`")
        lines.append("")
    if len(fp_diagnosed) > len(fp_display):
        omitted = len(fp_diagnosed) - len(fp_display)
        lines.append(f"_Omitted {omitted} additional diagnosed FP example(s) for brevity._")
        lines.append("")
    elif not fp_diagnosed and len(fp_undiagnosed) > len(fp_display):
        omitted = len(fp_undiagnosed) - len(fp_display)
        lines.append(f"_Omitted {omitted} additional undiagnosed FP example(s) for brevity._")
        lines.append("")

    # Additional un-diagnosed examples
    undiag_fn = [] if fn_diagnosed else fn_examples[len(fn_diags):]
    undiag_fp = [] if fp_diagnosed else fp_examples[len(fp_diags):]
    if undiag_fn or undiag_fp:
        lines.append("### Additional examples (not pre-diagnosed)\n")
        for ex in undiag_fn[:MAX_ADDITIONAL_EXAMPLES_PER_KIND]:
            loc, msg, src = _normalize_example(ex)
            lines.append(f"- FN: `{loc}` — {msg}")
            if src:
                lines.append("  ```ruby")
                for s in src:
                    lines.append(f"  {s}")
                lines.append("  ```")
        for ex in undiag_fp[:MAX_ADDITIONAL_EXAMPLES_PER_KIND]:
            loc, msg, src = _normalize_example(ex)
            lines.append(f"- FP: `{loc}` — {msg}")
            if src:
                lines.append("  ```ruby")
                for s in src:
                    lines.append(f"  {s}")
                lines.append("  ```")

    return "\n".join(lines)


def _format_without_diagnostics(cop: str, fp_examples: list, fn_examples: list) -> str:
    """Format FP/FN examples without diagnostic enrichment (no --binary)."""
    lines = []

    if fp_examples:
        lines.append(f"### False Positives ({len(fp_examples)} examples)")
        lines.append("nitrocop flags these but RuboCop does not:\n")
        for ex in fp_examples[:30]:
            loc, msg, src = _normalize_example(ex)
            lines.append(f"- `{loc}`")
            if msg:
                lines.append(f"  Message: {msg}")
            if src:
                lines.append("  ```ruby")
                for s in src:
                    lines.append(f"  {s}")
                lines.append("  ```")

    if fn_examples:
        lines.append(f"\n### False Negatives ({len(fn_examples)} examples)")
        lines.append("RuboCop flags these but nitrocop does not:\n")
        for ex in fn_examples[:30]:
            loc, msg, src = _normalize_example(ex)
            lines.append(f"- `{loc}`")
            if msg:
                lines.append(f"  Message: {msg}")
            if src:
                lines.append("  ```ruby")
                for s in src:
                    lines.append(f"  {s}")
                lines.append("  ```")

    return "\n".join(lines) if lines else "(no example locations in corpus data)"


def detect_prism_pitfalls(rust_source: str) -> list[str]:
    """Detect relevant Prism pitfalls from the cop's Rust source."""
    notes = []
    for keyword, note in PRISM_PITFALLS.items():
        if keyword in rust_source:
            notes.append(note)
    return notes


def generate_task(
    cop: str,
    input_path: Path | None = None,
    binary_path: Path | None = None,
) -> str:
    """Generate the full task markdown for a cop."""
    dept, name, snake = parse_cop_name(cop)
    dept_snake = dept_dir_name(dept)

    # Gather all sources
    rust_path = find_rust_source(dept, snake)
    rust_source = read_file_safe(rust_path)
    if rust_source is None:
        print(f"Error: Rust source not found at {rust_path}", file=sys.stderr)
        sys.exit(1)

    ruby_path = find_vendor_ruby_source(dept, snake)
    spec_path = find_vendor_spec(dept, snake)

    offense_fixture, no_offense_fixture = find_fixtures(dept, snake)
    corpus = get_corpus_data(
        cop, input_path,
        require_examples=binary_path is not None,
    )

    # Run pre-diagnostic if binary is available
    diagnostics = None
    if binary_path and binary_path.exists():
        print(f"Running pre-diagnostic with {binary_path}...", file=sys.stderr)
        diagnostics = run_diagnostic(
            binary_path, cop,
            corpus["fp_examples"], corpus["fn_examples"],
        )
        n_diag = sum(1 for d in diagnostics if d.get("diagnosed"))
        print(f"  Diagnosed {n_diag}/{len(diagnostics)} examples", file=sys.stderr)

    # Classify diagnosed examples
    has_code_bugs = False
    has_config_issues = False
    if diagnostics:
        for d in diagnostics:
            if not d.get("diagnosed"):
                continue
            if d["kind"] == "fn" and not d.get("detected") and not d.get("full_file_detected"):
                has_code_bugs = True
            elif d["kind"] == "fn" and d.get("detected") and d.get("full_file_detected") is False:
                # Snippet detects but full file doesn't → context-sensitive
                # code bug (e.g. rubocop:disable handling, begin/rescue).
                has_code_bugs = True
            elif d["kind"] == "fn" and (d.get("detected") or d.get("full_file_detected")):
                has_config_issues = True
            elif d["kind"] == "fp" and (d.get("detected") or d.get("full_file_detected")):
                has_code_bugs = True
            elif d["kind"] == "fp" and not d.get("detected") and not d.get("full_file_detected"):
                has_config_issues = True

    # Detect Prism pitfalls
    pitfalls = detect_prism_pitfalls(rust_source)

    # Build the task document
    parts = []

    # Header
    # Load variant data early for title/framing
    variants = load_variant_data_for_cop(cop)
    diverging_variants = [v for v in variants if v["fp"] + v["fn"] > 0]
    default_perfect = corpus.get("fp", 0) + corpus.get("fn", 0) == 0
    variant_fp = sum(v["fp"] for v in diverging_variants)
    variant_fn = sum(v["fn"] for v in diverging_variants)

    if default_perfect and diverging_variants:
        parts.append(f"# Fix {cop} — variant divergence: {variant_fp:,} FP, {variant_fn:,} FN\n")
    else:
        parts.append(f"# Fix {cop} — {corpus['fp']} FP, {corpus['fn']} FN\n")

    # Instructions
    if default_perfect and diverging_variants:
        focus = "variant style divergence"
        focus_detail = (
            "default config is perfect — the issue is with non-default "
            "`EnforcedStyle` values"
        )
    elif corpus["fp"] > corpus["fn"]:
        focus = "FP"
        focus_detail = "nitrocop flags code RuboCop does not"
    elif corpus["fn"] > corpus["fp"]:
        focus = "FN"
        focus_detail = "RuboCop flags code nitrocop misses"
    else:
        focus = "both FP and FN"
        focus_detail = "both directions"
    parts.append(f"""## Instructions

You are fixing ONE cop in **nitrocop**, a Rust Ruby linter that uses Prism for parsing.

**Current state:** {corpus['matches']:,} matches, {corpus['fp']} false positives, {corpus['fn']} false negatives.
**Focus on:** {focus} ({focus_detail}).

**⚠ {corpus['matches']:,} existing matches must not regress.** Validate with `check_cop.py` before committing.

### Workflow
1. Read the **Pre-diagnostic Results** and **Corpus FP/FN Examples** sections below first
2. **Verify with RuboCop first** (for FP fixes): before writing any code, confirm RuboCop's
   behavior on BOTH the specific FP case AND the general pattern:
   ```bash
   echo '<specific FP case>' > /tmp/test.rb
   BUNDLE_GEMFILE=bench/corpus/Gemfile BUNDLE_PATH=bench/corpus/vendor/bundle \\
     bundle exec rubocop --only {cop} /tmp/test.rb
   ```
   **Important:** Always use `BUNDLE_GEMFILE=bench/corpus/Gemfile BUNDLE_PATH=bench/corpus/vendor/bundle bundle exec rubocop` — bare `rubocop` won't find the correct version.
   If RuboCop flags the general pattern, your fix must be narrow enough to not suppress it.
3. Add a test case FIRST:
   - FN fix: add the missed pattern to `tests/fixtures/cops/{dept_snake}/{snake}/offense.rb` with `^` annotation
   - FP fix: add the false-positive pattern to `tests/fixtures/cops/{dept_snake}/{snake}/no_offense.rb`
4. Verify test fails: `cargo test --lib -- cop::{dept_snake}::{snake}`
5. Fix `src/cop/{dept_snake}/{snake}.rs`
6. Verify test passes: `cargo test --lib -- cop::{dept_snake}::{snake}`
7. **Validate against corpus** (REQUIRED before finishing):
   ```bash
   python3 scripts/check_cop.py {cop} --rerun --clone --sample 15
   ```
   If this reports FP or FN regression, your fix is too broad — narrow it down.
8. Add a `///` doc comment on the cop struct documenting what you found and fixed
9. Leave your changes unstaged — the workflow commits for you

### Fixture Format
Mark offenses with `^` markers on the line AFTER the offending source line.
The `^` characters must align with the offending columns. The message format is `{cop}: <message text>`.
See the **Current Fixture** sections below for real examples from this cop.""")

    # Add diagnostic-aware guidance
    # Check if config issues are FP-only (likely config-resolution bugs in nitrocop)
    # vs FN-only (likely repo disables the cop — nothing to fix)
    fp_config_only = (
        diagnostics and has_config_issues and not has_code_bugs
        and any(d["kind"] == "fp" and d.get("diagnosed") for d in diagnostics)
        and not any(d["kind"] == "fn" and d.get("diagnosed") and not d.get("detected") for d in diagnostics)
    )
    if diagnostics and has_config_issues and not has_code_bugs and fp_config_only:
        parts.append(f"""
### Config-resolution FPs — the cop logic is correct but config handling differs
Pre-diagnostic shows nitrocop detects these patterns correctly in isolation (with default
config). The FPs only appear when running against target repos with custom `.rubocop.yml`.
This means nitrocop is reading the repo's config differently than RuboCop does.

**The detection logic is correct — the bug is in config resolution.**

Do NOT add `no_offense.rb` fixtures for these patterns (they ARE offenses under default
config). Instead:
1. Use `python3 scripts/check_cop.py {cop} --rerun --clone --sample 15` to confirm the
   FPs reproduce when running against real repos (this uses the repo's config, not defaults)
2. Investigate `src/config/` for how this cop's config is loaded and applied
3. Common causes: `Max` value not read from repo config, `Exclude` patterns not applied,
   `Enabled: false` in a department-level override not respected, inherited configs
   (e.g., `inherit_from`) not followed
4. If you find a config-resolution fix, apply it and verify with `check_cop.py --rerun`
5. Do NOT use `verify_cop_locations.py` unless the corpus repos are cloned locally —
   it silently reports "fixed" when the repos are not on disk""")
    elif diagnostics and has_config_issues and not has_code_bugs:
        parts.append("""
### IMPORTANT: This is a config/context issue, NOT a detection bug
Pre-diagnostic shows nitrocop already detects all FP/FN patterns correctly in isolation.
The corpus mismatches are caused by configuration differences in target repos.

**Do NOT loop trying to fix detection logic — the detection code is correct.**

Instead:
1. Investigate why the cop doesn't fire (FN) or fires incorrectly (FP) in the target
   repo's config context. Common causes:
   - Include/Exclude patterns in the cop's config not matching the file path
   - The cop being disabled by the target repo's `.rubocop.yml`
   - `# rubocop:disable` comments in the source file
   - File path patterns (e.g., spec files excluded by default)
2. Look at `src/config/` for how config affects this cop
3. If you can fix the config resolution, do so. Otherwise document your findings as a
   `///` comment on the cop struct and leave your changes as-is.""")

    elif diagnostics and has_config_issues and has_code_bugs:
        parts.append("""
### Mixed issues: some code bugs, some config issues
Pre-diagnostic shows SOME patterns are correctly detected in isolation (config issues)
and SOME are genuinely missed (code bugs). See the per-example diagnosis below.

- For examples marked **CODE BUG**: follow the standard TDD workflow
- For examples marked **CONFIG/CONTEXT**: investigate config resolution, not detection logic""")

    parts.append(f"""
### If your test passes immediately
If you add a test case and it passes without code changes, the corpus mismatch is
caused by config/context differences, not a detection bug.
**Do NOT loop** trying to make the test fail. Instead:
1. Investigate config resolution (Include/Exclude, cop enablement, disable comments)
2. The fix is likely in `src/config/` or the cop's config handling, not detection logic
3. If you cannot determine the root cause within 5 minutes, document your findings as
   a `///` comment on the cop struct and leave your changes as-is

### Do NOT make doc-only changes when CODE BUGs were reported
If the pre-diagnostic classified examples as **CODE BUG** but you cannot reproduce them
or find a code fix, do NOT fall back to only adding `///` doc comments. The
pre-diagnostic ran your binary against real corpus files — if it says CODE BUG, the
mismatch is real. Re-read the pre-diagnostic output and double-check your test covers
the exact pattern (receiver shape, nesting depth, argument structure, modifier context).

If after thorough investigation you still cannot fix the code, **exit without making changes**.
The workflow will close the PR cleanly and the issue stays open for a future retry.
Doc-only changes add noise to git history without closing the FP/FN gap.

### When the pre-diagnostic contradicts existing doc comments
If the pre-diagnostic classifies an example as **CODE BUG** but existing `///` doc
comments on the cop struct say it's "not real" or a "corpus artifact," the
pre-diagnostic takes precedence — it ran your current binary against the actual
corpus source. Prior conclusions may have been based on incorrect manual
verification. Investigate the example fresh rather than deferring to the doc comment.

### CRITICAL: Avoid regressions in the opposite direction
When fixing FPs, your change MUST NOT suppress legitimate detections. When fixing FNs,
your change MUST NOT flag code that RuboCop accepts. A fix that eliminates a few issues
in one direction but introduces hundreds in the other is a catastrophic regression.

**Before exempting a category of patterns**, verify with RuboCop that the general case
is still an offense:
```bash
rubocop --only {cop} /tmp/test.rb
```
If RuboCop flags the general pattern but not your specific case, the difference is in
a narrow context (e.g., enclosing structure, receiver type, argument count) — your fix
must target that specific context, not the broad category.

**Rule of thumb:** if your fix adds an early `return` or `continue` that skips a whole
node type, operator class, or naming pattern, it's probably too broad. Prefer adding a
condition that matches the SPECIFIC differentiating context.

### Rules
- Only modify `src/cop/{dept_snake}/{snake}.rs` and `tests/fixtures/cops/{dept_snake}/{snake}/`
- Run `cargo test --lib -- cop::{dept_snake}::{snake}` to verify your fix (do NOT run the full test suite)
- Run `python3 scripts/check_cop.py {cop} --rerun --clone --sample 15` before finishing to catch regressions
- Do NOT touch unrelated files
- Do NOT use `git stash`
- Do NOT push — you do not have push permission; the workflow handles pushing after you exit

### If You Cannot Fix It
If you exit without changes, your final message is posted to the tracker issue for future
agents. Include: (1) what you changed and why it didn't work, with exact `check_cop.py`
regression numbers, (2) which corpus repos/patterns regressed, (3) why the approach failed,
(4) what a correct fix would need to handle differently. Vague findings like "tried a fix
but it regressed" are not acceptable — be specific enough that the next agent won't repeat
your work.
""")

    # Prism pitfall notes
    if pitfalls:
        parts.append("### Prism Notes")
        for note in pitfalls:
            parts.append(f"- {note}")
        parts.append("")

    # Tier/preview note — agents must know that preview-tier cops are skipped
    # by default CLI invocations, so `cargo run -- file.rb` won't show offenses
    # unless `--preview` is passed.
    tier = _cop_tier(cop)
    if tier == "preview":
        parts.append(f"""### ⚠ Preview-tier cop
`{cop}` is in the **preview** tier. Plain `cargo run -- file.rb` will NOT
report offenses for this cop unless you pass `--preview`:
```bash
cargo run --quiet -- --preview --no-cache --force-default-config --only {cop} /tmp/test.rb
```
Unit tests (`cargo test --lib`) are NOT affected — they bypass tier gating.
If the CLI reports 0 offenses but your unit test passes, you almost certainly
forgot `--preview`. Do NOT rewrite the cop architecture to work around this.
""")

    # Fixtures inline — they're small and provide essential context for the agent
    if offense_fixture:
        parts.append(f"## Current Fixture: offense.rb\n`tests/fixtures/cops/{dept_snake}/{snake}/offense.rb`\n")
        parts.append(f"```ruby\n{offense_fixture}```\n")

    if no_offense_fixture:
        parts.append(f"## Current Fixture: no_offense.rb\n`tests/fixtures/cops/{dept_snake}/{snake}/no_offense.rb`\n")
        parts.append(f"```ruby\n{no_offense_fixture}```\n")

    # Include variant fixture files if they exist
    fixture_dir = PROJECT_ROOT / "tests" / "fixtures" / "cops" / dept_snake / snake
    if fixture_dir.exists():
        variant_fixtures = sorted(
            f for f in fixture_dir.iterdir()
            if f.suffix == ".rb" and f.name not in ("offense.rb", "no_offense.rb")
            and ("offense." in f.name or "no_offense." in f.name)
        )
        if variant_fixtures:
            parts.append("## Existing Variant Fixtures\n")
            for vf in variant_fixtures:
                content = read_file_safe(vf)
                if content:
                    rel = vf.relative_to(PROJECT_ROOT)
                    parts.append(f"### `{rel}`\n")
                    parts.append(f"```ruby\n{content}```\n")

    # Large source files — just reference paths, the agent can read them
    parts.append("## Key Source Files\n")
    parts.append(f"- Rust implementation: `{rust_path.relative_to(PROJECT_ROOT)}`")
    if ruby_path:
        parts.append(f"- RuboCop Ruby source (ground truth): `{ruby_path.relative_to(PROJECT_ROOT)}`")
    if spec_path:
        parts.append(f"- RuboCop test excerpts: `{spec_path.relative_to(PROJECT_ROOT)}`")
    parts.append("")
    parts.append("Read these files before making changes.\n")

    # For variant-only cops (default perfect), put variant section first
    # as it's the primary task. For mixed cops, put it after default data.
    def _variant_section() -> list[str]:
        if not diverging_variants:
            return []
        lines = ["## Style Variant Divergence\n"]
        if default_perfect:
            lines.append(
                "**The default config is already perfect.** Your task is fixing "
                "non-default style variants listed below.\n"
            )
        else:
            lines.append(
                "This cop also has divergence under non-default style configurations.\n"
            )
        lines.append("| Style | Matches | FP | FN |")
        lines.append("|-------|--------:|---:|---:|")
        for v in sorted(diverging_variants, key=lambda x: x["fp"] + x["fn"], reverse=True):
            lines.append(
                f"| {v['style_label']} | {v['matches']:,} | {v['fp']:,} | {v['fn']:,} |"
            )
        lines.append("")
        lines.append("### How to fix variant divergence\n")
        lines.append(
            "1. **Find the config read:** Look for `config.get_str(\"EnforcedStyle\", ...)` or "
            "similar `Enforced*` reads in the Rust source. The cop should branch on the "
            "configured style value.\n"
        )
        lines.append(
            "2. **Check the RuboCop Ruby source** for how each style value changes behavior. "
            "The ground truth is in the vendored Ruby cop file.\n"
        )
        lines.append(
            "3. **Spot-check with RuboCop** to understand the expected behavior:\n"
            "```bash\n"
            "echo '<test code>' > /tmp/test.rb\n"
            "BUNDLE_GEMFILE=bench/corpus/Gemfile BUNDLE_PATH=bench/corpus/vendor/bundle \\\n"
            f"  bundle exec rubocop --only {cop} /tmp/test.rb\n"
            "```\n"
        )
        lines.append(
            "4. **Add fixture tests** for each broken variant. Use the `# nitrocop-config:` "
            "directive at the top of fixture files:\n"
            "```ruby\n"
            "# nitrocop-config: EnforcedStyle: comma\n"
            "# ^^^ offense annotation here\n"
            "trailing_comma_example(a, b,)\n"
            "```\n"
        )
        lines.append(
            "5. **Validate locally** with `check_cop.py --style`:\n"
            f"```bash\n"
            f"python3 scripts/check_cop.py {cop} --rerun --clone --sample 15 "
            f"--style EnforcedStyle=<value>\n"
            f"```\n"
        )
        lines.append(
            "6. **All variants must pass.** The CI gate (`--check-variants`) runs ALL "
            "variant styles automatically. Fixing one variant must not break another.\n"
        )
        return lines

    start_here = build_start_here_section(cop, corpus)
    if start_here:
        parts.append(start_here)

    # Variant-only cops: variant section comes first (it's the primary task)
    if default_perfect and diverging_variants:
        parts.extend(_variant_section())

    # Pre-diagnostic results (high-value: before source code)
    if diagnostics:
        parts.append("## Pre-diagnostic Results\n")
        parts.append(format_corpus_section(cop, corpus, diagnostics))
        parts.append("")

    # Corpus data (without diagnostics, for fallback)
    if not diagnostics and not default_perfect:
        parts.append("## Corpus FP/FN Examples\n")
        parts.append(format_corpus_section(cop, corpus, None))
        parts.append("")

    # Mixed cops: variant section comes after default data
    if not default_perfect and diverging_variants:
        parts.extend(_variant_section())

    return "\n".join(parts)


DEPT_OVERRIDES = {
    "rspec": "RSpec",
    "rspec_rails": "RSpecRails",
    "factory_bot": "FactoryBot",
}

TIER_THRESHOLDS = {
    1: (1, 50),
    2: (51, 1000),
    3: (1001, 999999),
}


def snake_to_pascal(name: str) -> str:
    return "".join(word.capitalize() for word in name.split("_"))


def dept_snake_to_pascal(name: str) -> str:
    return DEPT_OVERRIDES.get(name, snake_to_pascal(name))


def detect_cops(base: str, head: str) -> list[str]:
    """Find cops whose implementation changed between base and head.

    Only triggers on src/cop/ changes.  Fixture-only changes
    (tests/fixtures/cops/) are validated by cargo test and do not need
    a corpus rerun.
    """
    result = subprocess.run(
        ["git", "diff", "--name-only", f"{base}...{head}"],
        capture_output=True,
        text=True,
        check=True,
    )
    changed = result.stdout.strip().splitlines()

    cops = set()
    for path in changed:
        match = re.match(r"src/cop/([^/]+)/([^/]+)\.rs$", path)
        if not match:
            continue
        dept, name = match.group(1), match.group(2)

        if dept == "variable_force":
            cops.update(VARIABLE_FORCE_CONSUMERS)
        elif name in SHARED_MODULE_CONSUMERS:
            cops.update(SHARED_MODULE_CONSUMERS[name])
        elif dept == "shared" or name == "mod":
            # shared/util.rs, shared/node_type.rs, mod.rs — used by nearly
            # all cops; changes validated by cargo test, skip dispatch.
            pass
        else:
            cops.add(f"{dept_snake_to_pascal(dept)}/{snake_to_pascal(name)}")
        continue

    return sorted(cops)


def load_dispatch_corpus(input_path: Path | None) -> dict:
    if input_path:
        return json.loads(input_path.read_text())
    path, _, _ = _download_corpus()
    return json.loads(path.read_text())


def tier_cops(data: dict) -> tuple[list[dict], dict[int, list[dict]]]:
    cops = []
    for entry in data.get("by_cop", []):
        fp = entry.get("fp", 0)
        fn = entry.get("fn", 0)
        total = fp + fn
        if total == 0:
            continue
        cops.append({
            "cop": entry["cop"],
            "fp": fp,
            "fn": fn,
            "total": total,
            "matches": entry.get("matches", 0),
            "match_rate": entry.get("match_rate", 0),
        })

    cops.sort(key=lambda entry: entry["total"])
    tiers: dict[int, list[dict]] = {1: [], 2: [], 3: []}
    for cop in cops:
        for tier, (low, high) in TIER_THRESHOLDS.items():
            if low <= cop["total"] <= high:
                tiers[tier].append(cop)
                break
    return cops, tiers


def tier_for_total(total: int) -> int:
    for tier, (low, high) in TIER_THRESHOLDS.items():
        if low <= total <= high:
            return tier
    return 3


def total_for_entry(entry: dict) -> int:
    return entry.get("total", entry.get("fp", 0) + entry.get("fn", 0))


def backend_family(backend: str) -> str:
    if backend.startswith("codex-"):
        return "codex"
    if backend.startswith("claude-"):
        return "claude"
    if backend == "minimax":
        return "minimax"
    return backend


def backend_strength(backend: str) -> str:
    if backend.endswith("-hard"):
        return "hard"
    return "normal"


def backend_display_label(backend: str) -> str:
    return f"{backend_family(backend)} / {backend_strength(backend)}"


def should_consider_easy_candidate(
    entry: dict, min_total: int = 3, max_total: int = 15, min_matches: int = 50,
) -> bool:
    total = total_for_entry(entry)
    return min_total <= total <= max_total and entry.get("matches", 0) >= min_matches



def select_backend_for_entry(
    cop: str,
    entry: dict | None,
    *,
    mode: str,
    binary: Path | None,
    prior_prs: list[dict] | None = None,
    issue_difficulty: str | None = None,
    min_total: int = 3,
    max_total: int = 15,
    min_matches: int = 50,
    min_bugs: int = 1,
    precomputed_diagnosis: tuple[int, int] | None = None,
) -> dict[str, object]:
    prior_prs = prior_prs or []
    total = total_for_entry(entry or {})
    tier = tier_for_total(total) if total else 3

    def _result(backend: str, reason: str, code_bugs: int = 0,
                config_issues: int = 0, easy: bool = False) -> dict[str, object]:
        return {
            "backend": backend,
            "reason": reason,
            "tier": tier,
            "code_bugs": code_bugs,
            "config_issues": config_issues,
            "easy": easy,
        }

    # Routing strategy:
    # - codex-normal:        easy mechanical fixes with confirmed code bugs (fast, cheap)
    # - codex-hard:          complex cops with many divergences, or medium difficulty
    # - claude-oauth-normal: config/parser-only issues needing investigation judgment
    # - claude-oauth-hard:   retries, prior failures

    # Retries and prior failures need fresh thinking, not more brute force
    if mode == "retry":
        return _result("claude-oauth-hard", "retry mode needs fresh investigation approach")

    # Reduce mode: high-divergence cops needing focused incremental progress
    if mode == "reduce":
        return _result("codex-hard", "reduce mode: high-divergence cop needs focused progress")

    # Explicit issue difficulty labels
    if issue_difficulty:
        if issue_difficulty == "simple":
            return _result("codex-normal", "issue difficulty label is simple", easy=True)
        if issue_difficulty == "complex":
            return _result("claude-oauth-hard", f"issue difficulty label is {issue_difficulty}")
        # medium → codex-hard (mechanical but needs more reasoning)
        return _result("codex-hard", f"issue difficulty label is {issue_difficulty}")

    if not entry:
        return _result("codex-hard", "cop is missing from corpus data")

    # Run prediagnosis to classify code bugs vs config/parser issues
    code_bugs = 0
    config_issues = 0
    is_easy_candidate = should_consider_easy_candidate(
        entry, min_total=min_total, max_total=max_total, min_matches=min_matches,
    )
    if precomputed_diagnosis is not None:
        code_bugs, config_issues = precomputed_diagnosis
    elif binary and binary.exists() and is_easy_candidate:
        fn_bugs, fn_cfg = diagnose_examples(binary, cop, entry.get("fn_examples", []), "fn")
        fp_bugs, fp_cfg = diagnose_examples(binary, cop, entry.get("fp_examples", []), "fp")
        code_bugs = fn_bugs + fp_bugs
        config_issues = fn_cfg + fp_cfg

    if is_easy_candidate and code_bugs >= min_bugs:
        # Confirmed code bugs in an easy cop — codex handles these well
        return _result(
            "codex-normal",
            f"easy cop: total={total_for_entry(entry)}, matches={entry.get('matches', 0)}, "
            f"diagnosed_code_bugs={code_bugs}",
            code_bugs=code_bugs, config_issues=config_issues, easy=True,
        )

    if is_easy_candidate and config_issues > 0 and code_bugs == 0:
        # All issues are config/parser-level, not cop bugs — claude
        # has better judgment on whether to fix or document
        return _result(
            "claude-oauth-normal",
            f"all {config_issues} issues are config/parser-level, not code bugs; "
            f"needs investigation to determine correct action",
            code_bugs=code_bugs, config_issues=config_issues,
        )

    # Complex cop (outside easy thresholds) or no binary for prediagnosis —
    # codex-hard handles high-volume mechanical work well
    if total > max_total:
        return _result(
            "codex-hard",
            f"complex cop with many divergences (total={total})",
            code_bugs=code_bugs, config_issues=config_issues,
        )

    # Default: moderate complexity, codex-hard for mechanical work
    return _result(
        "codex-hard",
        f"moderate cop (total={total}, matches={entry.get('matches', 0)}); "
        f"code_bugs={code_bugs}, config_issues={config_issues}",
        code_bugs=code_bugs, config_issues=config_issues,
    )


def classify_issue_difficulty(entry: dict, recommendation: dict[str, object]) -> str:
    if recommendation.get("easy"):
        return "simple"
    if tier_for_total(total_for_entry(entry)) >= 3:
        return "complex"
    return "medium"


def run_nitrocop(binary: Path, cwd: str, cop: str, filename: str = "test.rb") -> list[dict]:
    cmd = [str(binary), "--preview", "--no-cache", "--format", "json"]
    if BASELINE_CONFIG.exists():
        cmd.extend(["--config", str(BASELINE_CONFIG)])
    else:
        cmd.append("--force-default-config")
    cmd.extend(["--only", cop, filename])
    proc = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=30,
        cwd=cwd,
    )
    if proc.stdout.strip():
        try:
            return json.loads(proc.stdout).get("offenses", [])
        except json.JSONDecodeError:
            pass
    return []


def extract_diagnostic_lines(src: list[str]) -> tuple[list[str], str | None]:
    lines, offense = [], None
    for source_line in src:
        is_offense = source_line.strip().startswith(">>>")
        cleaned = re.sub(r"^(>>>\s*)?\s*\d+:\s?", "", source_line)
        lines.append(cleaned)
        if is_offense:
            offense = cleaned
    return lines, offense


def diagnose_examples(binary: Path, cop: str, examples: list, kind: str) -> tuple[int, int]:
    bugs, config_issues = 0, 0
    for example in examples[:15]:
        if not isinstance(example, dict) or not example.get("src"):
            continue
        lines, offense = extract_diagnostic_lines(example["src"])
        if not lines:
            continue
        # Preserve the original repo-relative path so that cops with
        # Include patterns (e.g., Rails cops matching **/test/**/*.rb or
        # RSpec cops matching **/*_spec.rb) can match the file.
        loc = example.get("loc", "")
        parsed = _parse_example_loc(loc)
        rel_path = parsed[1] if parsed else "test.rb"
        tmp = tempfile.mkdtemp()
        filepath = os.path.join(tmp, rel_path)
        os.makedirs(os.path.dirname(filepath), exist_ok=True)
        try:
            with open(filepath, "w") as file_handle:
                file_handle.write("\n".join(lines) + "\n")
            offenses = run_nitrocop(binary, tmp, cop, rel_path)
            if not offenses and offense:
                with open(filepath, "w") as file_handle:
                    file_handle.write(offense + "\n")
                offenses = run_nitrocop(binary, tmp, cop, rel_path)
            detected = len(offenses) > 0
            if (kind == "fn" and not detected) or (kind == "fp" and detected):
                bugs += 1
            else:
                config_issues += 1
        except Exception:
            pass
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    return bugs, config_issues


def run_gh(args: list[str], check: bool = True) -> str:
    result = subprocess.run(
        ["gh", *args],
        capture_output=True,
        text=True,
        check=check,
    )
    return result.stdout.strip()


def ensure_labels(repo: str) -> None:
    for label, color in LABEL_COLORS.items():
        subprocess.run(
            ["gh", "label", "create", label, "--repo", repo, "--color", color],
            capture_output=True,
            text=True,
            check=False,
        )


def list_agent_fix_prs(repo: str, state: str = "all") -> list[dict]:
    try:
        output = run_gh([
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            state,
            "--label",
            "type:cop-fix",
            "--limit",
            str(MAX_GH_PAGE),
            "--json",
            "number,title,state,url,headRefName,mergedAt,closedAt,body",
        ])
    except subprocess.CalledProcessError:
        return []
    return json.loads(output) if output else []


def index_prs_by_cop(prs: list[dict]) -> dict[str, list[dict]]:
    by_cop: dict[str, list[dict]] = {}
    for pr in prs:
        cop = extract_cop_from_pr(pr)
        if not cop:
            continue
        by_cop.setdefault(cop, []).append(pr)
    return by_cop


def list_tracker_issues(repo: str) -> list[dict]:
    try:
        output = run_gh([
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            "all",
            "--label",
            TRACKER_LABEL,
            "--limit",
            str(MAX_GH_PAGE),
            "--json",
            "number,title,state,url,body,labels",
        ])
    except subprocess.CalledProcessError:
        return []
    return json.loads(output) if output else []


def index_issues_by_cop(issues: list[dict]) -> dict[str, dict]:
    by_cop: dict[str, dict] = {}
    for issue in issues:
        cop = extract_cop_from_issue(issue)
        if not cop:
            continue
        by_cop[cop] = issue
    return by_cop


def issue_label_names(issue: dict) -> set[str]:
    return {label["name"] for label in issue.get("labels", [])}


def choose_issue_state(
    existing_issue: dict | None,
    has_open_pr: bool,
    entry: dict | None = None,
) -> str:
    if has_open_pr:
        return STATE_PR_OPEN
    if existing_issue and STATE_BLOCKED in issue_label_names(existing_issue):
        # Unblock if corpus numbers changed — the situation may have improved
        # (e.g., another cop fix reduced the FN, or a config change resolved it).
        if entry is not None:
            old = parse_marker_fields(existing_issue.get("body", ""), TRACKER_RE)
            old_fp, old_fn = old.get("fp"), old.get("fn")
            new_fp, new_fn = str(entry.get("fp", 0)), str(entry.get("fn", 0))
            if (old_fp, old_fn) != (new_fp, new_fn):
                return STATE_BACKLOG
        return STATE_BLOCKED
    return STATE_BACKLOG


def render_tracker_marker(
    cop: str, fp: int, fn: int, matches: int, difficulty: str,
) -> str:
    total = fp + fn
    return (
        f"<!-- {COP_TRACKER_MARKER}: cop={cop} fp={fp} fn={fn} "
        f"total={total} matches={matches} difficulty={difficulty} -->"
    )


def render_issue_body(
    cop: str,
    entry: dict,
    *,
    repo: str,
    difficulty: str,
    state_label: str,
    open_pr: dict | None,
    corpus_kind: str,
    run_id: str | None,
    head_sha: str | None,
    variant_entries: list[dict] | None = None,
) -> str:
    fp = entry.get("fp", 0)
    fn = entry.get("fn", 0)
    matches = entry.get("matches", 0)
    total = fp + fn
    lines = [
        f"# {make_issue_title(cop)}",
        "",
        "This issue is managed by the corpus dispatch backlog automation.",
        "",
        "## Corpus Status (default config)",
        "",
        f"- Cop: `{cop}`",
        f"- Corpus: `{corpus_kind}`",
        f"- False positives: `{fp}`",
        f"- False negatives: `{fn}`",
        f"- Total divergence: `{total}`",
        f"- Matches: `{matches}`",
        f"- Difficulty: `{difficulty}`",
        f"- Current state label: `{state_label}`",
    ]
    if run_id:
        lines.append(f"- Source run: [#{run_id}](https://github.com/{repo}/actions/runs/{run_id})")
    if head_sha:
        lines.append(f"- Corpus head SHA: `{head_sha}`")
    if open_pr:
        lines.append(f"- Open bot PR: #{open_pr['number']} ({open_pr['url']})")

    # Variant divergence section
    diverging_variants = [
        v for v in (variant_entries or [])
        if v.get("fp", 0) + v.get("fn", 0) > 0
    ]
    if diverging_variants:
        lines.extend([
            "",
            "## Style Variant Divergence",
            "",
        ])
        if total == 0:
            lines.append("Default config is perfect. Divergence is in non-default styles only:")
        else:
            lines.append("Additional divergence under non-default styles:")
        lines.extend([
            "",
            "| Style | Matches | FP | FN |",
            "|-------|--------:|---:|---:|",
        ])
        for v in sorted(diverging_variants, key=lambda x: x["fp"] + x["fn"], reverse=True):
            lines.append(
                f"| {v.get('style_label', '?')} | {v['matches']:,} | {v['fp']:,} | {v['fn']:,} |"
            )

    lines.extend([
        "",
        "## Automation Notes",
        "",
        "- This issue stays open while the cop still diverges (default or variant) or a bot PR is active.",
        "- If a later corpus run shows the cop diverging again after merge, this same issue should be reopened and reused.",
        "",
        render_tracker_marker(cop, fp, fn, matches, difficulty),
        "",
    ])
    return "\n".join(lines)


def sync_issue_labels(repo: str, issue_number: int, labels: list[str]) -> None:
    remove = STATE_LABELS + list(DIFFICULTY_LABELS.values())
    subprocess.run(
        [
            "gh", "issue", "edit", str(issue_number),
            "--repo", repo,
            "--remove-label", ",".join(remove),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    # Keep remove/add separate so gh does not need to reconcile overlapping
    # state/difficulty labels in one edit request.
    subprocess.run(
        [
            "gh", "issue", "edit", str(issue_number),
            "--repo", repo,
            "--add-label", ",".join([TRACKER_LABEL, *labels]),
        ],
        capture_output=True,
        text=True,
        check=True,
    )


def find_prior_prs(cop: str) -> list[dict]:
    try:
        output = run_gh([
            "pr",
            "list",
            "--state",
            "all",
            "--search",
            f"{cop} in:title",
            "--json",
            "number,title,state,headRefName,mergedAt,closedAt,url",
            "--limit",
            "20",
        ])
    except subprocess.CalledProcessError:
        return []

    prs = json.loads(output) if output else []
    cop_lower = cop.lower().replace("/", "")
    return [
        pr for pr in prs
        if cop_lower in pr.get("title", "").lower().replace("/", "").replace(" ", "")
        or cop_lower in pr.get("headRefName", "").lower().replace("-", "").replace("_", "")
    ]


def get_pr_diff(pr_number: int) -> str:
    try:
        return run_gh(["pr", "diff", str(pr_number)], check=False)
    except Exception:
        return "(could not fetch diff)"


def get_pr_checks(pr_number: int) -> str:
    try:
        return run_gh(["pr", "checks", str(pr_number)], check=False)
    except Exception:
        return "(could not fetch checks)"


def get_failed_check_logs(pr_number: int) -> str:
    try:
        checks_json = run_gh([
            "pr",
            "checks",
            str(pr_number),
            "--json",
            "name,state,detailsUrl",
        ], check=False)
        if not checks_json:
            return ""

        checks = json.loads(checks_json)
        failed = [check for check in checks if check.get("state") in ("FAILURE", "ERROR")]
        if not failed:
            return ""

        logs = []
        for check in failed[:3]:
            name = check.get("name", "unknown")
            url = check.get("detailsUrl", "")
            if "/actions/runs/" not in url:
                logs.append(f"#### {name} (FAILED)\nSee: {url}")
                continue

            run_id = url.split("/actions/runs/")[1].split("/")[0]
            try:
                log = run_gh(["run", "view", run_id, "--log-failed"], check=False)
            except Exception:
                log = ""
            if not log:
                logs.append(f"#### {name} (FAILED)\nSee: {url}")
                continue

            log_lines = log.splitlines()
            if len(log_lines) > 100:
                log = "\n".join(
                    ["... (truncated, showing last 100 lines) ...", *log_lines[-100:]]
                )
            logs.append(f"#### {name} (FAILED)\n```\n{log}\n```")

        return "\n\n".join(logs)
    except Exception:
        return ""


def _collect_nofix_findings(cop: str) -> list[str]:
    """Scrape 'Agent findings' from tracker issue comments for no-fix runs.

    Both cop-fix and PR-repair workflows post agent findings to the tracker
    issue when the agent investigates but fails to produce a fix.  These
    contain what was tried and why it failed — critical context for future
    agents.
    """
    title = make_issue_title(cop)
    try:
        output = run_gh([
            "issue", "list",
            "--search", f"{title} in:title",
            "--state", "all",
            "--json", "number",
            "--limit", "1",
        ])
    except subprocess.CalledProcessError:
        return []
    issues = json.loads(output) if output else []
    if not issues:
        return []

    issue_number = issues[0]["number"]
    try:
        output = run_gh([
            "issue", "view", str(issue_number),
            "--json", "comments",
        ])
    except subprocess.CalledProcessError:
        return []
    if not output:
        return []

    try:
        comments = json.loads(output).get("comments", [])
    except json.JSONDecodeError:
        return []

    findings: list[str] = []
    for comment in comments:
        body = comment.get("body", "")
        # Match comments that contain agent findings (from cop-fix or repair)
        if "Agent findings" not in body:
            continue
        lines = body.splitlines()
        in_fence = False
        finding_lines: list[str] = []
        run_url = ""
        for line in lines:
            if line.startswith("- Run:") or line.startswith("- Repair workflow:"):
                run_url = line.split(":", 1)[1].strip().split("]")[0].lstrip("[")
            if line.strip() == "```" and in_fence:
                in_fence = False
                continue
            if in_fence:
                finding_lines.append(line)
            if line.strip() == "```" and not in_fence:
                in_fence = True
        if finding_lines:
            header = "#### No-fix run findings"
            if run_url:
                header += f" ({run_url})"
            if len(finding_lines) > 30:
                finding_lines = [*finding_lines[:30], "... (truncated)"]
            findings.append(f"{header}\n```\n" + "\n".join(finding_lines) + "\n```")
    return findings


def collect_attempts(cop: str) -> str:
    prs = find_prior_prs(cop)
    nofix_findings = _collect_nofix_findings(cop)

    if not prs and not nofix_findings:
        return ""

    prs.sort(key=lambda pr: pr.get("number", 0))
    failed = [pr for pr in prs if not pr.get("mergedAt")]
    merged = [pr for pr in prs if pr.get("mergedAt")]
    if not failed and not merged and not nofix_findings:
        return ""

    parts = [f"## Prior Attempts ({len(failed)} failed, {len(merged)} merged)", ""]
    for index, pr in enumerate(failed, 1):
        number = pr["number"]
        parts.extend([
            f"### Attempt {index}: PR #{number} ({pr.get('state', '')})",
            f"Title: {pr.get('title', '')}",
            f"URL: {pr.get('url', '')}",
            "",
        ])

        diff = get_pr_diff(number)
        if diff and diff != "(could not fetch diff)":
            diff_lines = diff.splitlines()
            if len(diff_lines) > 80:
                diff = "\n".join([*diff_lines[:80], f"... ({len(diff_lines) - 80} more lines truncated)"])
            parts.extend(["#### What was changed", f"```diff\n{diff}\n```", ""])

        checks = get_pr_checks(number)
        if checks:
            parts.extend(["#### CI Status", f"```\n{checks}\n```", ""])

        logs = get_failed_check_logs(number)
        if logs:
            parts.extend(["#### Failure Logs", logs, ""])

    if nofix_findings:
        parts.append("### No-Fix Runs (agent investigated but reverted or produced no code change)")
        parts.append("")
        parts.append("**IMPORTANT: These contain findings from agents that tried and failed.")
        parts.append("Do NOT repeat the same approaches. Study what was tried and why it failed.**")
        parts.append("")
        parts.extend(nofix_findings)
        parts.append("")

    if merged:
        parts.append("### Previously Merged PRs (for reference)")
        parts.extend(
            f"- PR #{pr['number']}: {pr.get('title', '')} ({pr.get('url', '')})"
            for pr in merged
        )
        parts.append("")

    return "\n".join(parts)


def build_entry_index(data: dict) -> dict[str, dict]:
    return {entry["cop"]: entry for entry in data.get("by_cop", [])}


def comment_on_issue(repo: str, issue_number: int, body: str) -> None:
    subprocess.run(
        ["gh", "issue", "comment", str(issue_number), "--repo", repo, "--body", body],
        capture_output=True,
        text=True,
        check=True,
    )


def create_tracker_issue(repo: str, title: str, body: str, labels: list[str]) -> int:
    output = run_gh([
        "issue",
        "create",
        "--repo",
        repo,
        "--title",
        title,
        "--body",
        body,
        "--label",
        ",".join([TRACKER_LABEL, *labels]),
    ])
    match = re.search(r"/issues/(\d+)$", output.strip())
    if not match:
        raise RuntimeError(f"Could not parse issue number from gh issue create output: {output}")
    return int(match.group(1))


def update_tracker_issue(repo: str, issue_number: int, title: str, body: str, labels: list[str]) -> None:
    subprocess.run(
        [
            "gh", "issue", "edit", str(issue_number),
            "--repo", repo,
            "--title", title,
            "--body", body,
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    sync_issue_labels(repo, issue_number, labels)


def close_tracker_issue(repo: str, issue_number: int, body: str) -> None:
    subprocess.run(
        [
            "gh", "issue", "close", str(issue_number),
            "--repo", repo,
            "--comment", body,
        ],
        capture_output=True,
        text=True,
        check=True,
    )


def reopen_tracker_issue(repo: str, issue_number: int) -> None:
    subprocess.run(
        ["gh", "issue", "reopen", str(issue_number), "--repo", repo],
        capture_output=True,
        text=True,
        check=True,
    )


def fetch_corpus_for_sync(input_path: Path | None) -> tuple[dict, str | None, str | None]:
    if input_path:
        return json.loads(input_path.read_text()), None, None
    path, run_id, head_sha = _download_corpus()
    return json.loads(path.read_text()), str(run_id), head_sha



def cmd_backend(args: argparse.Namespace) -> int:
    data = load_dispatch_corpus(args.input)
    entry = build_entry_index(data).get(args.cop)
    prior_prs = index_prs_by_cop(list_agent_fix_prs(args.repo, state="all")).get(args.cop, [])
    binary = args.binary.resolve() if args.binary else None
    recommendation = select_backend_for_entry(
        args.cop,
        entry,
        mode=args.mode,
        binary=binary,
        prior_prs=prior_prs,
        issue_difficulty=args.issue_difficulty,
    )
    print(f"backend={recommendation['backend']}")
    print(f"family={backend_family(recommendation['backend'])}")
    print(f"strength={backend_strength(recommendation['backend'])}")
    print(f"display_label={backend_display_label(recommendation['backend'])}")
    print(f"reason={recommendation['reason']}")
    print(f"tier={recommendation['tier']}")
    print(f"code_bugs={recommendation['code_bugs']}")
    print(f"config_issues={recommendation['config_issues']}")
    print(f"easy={'true' if recommendation['easy'] else 'false'}")
    return 0


def cmd_issues_sync(args: argparse.Namespace) -> int:
    repo = args.repo
    ensure_labels(repo)
    data, run_id, head_sha = fetch_corpus_for_sync(args.input)
    corpus_kind = "corpus" if not args.input else "custom"
    entries = build_entry_index(data)
    issues = list_tracker_issues(repo)
    issues_by_cop = index_issues_by_cop(issues)
    prs_by_cop = index_prs_by_cop(list_agent_fix_prs(repo, state="all"))
    open_prs_by_cop = {
        cop: next((pr for pr in prs if pr.get("state") == "OPEN"), None)
        for cop, prs in prs_by_cop.items()
    }
    binary = args.binary.resolve() if args.binary else None
    diverging_cops = {cop for cop, entry in entries.items() if total_for_entry(entry) > 0}

    # Expand diverging set with variant-only divergence: cops that are
    # perfect in default config but broken under non-default styles.
    variant_diverging: dict[str, list[dict]] = {}  # cop -> variant entries
    from shared.corpus_artifacts import get_variant_results_path
    variant_path = get_variant_results_path(int(run_id)) if run_id else None
    if variant_path:
        try:
            vdata = json.loads(variant_path.read_text())
            for batch in vdata.get("batches", []):
                for cop_entry in batch.get("by_cop", []):
                    cn = cop_entry.get("cop", "")
                    if cn and cop_entry.get("fp", 0) + cop_entry.get("fn", 0) > 0:
                        variant_diverging.setdefault(cn, []).append(cop_entry)
        except (json.JSONDecodeError, OSError):
            pass
    variant_only_cops = set(variant_diverging.keys()) - diverging_cops
    if variant_only_cops:
        print(
            f"  {len(variant_only_cops)} cops diverge only in style variants",
            file=sys.stderr,
        )
    diverging_cops |= variant_only_cops

    # Filter by department if requested
    dept_filter = args.department
    if dept_filter:
        diverging_cops = {cop for cop in diverging_cops if cop.startswith(dept_filter + "/")}

    created = updated = reopened = closed = config_only = failed = 0

    # ── Phase 1: Pre-diagnose all cops in parallel ──────────────────────
    # Prefer pre-computed diagnosis from corpus oracle (embedded in by_cop
    # entries by bench/corpus/diagnose_corpus.py). Falls back to runtime
    # diagnosis when pre-computed data is missing (older corpus artifacts).
    diagnosis: dict[str, tuple[int, int]] = {}  # cop -> (code_bugs, config_issues)
    precomputed_count = 0
    for cop_entry in data.get("by_cop", []):
        diag = cop_entry.get("diagnosis")
        if diag and cop_entry["cop"] in diverging_cops:
            diagnosis[cop_entry["cop"]] = (diag["code_bugs"], diag["config_issues"])
            precomputed_count += 1

    if precomputed_count:
        print(f"Using pre-computed diagnosis for {precomputed_count} cops", file=sys.stderr)

    # Fall back to runtime diagnosis for any cops without pre-computed data
    remaining = diverging_cops - diagnosis.keys()
    if remaining and binary:
        def _diagnose_cop(cop: str) -> tuple[str, int, int]:
            entry = entries[cop]
            fn_bugs, fn_cfg = diagnose_examples(binary, cop, entry.get("fn_examples", []), "fn")
            fp_bugs, fp_cfg = diagnose_examples(binary, cop, entry.get("fp_examples", []), "fp")
            return cop, fn_bugs + fp_bugs, fn_cfg + fp_cfg

        print(f"Pre-diagnosing {len(remaining)} cops (no pre-computed data)...", file=sys.stderr)
        with ThreadPoolExecutor(max_workers=8) as pool:
            futures = {pool.submit(_diagnose_cop, cop): cop for cop in remaining}
            for future in as_completed(futures):
                cop_name, code_bugs, cfg_issues = future.result()
                diagnosis[cop_name] = (code_bugs, cfg_issues)
        print("  diagnosis complete", file=sys.stderr)

    # ── Phase 2: Compute issue metadata and determine actions ───────────
    # Each action is (action_type, callable) where callable performs the
    # GitHub API call.  We group by department for progress logging.
    Action = tuple[str, callable]  # ("created"|"updated"|"reopened", fn)
    dept_actions: dict[str, list[Action]] = {}

    for cop in sorted(diverging_cops):
        dept = cop.split("/")[0]
        entry = entries[cop]
        prior_prs = prs_by_cop.get(cop, [])

        code_bugs, cfg_issues = diagnosis.get(cop, (0, 0))
        # Classify as config-only when pre-diagnostic finds 0 code bugs AND:
        # - Zero matches (Include-gated cop, never fires), OR
        # - All divergence is config/context issues (snippet + full-file
        #   fallback couldn't reproduce any FP/FN with default config)
        is_config_only = binary is not None and code_bugs == 0 and cfg_issues > 0
        precomputed = (code_bugs, cfg_issues) if binary else None

        recommendation = select_backend_for_entry(
            cop,
            entry,
            mode="fix",
            binary=binary,
            prior_prs=prior_prs,
            precomputed_diagnosis=precomputed,
        )
        difficulty = "config-only" if is_config_only else classify_issue_difficulty(entry, recommendation)
        if is_config_only:
            config_only += 1
        open_pr = open_prs_by_cop.get(cop)
        existing_issue = issues_by_cop.get(cop)
        state_label = choose_issue_state(existing_issue, open_pr is not None, entry)
        labels = [state_label, DIFFICULTY_LABELS[difficulty]]
        title = make_issue_title(cop)
        body = render_issue_body(
            cop,
            entry,
            repo=repo,
            difficulty=difficulty,
            state_label=state_label,
            open_pr=open_pr,
            corpus_kind=corpus_kind,
            run_id=run_id,
            head_sha=head_sha,
            variant_entries=variant_diverging.get(cop, []),
        )

        if existing_issue is None:
            dept_actions.setdefault(dept, []).append(
                ("created", lambda r=repo, t=title, b=body, lb=labels: create_tracker_issue(r, t, b, lb)),
            )
            continue

        issue_num = existing_issue["number"]

        if existing_issue.get("state") == "CLOSED":
            reopen_comment = (
                f"Reopened from the latest {corpus_kind} corpus sync"
                + (f" (run #{run_id})." if run_id else ".")
            )

            def _reopen_and_update(
                r=repo, n=issue_num, t=title, b=body, lb=labels, c=reopen_comment,
            ) -> None:
                reopen_tracker_issue(r, n)
                comment_on_issue(r, n, c)
                update_tracker_issue(r, n, t, b, lb)

            dept_actions.setdefault(dept, []).append(("reopened", _reopen_and_update))
            continue

        dept_actions.setdefault(dept, []).append(
            ("updated", lambda r=repo, n=issue_num, t=title, b=body, lb=labels: update_tracker_issue(r, n, t, b, lb)),
        )

    # ── Phase 3: Execute GitHub API calls in parallel, per department ───
    for dept in sorted(dept_actions):
        actions = dept_actions[dept]
        print(f"Syncing {dept} ({len(actions)} cops)...", file=sys.stderr)
        results: list[str] = []
        with ThreadPoolExecutor(max_workers=4) as pool:
            def _run_action(action: Action) -> str:
                action_type, fn = action
                max_attempts = 3
                for attempt in range(1, max_attempts + 1):
                    try:
                        fn()
                        return action_type
                    except Exception as exc:
                        if attempt < max_attempts:
                            backoff = attempt  # 1s then 2s
                            print(
                                f"  warning: {action_type} attempt {attempt}/{max_attempts}"
                                f" failed ({exc}), retrying in {backoff}s...",
                                file=sys.stderr,
                            )
                            time.sleep(backoff)
                        else:
                            print(
                                f"  warning: {action_type} failed after {max_attempts}"
                                f" attempts: {exc}",
                                file=sys.stderr,
                            )
                            return "failed"
                return "failed"  # unreachable, but keeps mypy happy
            results = list(pool.map(_run_action, actions))
        dept_created = results.count("created")
        dept_updated = results.count("updated")
        dept_reopened = results.count("reopened")
        dept_failed = results.count("failed")
        created += dept_created
        updated += dept_updated
        reopened += dept_reopened
        failed += dept_failed
        summary_parts = [
            f"{dept_created} created",
            f"{dept_updated} updated",
            f"{dept_reopened} reopened",
        ]
        if dept_failed:
            summary_parts.append(f"{dept_failed} failed")
        print(
            f"  done ({', '.join(summary_parts)})",
            file=sys.stderr,
        )

    # ── Phase 4: Close resolved issues in parallel ──────────────────────
    to_close: list[tuple[int, str]] = []
    for cop, issue in issues_by_cop.items():
        if cop in diverging_cops:
            continue
        if dept_filter and not cop.startswith(dept_filter + "/"):
            continue
        open_pr = open_prs_by_cop.get(cop)
        if open_pr is not None or issue.get("state") != "OPEN":
            continue
        close_comment = (
            f"No longer diverges in the latest {corpus_kind} corpus sync"
            + (f" (run #{run_id})." if run_id else ".")
        )
        to_close.append((issue["number"], close_comment))

    if to_close:
        print(f"Closing {len(to_close)} resolved issues...", file=sys.stderr)
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(lambda t: close_tracker_issue(repo, t[0], t[1]), to_close))
        closed = len(to_close)

    if failed:
        print(
            f"warning: {failed} action(s) failed due to transient errors"
            " (sync is best-effort)",
            file=sys.stderr,
        )

    print(
        json.dumps(
            {
                "created": created,
                "updated": updated,
                "reopened": reopened,
                "closed": closed,
                "failed": failed,
                "config_only": config_only,
                "diverging_cops": len(diverging_cops),
            },
            indent=2,
        )
    )
    return 0


def main():
    parser = argparse.ArgumentParser(description="Dispatch-related corpus tooling")
    subparsers = parser.add_subparsers(dest="command", required=True)

    task_parser = subparsers.add_parser("task", help="Generate a self-contained task prompt")
    task_parser.add_argument("cop", help="Cop name (e.g., Style/NegatedWhile)")
    task_parser.add_argument("--output", "-o", type=Path, help="Output file path (default: stdout)")
    task_parser.add_argument("--input", type=Path, help="Path to corpus-results.json")
    task_parser.add_argument("--binary", type=Path, help="Path to nitrocop binary for pre-diagnostic classification")

    changed_parser = subparsers.add_parser("changed", help="Detect cops changed between two refs")
    changed_parser.add_argument("--base", default="origin/main", help="Base ref")
    changed_parser.add_argument("--head", default="HEAD", help="Head ref")

    tiers_parser = subparsers.add_parser("tiers", help="Classify diverging cops into tiers")
    tiers_parser.add_argument("--input", type=Path, help="Path to corpus-results.json")
    tiers_parser.add_argument("--tier", type=int, choices=[1, 2, 3], help="Show only one tier")
    tiers_parser.add_argument("--names", action="store_true", help="Output just cop names")

    rank_parser = subparsers.add_parser("rank", help="Rank cops by dispatchability")
    rank_parser.add_argument("--binary", type=Path, help="Path to nitrocop binary")
    rank_parser.add_argument("--min-bugs", type=int, default=1)
    rank_parser.add_argument("--min-matches", type=int, default=50)
    rank_parser.add_argument("--department", help="Only include cops in this department (e.g., Style, Layout)")
    rank_parser.add_argument("--limit", type=int, default=0, help="Return at most N cops (0 = unlimited)")
    rank_parser.add_argument("--json", action="store_true")

    prior_parser = subparsers.add_parser("prior-attempts", help="Collect prior failed PR attempts")
    prior_parser.add_argument("--cop", required=True, help="Cop name (e.g., Style/NegatedWhile)")
    prior_parser.add_argument("--output", "-o", type=Path, help="Output file path")

    backend_parser = subparsers.add_parser("backend", help="Select a backend for a cop")
    backend_parser.add_argument("--cop", required=True, help="Cop name (e.g., Style/NegatedWhile)")
    backend_parser.add_argument("--mode", choices=["fix", "retry", "reduce"], default="fix")
    backend_parser.add_argument("--binary", type=Path, help="Path to nitrocop binary")
    backend_parser.add_argument("--input", type=Path, help="Path to corpus-results.json")
    backend_parser.add_argument(
        "--issue-difficulty",
        choices=["simple", "medium", "complex"],
        help="Use linked issue difficulty label as the routing hint",
    )
    backend_parser.add_argument(
        "--repo",
        default=os.environ.get("GITHUB_REPOSITORY", ""),
        help="GitHub repo (owner/name) for prior-attempt lookup",
    )

    issues_sync = subparsers.add_parser("issues-sync", help="Sync one tracker issue per diverging cop")
    issues_sync.add_argument("--input", type=Path, help="Path to corpus-results.json")
    issues_sync.add_argument("--binary", type=Path, help="Path to nitrocop binary for backend routing")
    issues_sync.add_argument(
        "--department",
        help="Only sync cops in this department (e.g., Rails, Style, Performance)",
    )
    issues_sync.add_argument(
        "--repo",
        default=os.environ.get("GITHUB_REPOSITORY", ""),
        help="GitHub repo (owner/name)",
    )

    args = parser.parse_args()

    if args.command == "task":
        binary = args.binary.resolve() if args.binary else None
        task = generate_task(args.cop, args.input, binary)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(task)
            print(f"Task written to {args.output}", file=sys.stderr)
        else:
            print(task)
        return

    if args.command == "changed":
        for cop in detect_cops(args.base, args.head):
            print(cop)
        return

    if args.command == "tiers":
        data = load_dispatch_corpus(args.input)
        cops, tiers = tier_cops(data)
        if args.names:
            target = tiers[args.tier] if args.tier else cops
            for cop in target:
                print(cop["cop"])
            return
        if args.tier:
            tier_entries = tiers[args.tier]
            low, high = TIER_THRESHOLDS[args.tier]
            print(f"Tier {args.tier} ({low}-{high} FP+FN): {len(tier_entries)} cops\n")
            print(f"{'Cop':<50} {'FP':>6} {'FN':>6} {'Total':>6} {'Match%':>7}")
            print(f"{'-'*50} {'-'*6} {'-'*6} {'-'*6} {'-'*7}")
            for cop in tier_entries:
                pct = f"{cop['match_rate']*100:.1f}%" if cop['match_rate'] else "?"
                print(f"{cop['cop']:<50} {cop['fp']:>6} {cop['fn']:>6} {cop['total']:>6} {pct:>7}")
            return

        print(f"Total diverging cops: {len(cops)}\n")
        for tier in [1, 2, 3]:
            tier_entries = tiers[tier]
            low, high = TIER_THRESHOLDS[tier]
            total = sum(cop["total"] for cop in tier_entries)
            print(f"Tier {tier} ({low}-{high} FP+FN): {len(tier_entries)} cops, {total:,} total FP+FN")
        print(f"\nTotal FP+FN: {sum(cop['total'] for cop in cops):,}")
        print("\nUse --tier N to see individual cops in a tier.")
        print("Use --tier N --names for scripting (one cop name per line).")
        return

    if args.command == "rank":
        binary = args.binary
        if not binary:
            for candidate in [
                Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "debug" / "nitrocop",
                Path("target-linux/debug/nitrocop"),
                Path("target/debug/nitrocop"),
            ]:
                if candidate.exists():
                    binary = candidate.resolve()
                    break
        if not binary or not binary.exists():
            print("Error: nitrocop binary not found. Build with 'cargo build' or pass --binary", file=sys.stderr)
            raise SystemExit(1)

        print(f"Using binary: {binary}", file=sys.stderr)
        path, _, _ = _download_corpus()
        data = json.loads(path.read_text())
        results = []
        for entry in sorted(data["by_cop"], key=lambda item: item.get("fp", 0) + item.get("fn", 0)):
            fp = entry.get("fp", 0)
            fn = entry.get("fn", 0)
            total = fp + fn
            if total < 1:
                continue
            if entry.get("matches", 0) < args.min_matches:
                continue

            cop_name = entry["cop"]
            if args.department and not cop_name.startswith(args.department + "/"):
                continue

            # Use pre-computed diagnosis from corpus artifact when available
            diag = entry.get("diagnosis")
            if diag:
                bugs = diag["code_bugs"]
                cfg = diag["config_issues"]
            else:
                fn_bugs, fn_cfg = diagnose_examples(binary, cop_name, entry.get("fn_examples", []), "fn")
                fp_bugs, fp_cfg = diagnose_examples(binary, cop_name, entry.get("fp_examples", []), "fp")
                bugs = fn_bugs + fp_bugs
                cfg = fn_cfg + fp_cfg

            if bugs >= args.min_bugs:
                results.append({
                    "cop": cop_name,
                    "fp": fp,
                    "fn": fn,
                    "code_bugs": bugs,
                    "config_issues": cfg,
                    "matches": entry.get("matches", 0),
                })

        # Add variant-only diverging cops (perfect in default, broken in variants).
        # These are skipped by the default loop (total < 1) but are real work.
        ranked_cops = {r["cop"] for r in results}
        variant_candidates = load_variant_only_candidates(
            data, args.department, ranked_cops)
        results.extend(variant_candidates)

        # Sort: highest code-bug ratio first, then lowest total divergence, then name.
        # Variant-only cops have code_bugs=0 so they sort to the end — dispatched
        # only after all default-diverging cops are handled.
        results.sort(key=lambda r: (-r["code_bugs"] / max(r["code_bugs"] + r["config_issues"], 1),
                                     r["fp"] + r["fn"],
                                     r["cop"]))
        if args.limit > 0:
            results = results[:args.limit]

        if args.json:
            json.dump(results, sys.stdout, indent=2)
            sys.stdout.write("\n")
        else:
            print(f"\n{'Cop':<42} {'FP':>3} {'FN':>3} {'Bugs':>4} {'Cfg':>4} {'Matches':>7} {'Var':>5}")
            print("-" * 75)
            for result in results:
                var_total = result.get("variant_fp", 0) + result.get("variant_fn", 0)
                var_str = str(var_total) if var_total > 0 else ""
                print(
                    f"{result['cop']:<42} {result['fp']:>3} {result['fn']:>3} "
                    f"{result['code_bugs']:>4} {result['config_issues']:>4} "
                    f"{result['matches']:>7} {var_str:>5}"
                )
            print(f"\n{len(results)} cops with {args.min_bugs}+ code bugs", file=sys.stderr)
        return

    if args.command == "prior-attempts":
        result = collect_attempts(args.cop)
        if args.output:
            args.output.write_text(result)
            if result:
                print(f"Collected prior attempts → {args.output}", file=sys.stderr)
            else:
                print(f"No prior attempts found for {args.cop}", file=sys.stderr)
        else:
            if result:
                print(result)
            else:
                print(f"No prior attempts found for {args.cop}", file=sys.stderr)
        return

    if args.command == "backend":
        if not args.repo:
            print("Error: --repo or GITHUB_REPOSITORY is required", file=sys.stderr)
            raise SystemExit(1)
        raise SystemExit(cmd_backend(args))

    if args.command == "issues-sync":
        if not args.repo:
            print("Error: --repo or GITHUB_REPOSITORY is required", file=sys.stderr)
            raise SystemExit(1)
        raise SystemExit(cmd_issues_sync(args))


if __name__ == "__main__":
    main()
