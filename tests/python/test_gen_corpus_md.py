#!/usr/bin/env python3
"""Tests for bench/corpus/gen_corpus_md.py."""

import importlib.util
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "bench" / "corpus" / "gen_corpus_md.py"
SPEC = importlib.util.spec_from_file_location("gen_corpus_md", SCRIPT)
assert SPEC and SPEC.loader
gen_corpus_md = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gen_corpus_md)


def test_generate_md_basic():
    """Generates valid markdown from minimal corpus data."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 5, "repos_error": 0,
            "total_offenses_compared": 100, "matches": 95, "fp": 3, "fn": 2,
            "registered_cops": 5, "perfect_cops": 3, "diverging_cops": 2,
            "inactive_cops": 0, "overall_match_rate": 0.95,
            "total_files_inspected": 500, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 5, "perfect_cops": 3,
            "diverging_cops": 2, "inactive_cops": 0,
            "matches": 95, "fp": 3, "fn": 2, "match_rate": 0.95,
        }],
        "by_cop": [
            {"cop": "Style/Foo", "matches": 50, "fp": 3, "fn": 0,
             "match_rate": 0.9433, "exercised": True, "perfect_match": False,
             "diverging": True, "fp_examples": ["repo_a: a.rb:1"], "fn_examples": []},
            {"cop": "Style/Bar", "matches": 45, "fp": 0, "fn": 2,
             "match_rate": 0.9574, "exercised": True, "perfect_match": False,
             "diverging": True, "fp_examples": [], "fn_examples": ["repo_a: b.rb:5"]},
        ],
        "by_repo": [{"repo": "repo_a", "status": "ok", "match_rate": 0.95,
                      "matches": 95, "fp": 3, "fn": 2, "files_inspected": 50}],
        "by_repo_cop": {},
    }
    md = gen_corpus_md.generate_md(data, {})
    assert "# Corpus Oracle Results" in md
    assert "## Summary" in md
    assert "## Diverging Cops" in md
    assert "Style/Foo" in md
    assert "Match rate (default config)" in md


def test_generate_md_with_variants():
    """Variant data adds variant rows and match rate."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 5, "repos_error": 0,
            "total_offenses_compared": 100, "matches": 100, "fp": 0, "fn": 0,
            "registered_cops": 1, "perfect_cops": 1, "diverging_cops": 0,
            "inactive_cops": 0, "overall_match_rate": 1.0,
            "total_files_inspected": 500, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 1, "perfect_cops": 1,
            "diverging_cops": 0, "inactive_cops": 0,
            "matches": 100, "fp": 0, "fn": 0, "match_rate": 1.0,
        }],
        "by_cop": [
            {"cop": "Style/Foo", "matches": 100, "fp": 0, "fn": 0,
             "match_rate": 1.0, "exercised": True, "perfect_match": True,
             "diverging": False, "fp_examples": [], "fn_examples": []},
        ],
        "by_repo": [], "by_repo_cop": {},
    }
    variant_by_cop = {
        "Style/Foo": [
            {"style_label": "bar", "matches": 80, "fp": 10, "fn": 5},
        ],
    }
    md = gen_corpus_md.generate_md(data, variant_by_cop)
    assert "Match rate (all variants)" in md
    assert "### All `EnforcedStyle` Variants" in md
    assert "| Match % |" in md
    assert "Variant-only divergence" in md
    assert "Style/Foo (bar)" in md


def test_emit_examples():
    """_emit_examples appends FP/FN bullets with truncation."""
    md: list[str] = []
    gen_corpus_md._emit_examples(
        md,
        fp_list=["repo_a: a.rb:1  [msg1]", "repo_b: b.rb:2  [msg2]", "repo_c: c.rb:3  [msg3]", "repo_d: d.rb:4  [msg4]"],
        fn_list=["repo_x: x.rb:10  [missed]"],
        limit=2,
    )
    text = "\n".join(md)
    assert "FP: `repo_a: a.rb:1  [msg1]`" in text
    assert "FP: `repo_b: b.rb:2  [msg2]`" in text
    assert "2 more FP" in text
    assert "repo_c" not in text  # truncated
    assert "FN: `repo_x: x.rb:10  [missed]`" in text


def test_emit_examples_empty():
    """_emit_examples with no examples just appends a blank line."""
    md: list[str] = []
    gen_corpus_md._emit_examples(md, [], [], limit=3)
    assert md == [""]


