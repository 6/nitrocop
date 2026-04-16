#!/usr/bin/env python3
"""Pre-populate offense fixtures with failing corpus FN examples.

Confirmed FN code bugs append ready-made test snippets to offense.rb.
Confirmed FP code bugs stay in task.md as source context for the agent
to distill into a clean no_offense.rb case manually.

Variant fixture files (offense.<variant>.rb / no_offense.<variant>.rb) are
created from variant oracle data for ALL diverging variants of the cop,
providing cross-variant regression protection via `cargo test`.

Usage:
    python3 prepopulate_fixtures.py <task.md> <cop> <fixture_dir>

Reads pre-diagnostic results from task.md, extracts confirmed code bug
examples, and appends only FN snippets to offense.rb.
"""
import re
import sys
from pathlib import Path


def extract_diagnostics_from_task(task_path: Path) -> list[dict]:
    """Parse pre-diagnostic results from the task markdown.

    Looks for FP/FN sections with CODE BUG markers and extracts
    the source context and test snippets."""
    text = task_path.read_text()
    results = []

    # Find all FP CODE BUG sections with source context
    fp_pattern = re.compile(
        r'### FP #\d+:.*?\n'
        r'\*\*CONFIRMED false positive — CODE BUG\*\*.*?'
        r'(?:Full source context.*?```ruby\n(.*?)```|Add to no_offense\.rb:\n```ruby\n(.*?)```)',
        re.DOTALL,
    )
    for m in fp_pattern.finditer(text):
        source = m.group(1) or m.group(2)
        if source and source.strip():
            results.append({"kind": "fp", "source": source.strip()})

    # Find all FN CODE BUG sections with test snippets
    fn_pattern = re.compile(
        r'### FN #\d+:.*?\n'
        r'\*\*NOT DETECTED — CODE BUG\*\*.*?'
        r'Ready-made test snippet.*?```ruby\n(.*?)```',
        re.DOTALL,
    )
    for m in fn_pattern.finditer(text):
        snippet = m.group(1)
        if snippet and snippet.strip():
            results.append({"kind": "fn", "source": snippet.strip()})

    return results


def extract_variant_examples_from_task(task_path: Path) -> list[dict]:
    """Parse variant FP/FN examples from the task markdown.

    Returns list of {variant, kind (fp/fn), source, message} dicts.
    Extracts from the 'Variant FP/FN Examples' or 'Style Variant Divergence'
    sections."""
    text = task_path.read_text()
    results = []

    # Find variant example sections: ### Style: `<label>`
    variant_pattern = re.compile(
        r'### Style: `([^`]+)`\s*\n(.*?)(?=### Style:|### How to fix|## [A-Z]|\Z)',
        re.DOTALL,
    )
    for vm in variant_pattern.finditer(text):
        variant_label = vm.group(1).strip()
        section = vm.group(2)

        # Extract FP examples (code blocks under "False Positives")
        fp_section = re.search(
            r'\*\*False Positives\*\*.*?\n(.*?)(?=\*\*False Negatives\*\*|\Z)',
            section, re.DOTALL,
        )
        if fp_section:
            for code_match in re.finditer(r'```ruby\n(.*?)```', fp_section.group(1), re.DOTALL):
                source = code_match.group(1).strip()
                if source:
                    results.append({
                        "variant": variant_label,
                        "kind": "fp",
                        "source": source,
                    })

        # Extract FN examples (code blocks under "False Negatives")
        fn_section = re.search(
            r'\*\*False Negatives\*\*.*?\n(.*?)(?=\*\*False Positives\*\*|\Z)',
            section, re.DOTALL,
        )
        if fn_section:
            for code_match in re.finditer(r'```ruby\n(.*?)```', fn_section.group(1), re.DOTALL):
                source = code_match.group(1).strip()
                if source:
                    results.append({
                        "variant": variant_label,
                        "kind": "fn",
                        "source": source,
                    })

    return results


def infer_config_directive(variant_label: str) -> str:
    """Infer the # nitrocop-config: directive from a variant label.

    Labels are typically style values like 'comma', 'semantic', 'tabs'.
    Multi-param labels use commas: 'separator, separator, always_ignore'."""
    parts = [p.strip() for p in variant_label.split(",")]
    if len(parts) == 1:
        return f"# nitrocop-config: EnforcedStyle: {parts[0]}"
    # Multi-param: try common patterns
    # For now, join as-is — the agent can refine
    return f"# nitrocop-config: EnforcedStyle: {variant_label}"


MIN_FN_SNIPPET_LINES = 2
"""Minimum lines for an FN snippet to be valid as an offense fixture.

Corpus FN examples are often just the first line of a multi-line construct
(e.g., a bare `if` without the body/else/end). These incomplete single-line
fragments can't trigger the cop and cause fixture test failures."""


