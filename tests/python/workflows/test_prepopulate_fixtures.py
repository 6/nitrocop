#!/usr/bin/env python3
"""Tests for prepopulate_fixtures.py."""
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[3] / "scripts" / "workflows"))

import prepopulate_fixtures

TASK_WITH_FP = """
# Fix Lint/AmbiguousRange — 4 FP, 0 FN

## Pre-diagnostic Results

### FP #1: `repo: file.rb:10`
**CONFIRMED false positive — CODE BUG**
nitrocop incorrectly flags this pattern in isolation.
Fix the detection logic to not flag this.

Full source context (add relevant parts to no_offense.rb):
```ruby
1.. ..1
```

### FP #2: `repo: file.rb:20`
**CONFIRMED false positive — CODE BUG**
nitrocop incorrectly flags this pattern in isolation.
Fix the detection logic to not flag this.

Full source context (add relevant parts to no_offense.rb):
```ruby
def get_text(start)
  @string[start..@pos-1]
end
```
"""

TASK_WITH_FN = """
# Fix Style/Foo — 0 FP, 2 FN

## Pre-diagnostic Results

### FN #1: `repo: file.rb:5`
**NOT DETECTED — CODE BUG**
The cop fails to detect this pattern. Fix the detection logic.

Ready-made test snippet (add to offense.rb, adjust `^` count):
```ruby
super { |x| x.foo }
      ^^^^^^^^^^^^^^^^^ Style/Foo: Pass `&:foo` as an argument.
```
"""

TASK_WITH_SHORT_FN = """
# Fix Style/Foo — 0 FP, 2 FN

## Pre-diagnostic Results

### FN #1: `repo: file.rb:5`
**NOT DETECTED — CODE BUG**
The cop fails to detect this pattern. Fix the detection logic.

Ready-made test snippet (add to offense.rb, adjust `^` count):
```ruby
if item.to_s == pagy.page.to_s
```
"""

TASK_WITH_CONFIG_ONLY = """
# Fix Style/Bar — 0 FP, 1 FN

## Pre-diagnostic Results

### FN #1: `repo: file.rb:5`
**DETECTED in isolation — CONFIG/CONTEXT issue**
The cop correctly detects this pattern with default config.
"""

TASK_MIXED = """
# Fix Lint/Baz — 1 FP, 1 FN

## Pre-diagnostic Results

### FN #1: `repo: file.rb:5`
**NOT DETECTED — CODE BUG**
The cop fails to detect this pattern. Fix the detection logic.

Ready-made test snippet (add to offense.rb, adjust `^` count):
```ruby
some_pattern
^ Lint/Baz: Bad pattern.
```

### FP #1: `repo: file.rb:10`
**CONFIRMED false positive — CODE BUG**
nitrocop incorrectly flags this pattern in isolation.
Fix the detection logic to not flag this.

Full source context (add relevant parts to no_offense.rb):
```ruby
safe_pattern_here
```
"""

TASK_WITH_NOISY_BOUNDARIES = """
# Fix Style/MixinUsage — 1 FP, 0 FN

## Pre-diagnostic Results

### FP #1: `repo: file.rb:10`
**CONFIRMED false positive — CODE BUG**
nitrocop incorrectly flags this pattern in isolation.

Full source context (add relevant parts to no_offense.rb):
```ruby
#

BEGIN {
  include UtilityFunctions
}

#
```
"""


def make_fixtures(tmp: Path):
    """Create minimal fixture files."""
    (tmp / "offense.rb").write_text("# existing offenses\nfoo\n")
    (tmp / "no_offense.rb").write_text("# existing no-offenses\nbar\nbaz\nqux\nquux\nquuz\n")


def test_fp_stays_in_task_context():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_FP)
        result = prepopulate_fixtures.prepopulate(task, "Lint/AmbiguousRange", tmp)
        assert result["fp_context"] == 2
        assert result["fn_added"] == 0
        content = (tmp / "no_offense.rb").read_text()
        assert "1.. ..1" not in content
        assert "@pos-1" not in content
        assert "Pre-populated from corpus" not in content


def test_fn_appended_to_offense():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_FN)
        result = prepopulate_fixtures.prepopulate(task, "Style/Foo", tmp)
        assert result["fn_added"] == 1
        assert result["fp_context"] == 0
        content = (tmp / "offense.rb").read_text()
        assert "super { |x| x.foo }" in content
        assert "Style/Foo" in content
        assert "Pre-populated from corpus" not in content


def test_config_only_no_changes():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_CONFIG_ONLY)
        result = prepopulate_fixtures.prepopulate(task, "Style/Bar", tmp)
        assert result["fp_context"] == 0
        assert result["fn_added"] == 0


def test_mixed_fp_and_fn():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_MIXED)
        result = prepopulate_fixtures.prepopulate(task, "Lint/Baz", tmp)
        assert result["fp_context"] == 1
        assert result["fn_added"] == 1
        assert "safe_pattern_here" not in (tmp / "no_offense.rb").read_text()
        assert "some_pattern" in (tmp / "offense.rb").read_text()


def test_empty_task():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text("# Nothing here")
        result = prepopulate_fixtures.prepopulate(task, "Style/Foo", tmp)
        assert result["fp_context"] == 0
        assert result["fn_added"] == 0


def test_fp_noise_is_not_written_into_no_offense():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_NOISY_BOUNDARIES)
        result = prepopulate_fixtures.prepopulate(task, "Style/MixinUsage", tmp)
        assert result["fp_context"] == 1
        content = (tmp / "no_offense.rb").read_text()
        assert "BEGIN {" not in content
        assert "Pre-populated from corpus" not in content


