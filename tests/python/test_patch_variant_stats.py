#!/usr/bin/env python3
"""Tests for bench/corpus/patch_variant_stats.py."""

import importlib.util
import json
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "bench" / "corpus" / "patch_variant_stats.py"
SPEC = importlib.util.spec_from_file_location("patch_variant_stats", SCRIPT)
assert SPEC and SPEC.loader
patch_variant_stats = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(patch_variant_stats)


def test_patch_adds_variant_summary(tmp_path):
    corpus = {
        "summary": {"matches": 1000, "fp": 10, "fn": 5},
        "by_department": [
            {"department": "Style", "matches": 800, "fp": 8, "fn": 3},
            {"department": "Layout", "matches": 200, "fp": 2, "fn": 2},
        ],
        "by_cop": [
            {"cop": "Style/Foo", "matches": 800, "fp": 8, "fn": 3,
             "perfect_match": False, "diverging": True},
            {"cop": "Layout/Baz", "matches": 200, "fp": 2, "fn": 2,
             "perfect_match": False, "diverging": True},
        ],
    }
    variants = tmp_path / "variants.json"
    variants.write_text(json.dumps({
        "batches": [{
            "name": "batch_1",
            "by_cop": [
                {"cop": "Style/Foo", "style_label": "bar", "matches": 50, "fp": 5, "fn": 3},
                {"cop": "Layout/Baz", "style_label": "qux", "matches": 20, "fp": 1, "fn": 2},
            ],
        }]
    }))

    result = patch_variant_stats.patch(corpus, variants)
    assert result is True

    s = corpus["summary"]
    # Overall: default (1000+10+5) + variant (50+5+3 + 20+1+2) = 1096
    assert s["variant_matches"] == 1000 + 50 + 20
    assert s["variant_fp"] == 10 + 5 + 1
    assert s["variant_fn"] == 5 + 3 + 2

    # Department rates should use final default as base
    style = corpus["by_department"][0]
    assert "variant_match_rate" in style
    # Style: default_total=811, variant_extra=58, combined_matches=850, combined_total=869
    assert style["variant_match_rate"] < style["matches"] / (style["matches"] + style["fp"] + style["fn"])

    layout = corpus["by_department"][1]
    assert "variant_match_rate" in layout


def test_patch_variant_rate_always_lte_default(tmp_path):
    """Variant rate must always be <= default rate (adding offenses can only hurt)."""
    corpus = {
        "summary": {"matches": 10000, "fp": 100, "fn": 50},
        "by_department": [
            {"department": "Style", "matches": 10000, "fp": 100, "fn": 50},
        ],
    }
    variants = tmp_path / "variants.json"
    variants.write_text(json.dumps({
        "batches": [{
            "name": "batch_1",
            "by_cop": [
                {"cop": "Style/A", "style_label": "x", "matches": 500, "fp": 200, "fn": 100},
            ],
        }]
    }))

    patch_variant_stats.patch(corpus, variants)

    default_total = 10000 + 100 + 50
    default_rate = 10000 / default_total
    variant_rate = corpus["summary"]["variant_overall_match_rate"]
    assert variant_rate <= default_rate

    dept_variant = corpus["by_department"][0]["variant_match_rate"]
    dept_default = 10000 / default_total
    assert dept_variant <= dept_default


def test_patch_computes_variant_perfect_cops(tmp_path):
    """Per-department variant_perfect_cops counts cops perfect in both default and variants."""
    corpus = {
        "summary": {"matches": 100, "fp": 5, "fn": 0},
        "by_department": [
            {"department": "Style", "matches": 100, "fp": 5, "fn": 0},
        ],
        "by_cop": [
            # Perfect in default, perfect in variants → variant-perfect
            {"cop": "Style/A", "matches": 50, "fp": 0, "fn": 0,
             "perfect_match": True, "diverging": False},
            # Perfect in default, diverging in variants → variant-diverging
            {"cop": "Style/B", "matches": 30, "fp": 0, "fn": 0,
             "perfect_match": True, "diverging": False},
            # Diverging in default → variant-diverging regardless
            {"cop": "Style/C", "matches": 20, "fp": 5, "fn": 0,
             "perfect_match": False, "diverging": True},
        ],
    }
    variants = tmp_path / "variants.json"
    variants.write_text(json.dumps({
        "batches": [{
            "name": "batch_1",
            "by_cop": [
                # Style/A: 0 FP+FN → still perfect
                {"cop": "Style/A", "style_label": "x", "matches": 40, "fp": 0, "fn": 0},
                # Style/B: has FP → variant-diverging
                {"cop": "Style/B", "style_label": "y", "matches": 25, "fp": 3, "fn": 0},
                # Style/C: already diverging in default
                {"cop": "Style/C", "style_label": "z", "matches": 15, "fp": 2, "fn": 1},
            ],
        }]
    }))

    patch_variant_stats.patch(corpus, variants)

    dept = corpus["by_department"][0]
    # Style/A is variant-perfect, Style/B and Style/C are variant-diverging
    assert dept["variant_perfect_cops"] == 1
    assert dept["variant_diverging_cops"] == 2