def test_details_for_variant_only_diverging_cop():
    """Variant-only diverging cops get <details> with variant examples."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 9, "repos_error": 0,
            "total_offenses_compared": 200, "matches": 200, "fp": 0, "fn": 0,
            "registered_cops": 1, "perfect_cops": 1, "diverging_cops": 0,
            "inactive_cops": 0, "overall_match_rate": 1.0,
            "total_files_inspected": 100, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 1, "perfect_cops": 1,
            "diverging_cops": 0, "inactive_cops": 0,
            "matches": 200, "fp": 0, "fn": 0, "match_rate": 1.0,
        }],
        "by_cop": [
            {"cop": "Style/Qux", "matches": 200, "fp": 0, "fn": 0,
             "match_rate": 1.0, "exercised": True, "perfect_match": True,
             "diverging": False, "fp_examples": [], "fn_examples": []},
        ],
        "by_repo": [], "by_repo_cop": {},
    }
    variant_by_cop = {
        "Style/Qux": [
            {"style_label": "alternate", "matches": 50, "fp": 0, "fn": 3,
             "fp_examples": [],
             "fn_examples": ["repo_z: z.rb:7  [offense missed]", "repo_z: z.rb:12  [also missed]"]},
        ],
    }
    md = gen_corpus_md.generate_md(data, variant_by_cop)
    assert "<details>" in md
    assert "<strong>Style/Qux</strong>" in md
    assert "**alternate** (0 FP, 3 FN):" in md
    assert "FN: `repo_z: z.rb:7  [offense missed]`" in md
    assert "FN: `repo_z: z.rb:12  [also missed]`" in md


def test_details_for_default_diverging_cop_with_variant_examples():
    """Default-diverging cops show both default and per-variant example subsections."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 5, "repos_error": 0,
            "total_offenses_compared": 100, "matches": 90, "fp": 5, "fn": 5,
            "registered_cops": 1, "perfect_cops": 0, "diverging_cops": 1,
            "inactive_cops": 0, "overall_match_rate": 0.9,
            "total_files_inspected": 100, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Layout", "cops": 1, "perfect_cops": 0,
            "diverging_cops": 1, "inactive_cops": 0,
            "matches": 90, "fp": 5, "fn": 5, "match_rate": 0.9,
        }],
        "by_cop": [
            {"cop": "Layout/Thing", "matches": 90, "fp": 5, "fn": 5,
             "match_rate": 0.9, "exercised": True, "perfect_match": False,
             "diverging": True,
             "fp_examples": ["repo_a: a.rb:1  [default fp]"],
             "fn_examples": ["repo_a: a.rb:2  [default fn]"]},
        ],
        "by_repo": [], "by_repo_cop": {},
    }
    variant_by_cop = {
        "Layout/Thing": [
            {"style_label": "compact", "matches": 40, "fp": 2, "fn": 8,
             "fp_examples": ["repo_b: b.rb:3  [variant fp]"],
             "fn_examples": ["repo_b: b.rb:4  [variant fn]"]},
        ],
    }
    md = gen_corpus_md.generate_md(data, variant_by_cop)
    assert "<details>" in md
    assert "<strong>Layout/Thing</strong>" in md
    # Summary line aggregates default + variant FP/FN
    assert "7 FP, 13 FN" in md
    # Default subsection
    assert "**Default config** (5 FP, 5 FN):" in md
    assert "FP: `repo_a: a.rb:1  [default fp]`" in md
    assert "FN: `repo_a: a.rb:2  [default fn]`" in md
    # Variant subsection
    assert "**compact** (2 FP, 8 FN):" in md
    assert "FP: `repo_b: b.rb:3  [variant fp]`" in md
    assert "FN: `repo_b: b.rb:4  [variant fn]`" in md