TASK_WITH_VARIANT_EXAMPLES = """
# Fix Style/BlockDelimiters — variant divergence: 100 FP, 50 FN

## Variant FP/FN Examples

### Style: `semantic`

**False Positives** (100 total — nitrocop flags these but RuboCop does not with `semantic`):

- `AlchemyCMS__alchemy_cms: app/controllers/foo.rb:73`
  Message: Prefer `{...}` over `do...end` for functional blocks.
  ```ruby
  items.map do |x|
    x.name
  end
  ```

- `Arachni__arachni: lib/arachni/browser.rb:872`
  Message: Prefer `{...}` over `do...end` for functional blocks.
  ```ruby
  results.select do |r|
    r.valid?
  end
  ```

**False Negatives** (50 total — RuboCop flags these but nitrocop misses with `semantic`):

- `SomeRepo: file.rb:10`
  Message: Prefer `do...end` over `{...}` for procedural blocks.
  ```ruby
  items.each { |x|
    puts x
  }
  ```

### Style: `always_braces`

**False Positives** (3 total — nitrocop flags these but RuboCop does not with `always_braces`):

- `OtherRepo: app/models/foo.rb:5`
  Message: Prefer `{...}` over `do...end` for blocks.
  ```ruby
  foo.bar do
    baz
  end
  ```
"""

TASK_WITH_MULTI_PARAM_VARIANT = """
# Fix Layout/HashAlignment — variant divergence

## Variant FP/FN Examples

### Style: `separator, separator, always_ignore`

**False Positives** (5 total):

- `Repo: file.rb:10`
  ```ruby
  { a: 1, b: 2 }
  ```
"""


def test_variant_fp_creates_no_offense_fixture():
    """Variant FP examples create no_offense.<variant>.rb with config directive."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_VARIANT_EXAMPLES)
        result = prepopulate_fixtures.prepopulate(task, "Style/BlockDelimiters", tmp)
        assert result["variant_files"] >= 1

        # Check semantic no_offense fixture was created
        semantic_no = tmp / "no_offense.semantic.rb"
        assert semantic_no.exists(), "no_offense.semantic.rb should be created"
        content = semantic_no.read_text()
        assert content.startswith("# nitrocop-config: EnforcedStyle: semantic")
        assert "items.map do |x|" in content

        # Check always_braces no_offense fixture
        braces_no = tmp / "no_offense.always_braces.rb"
        assert braces_no.exists(), "no_offense.always_braces.rb should be created"
        content = braces_no.read_text()
        assert content.startswith("# nitrocop-config: EnforcedStyle: always_braces")
        assert "foo.bar do" in content


def test_variant_fn_creates_offense_fixture():
    """Variant FN examples create offense.<variant>.rb with config directive."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_VARIANT_EXAMPLES)
        prepopulate_fixtures.prepopulate(task, "Style/BlockDelimiters", tmp)

        # Check semantic offense fixture
        semantic_off = tmp / "offense.semantic.rb"
        assert semantic_off.exists(), "offense.semantic.rb should be created"
        content = semantic_off.read_text()
        assert content.startswith("# nitrocop-config: EnforcedStyle: semantic")
        assert "items.each { |x|" in content
        assert "TODO" in content  # should note annotations needed


def test_variant_multi_param_skipped():
    """Multi-param variant labels (with commas) are skipped."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_MULTI_PARAM_VARIANT)
        result = prepopulate_fixtures.prepopulate(task, "Layout/HashAlignment", tmp)
        assert result["variant_files"] == 0
        # No fixture files created for multi-param variants
        rb_files = list(tmp.glob("*.separator*.rb"))
        assert len(rb_files) == 0


def test_variant_fixtures_not_overwritten():
    """Pre-existing variant fixtures are not overwritten."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        existing = tmp / "no_offense.semantic.rb"
        existing.write_text("# nitrocop-config: EnforcedStyle: semantic\n# existing content\n")
        task = tmp / "task.md"
        task.write_text(TASK_WITH_VARIANT_EXAMPLES)
        prepopulate_fixtures.prepopulate(task, "Style/BlockDelimiters", tmp)
        content = existing.read_text()
        assert "existing content" in content
        assert "items.map" not in content


def test_short_fn_snippets_skipped():
    """Single-line FN snippets are too short to be valid offense fixtures."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_SHORT_FN)
        result = prepopulate_fixtures.prepopulate(task, "Style/Foo", tmp)
        assert result["fn_added"] == 0
        content = (tmp / "offense.rb").read_text()
        assert "pagy" not in content


def test_variant_no_examples_no_files():
    """Task without variant examples creates no variant files."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        make_fixtures(tmp)
        task = tmp / "task.md"
        task.write_text(TASK_WITH_FN)  # default FN only, no variants
        result = prepopulate_fixtures.prepopulate(task, "Style/Foo", tmp)
        assert result["variant_files"] == 0
        variant_files = [f for f in tmp.iterdir() if "semantic" in f.name or "comma" in f.name]
        assert len(variant_files) == 0


if __name__ == "__main__":
    test_fp_stays_in_task_context()
    test_fn_appended_to_offense()
    test_config_only_no_changes()
    test_mixed_fp_and_fn()
    test_empty_task()
    test_fp_noise_is_not_written_into_no_offense()
    test_short_fn_snippets_skipped()
    test_variant_fp_creates_no_offense_fixture()
    test_variant_fn_creates_offense_fixture()
    test_variant_multi_param_skipped()
    test_variant_fixtures_not_overwritten()
    test_variant_no_examples_no_files()
    print("All tests passed.")