def test_patch_synthetic_fallback_for_variant_perfect(tmp_path):
    """Cops unexercised in corpus but synth-perfect count as variant-perfect."""
    corpus = {
        "summary": {"matches": 100, "fp": 0, "fn": 0},
        "by_department": [
            {"department": "RSpecRails", "matches": 100, "fp": 0, "fn": 0},
        ],
        "by_cop": [
            # Corpus-perfect → variant-perfect
            {"cop": "RSpecRails/A", "matches": 50, "fp": 0, "fn": 0,
             "perfect_match": True, "diverging": False, "exercised": True},
            # NOT exercised in corpus, but synth-perfect → should be variant-perfect
            {"cop": "RSpecRails/B", "matches": 0, "fp": 0, "fn": 0,
             "perfect_match": False, "diverging": False, "exercised": False},
            # NOT exercised in corpus, NOT in synthetic → no data, not counted
            {"cop": "RSpecRails/C", "matches": 0, "fp": 0, "fn": 0,
             "perfect_match": False, "diverging": False, "exercised": False},
        ],
    }
    variants = tmp_path / "variants.json"
    variants.write_text(json.dumps({
        "batches": [{
            "name": "batch_1",
            "by_cop": [
                {"cop": "RSpecRails/A", "style_label": "x", "matches": 40, "fp": 0, "fn": 0},
            ],
        }]
    }))
    synthetic = tmp_path / "synthetic.json"
    synthetic.write_text(json.dumps({
        "by_cop": [
            {"cop": "RSpecRails/B", "matches": 5, "fp": 0, "fn": 0,
             "perfect_match": True, "exercised": True, "diverging": False},
        ],
    }))

    patch_variant_stats.patch(corpus, variants, synthetic)

    dept = corpus["by_department"][0]
    # A (corpus-perfect) + B (synth-perfect) = 2 variant-perfect
    assert dept["variant_perfect_cops"] == 2
    assert dept["variant_diverging_cops"] == 0


def test_patch_synthetic_variant_data_merged(tmp_path):
    """Synthetic variant data is merged into variant results."""
    corpus = {
        "summary": {"matches": 0, "fp": 0, "fn": 0},
        "by_department": [
            {"department": "Rails", "matches": 0, "fp": 0, "fn": 0},
        ],
        "by_cop": [
            # Synth-perfect for default, but synth variant diverges
            {"cop": "Rails/X", "matches": 0, "fp": 0, "fn": 0,
             "perfect_match": False, "diverging": False, "exercised": False},
        ],
    }
    # Empty corpus variants
    variants = tmp_path / "variants.json"
    variants.write_text(json.dumps({
        "batches": [{
            "name": "batch_1",
            "by_cop": [],
        }]
    }))
    # Synthetic: default perfect, but variant has FP
    synthetic = tmp_path / "synthetic.json"
    synthetic.write_text(json.dumps({
        "by_cop": [
            {"cop": "Rails/X", "matches": 3, "fp": 0, "fn": 0,
             "perfect_match": True, "exercised": True, "diverging": False},
        ],
        "variants": {
            "batches": [{
                "style_label": "variant_batch_1",
                "by_cop": [
                    {"cop": "Rails/X", "matches": 2, "fp": 1, "fn": 0},
                ],
            }]
        },
    }))

    patch_variant_stats.patch(corpus, variants, synthetic)

    dept = corpus["by_department"][0]
    # Rails/X is synth-default-perfect but synth-variant-diverging
    assert dept["variant_perfect_cops"] == 0
    assert dept["variant_diverging_cops"] == 1


def test_patch_empty_variants(tmp_path):
    corpus = {"summary": {"matches": 100, "fp": 0, "fn": 0}, "by_department": []}
    variants = tmp_path / "variants.json"
    variants.write_text(json.dumps({"batches": [{"name": "b", "by_cop": []}]}))

    result = patch_variant_stats.patch(corpus, variants)
    assert result is False
    assert "variant_overall_match_rate" not in corpus["summary"]