def test_no_details_when_no_examples():
    """Cops without FP/FN examples get no <details> section."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 5, "repos_error": 0,
            "total_offenses_compared": 100, "matches": 95, "fp": 3, "fn": 2,
            "registered_cops": 1, "perfect_cops": 0, "diverging_cops": 1,
            "inactive_cops": 0, "overall_match_rate": 0.95,
            "total_files_inspected": 100, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 1, "perfect_cops": 0,
            "diverging_cops": 1, "inactive_cops": 0,
            "matches": 95, "fp": 3, "fn": 2, "match_rate": 0.95,
        }],
        "by_cop": [
            {"cop": "Style/NoExamples", "matches": 95, "fp": 3, "fn": 2,
             "match_rate": 0.95, "exercised": True, "perfect_match": False,
             "diverging": True, "fp_examples": [], "fn_examples": []},
        ],
        "by_repo": [], "by_repo_cop": {},
    }
    md = gen_corpus_md.generate_md(data, {})
    assert "<details>" not in md or "<strong>Style/NoExamples</strong>" not in md


def test_load_variant_by_cop_preserves_examples():
    """load_variant_by_cop includes fp_examples and fn_examples."""
    import json
    import tempfile

    variant_data = {
        "batches": [{
            "name": "variant_batch_1",
            "by_cop": [{
                "cop": "Style/Foo",
                "style_label": "bar",
                "matches": 80, "fp": 1, "fn": 2,
                "fp_examples": ["repo_a: a.rb:1  [extra]"],
                "fn_examples": ["repo_a: a.rb:5  [missed]", "repo_a: a.rb:6  [also missed]"],
            }],
        }],
    }
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
        json.dump(variant_data, f)
        f.flush()
        result = gen_corpus_md.load_variant_by_cop(Path(f.name))

    assert "Style/Foo" in result
    v = result["Style/Foo"][0]
    assert v["fp_examples"] == ["repo_a: a.rb:1  [extra]"]
    assert len(v["fn_examples"]) == 2


def test_generate_md_no_variant_column_without_data():
    """Without variant data, no variant column appears."""
    data = {
        "summary": {
            "total_repos": 1, "repos_perfect": 1, "repos_error": 0,
            "total_offenses_compared": 10, "matches": 10, "fp": 0, "fn": 0,
            "registered_cops": 1, "perfect_cops": 1, "diverging_cops": 0,
            "inactive_cops": 0, "overall_match_rate": 1.0,
            "total_files_inspected": 5, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 1, "perfect_cops": 1,
            "diverging_cops": 0, "inactive_cops": 0,
            "matches": 10, "fp": 0, "fn": 0, "match_rate": 1.0,
        }],
        "by_cop": [], "by_repo": [], "by_repo_cop": {},
    }
    md = gen_corpus_md.generate_md(data, {})
    assert "All variants %" not in md
    assert "Match %" in md


def test_synthetic_row_in_summary():
    """Synthetic results add an 'incl. synthetic' row to the summary table."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 5, "repos_error": 0,
            "total_offenses_compared": 100, "matches": 95, "fp": 3, "fn": 2,
            "registered_cops": 4, "perfect_cops": 2, "diverging_cops": 1,
            "inactive_cops": 1, "overall_match_rate": 0.95,
            "total_files_inspected": 500, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 4, "perfect_cops": 2,
            "diverging_cops": 1, "inactive_cops": 1,
            "matches": 95, "fp": 3, "fn": 2, "match_rate": 0.95,
        }],
        "by_cop": [
            {"cop": "Style/Foo", "matches": 50, "fp": 0, "fn": 0,
             "match_rate": 1.0, "exercised": True, "perfect_match": True,
             "diverging": False},
            {"cop": "Style/Bar", "matches": 45, "fp": 3, "fn": 2,
             "match_rate": 0.9, "exercised": True, "perfect_match": False,
             "diverging": True},
            {"cop": "Style/Baz", "matches": 0, "fp": 0, "fn": 0,
             "exercised": False, "perfect_match": True,
             "diverging": False},
            # No-data cop that synthetic marks as perfect
            {"cop": "Style/NoData", "matches": 0, "fp": 0, "fn": 0,
             "exercised": False, "perfect_match": False,
             "diverging": False},
        ],
        "by_repo": [], "by_repo_cop": {},
    }
    synthetic = {"Style/NoData": {"cop": "Style/NoData", "perfect_match": True}}
    md = gen_corpus_md.generate_md(data, {}, synthetic)
    assert "| Cops with exact match | 2 |" in md
    assert "| Cops with exact match (incl. synthetic) | 3 |" in md


def test_no_synthetic_row_without_flag():
    """Without synthetic data, no 'incl. synthetic' row appears."""
    data = {
        "summary": {
            "total_repos": 1, "repos_perfect": 1, "repos_error": 0,
            "total_offenses_compared": 10, "matches": 10, "fp": 0, "fn": 0,
            "registered_cops": 1, "perfect_cops": 1, "diverging_cops": 0,
            "inactive_cops": 0, "overall_match_rate": 1.0,
            "total_files_inspected": 5, "rubocop_files_dropped": 0,
        },
        "by_department": [],
        "by_cop": [
            {"cop": "Style/Foo", "matches": 10, "fp": 0, "fn": 0,
             "exercised": True, "perfect_match": True, "diverging": False},
        ],
        "by_repo": [], "by_repo_cop": {},
    }
    md = gen_corpus_md.generate_md(data, {})
    assert "incl. synthetic" not in md