def normalize_fixture_snippet(source: str) -> str:
    """Trim noisy boundary lines from extracted corpus snippets.

    The corpus context sometimes includes leading/trailing blank lines or
    comment-only spacer lines (`#`) that are not useful fixture content.
    Keep interior spacing intact, but strip those boundary markers so the
    pre-populated fixtures stay readable.
    """
    lines = source.splitlines()

    def is_boundary_noise(line: str) -> bool:
        stripped = line.strip()
        return stripped == "" or stripped == "#"

    while lines and is_boundary_noise(lines[0]):
        lines.pop(0)
    while lines and is_boundary_noise(lines[-1]):
        lines.pop()

    return "\n".join(lines).rstrip()


def prepopulate(task_path: Path, cop: str, fixture_dir: Path) -> dict:
    """Append confirmed FN code bug examples to offense.rb.

    Also creates variant fixture files from variant oracle examples.
    Returns {"fp_context": int, "fn_added": int, "variant_files": int}."""
    diagnostics = extract_diagnostics_from_task(task_path)

    offense_path = fixture_dir / "offense.rb"
    fn_added = 0

    fp_examples = [d for d in diagnostics if d["kind"] == "fp"]

    # Append FN examples to offense.rb
    fn_examples = [d for d in diagnostics if d["kind"] == "fn"]
    if fn_examples and offense_path.exists():
        with open(offense_path, "a") as f:
            for ex in fn_examples:
                snippet = normalize_fixture_snippet(ex["source"])
                if not snippet:
                    continue
                # Skip snippets too short to be valid offense fixtures
                if snippet.count("\n") + 1 < MIN_FN_SNIPPET_LINES:
                    continue
                f.write(f"\n{snippet}\n")
                fn_added += 1

    # Create variant fixture files from ALL diverging variants
    variant_examples = extract_variant_examples_from_task(task_path)
    variant_files = 0

    # Group by variant and kind
    by_variant: dict[str, dict[str, list[str]]] = {}
    for ex in variant_examples:
        vd = by_variant.setdefault(ex["variant"], {"fp": [], "fn": []})
        vd[ex["kind"]].append(ex["source"])

    for variant_label, examples in by_variant.items():
        # Sanitize variant name for filename: "comma" -> "comma",
        # "separator, separator" -> skip (multi-param too complex for auto-fixture)
        if "," in variant_label:
            continue

        variant_slug = re.sub(r"[^a-z0-9]+", "_", variant_label.lower()).strip("_")
        config_directive = infer_config_directive(variant_label)

        # FP examples → no_offense.<variant>.rb
        # (patterns nitrocop flags but RuboCop does not — should NOT flag)
        if examples["fp"]:
            no_offense_path = fixture_dir / f"no_offense.{variant_slug}.rb"
            if not no_offense_path.exists():
                snippets = []
                for src in examples["fp"][:5]:  # cap at 5 per variant
                    snippet = normalize_fixture_snippet(src)
                    if snippet:
                        snippets.append(snippet)
                if snippets:
                    content = config_directive + "\n" + "\n\n".join(snippets) + "\n"
                    no_offense_path.write_text(content)
                    variant_files += 1

        # FN examples → offense.<variant>.rb
        # (patterns RuboCop flags but nitrocop misses — SHOULD flag)
        # Note: these need ^ annotations to be valid offense fixtures.
        # We add them as comments since we don't have annotation data.
        # The agent must add proper annotations.
        if examples["fn"]:
            offense_path_v = fixture_dir / f"offense.{variant_slug}.rb"
            if not offense_path_v.exists():
                snippets = []
                for src in examples["fn"][:5]:
                    snippet = normalize_fixture_snippet(src)
                    if not snippet:
                        continue
                    if snippet.count("\n") + 1 < MIN_FN_SNIPPET_LINES:
                        continue
                    snippets.append(snippet)
                if snippets:
                    header = (
                        f"{config_directive}\n"
                        f"# TODO: Add ^ offense annotations below.\n"
                        f"# These are FN examples from the corpus — RuboCop flags them\n"
                        f"# but nitrocop misses. The agent must add proper annotations.\n"
                    )
                    content = header + "\n".join(snippets) + "\n"
                    offense_path_v.write_text(content)
                    variant_files += 1

    return {
        "fp_context": len(fp_examples),
        "fn_added": fn_added,
        "variant_files": variant_files,
    }


def main():
    if len(sys.argv) != 4:
        print(f"Usage: {sys.argv[0]} <task.md> <cop> <fixture_dir>", file=sys.stderr)
        sys.exit(1)

    task_path = Path(sys.argv[1])
    cop = sys.argv[2]
    fixture_dir = Path(sys.argv[3])

    if not task_path.exists():
        print(f"Error: {task_path} not found", file=sys.stderr)
        sys.exit(1)

    if not fixture_dir.exists():
        print(f"Error: {fixture_dir} not found", file=sys.stderr)
        sys.exit(1)

    result = prepopulate(task_path, cop, fixture_dir)
    print(
        f"Left {result['fp_context']} FP examples in task.md for manual no_offense.rb distillation"
    )
    print(f"Added {result['fn_added']} FN examples to offense.rb")
    print(f"Created {result['variant_files']} variant fixture files")


if __name__ == "__main__":
    main()
