---
name: repair-variant-regression
description: Fix a cop-check regression (default or variant) on a bot PR using CI failure details — fetch failing source lines, add fixtures, fix the cop, push to the PR branch. CI does the heavy lifting.
---

# Repair Cop-Check Regression

Fix a +FP or +FN regression on a bot cop-fix PR using the CI failure details.
Works for both default-config and variant (EnforcedStyle) regressions.
The user provides:
1. A PR URL (e.g., `https://github.com/6/nitrocop/pull/1553`)
2. The CI failure line, which contains the cop name, variant, delta, and per-repo locations

Example CI failure line:
```
Style/EmptyStringInsideInterpolation (ternary)    2    11    3    0    +1    -11    ❌
Repos: basecamp__kamal__9c6252d(FP:lib/kamal/secrets.rb:44) e621ng__e621ng__cd2b40f(FP:test/unit/tag_query_test.rb:633) edavis10__redmine__2d6f552(FP:app/helpers/issues_helper.rb:68)
```

## Workflow

### Step 1: Parse the CI failure

Extract from the CI line:
- **Cop name** and **variant** (e.g., `Style/EmptyStringInsideInterpolation`, `EnforcedStyle=ternary`)
- **Delta**: which direction regressed (+FP or +FN)
- **Repo locations**: each entry has `repo_id(FP_or_FN:path:line)`

### Step 2: Checkout the PR branch

```bash
gh pr checkout <pr-number>
```

### Step 3: Fetch the failing source lines

For each repo location from the CI line, fetch the actual source code at the
exact corpus commit. Do NOT clone the repos.

1. Look up the repo in `bench/corpus/manifest.jsonl` to get the GitHub `repo_url` and `sha`:
   ```bash
   grep '<repo_id>' bench/corpus/manifest.jsonl
   ```

2. Fetch the file content at the pinned commit via GitHub API:
   ```bash
   gh api repos/{owner}/{repo}/contents/{path}?ref={sha} --jq '.content' | base64 -d | sed -n '{start},{end}p'
   ```
   Fetch ~15 lines of context around the failing line.

3. Record for each location:
   - The actual Ruby code at that line
   - Whether it's FP (nitrocop fires but RuboCop doesn't) or FN (RuboCop fires but nitrocop doesn't)
   - What pattern is triggering the mismatch

### Step 4: Understand root cause

Read the cop source and the vendor RuboCop implementation:
```bash
# Cop source
cat src/cop/<dept>/<cop_name>.rs

# RuboCop reference
find vendor/rubocop* -path "*/<cop_name_snake>*" -name "*.rb" | head -5
```

Compare behavior. Common root causes for variant regressions:
- Recursive visitor walking too deep (should only check direct children)
- Missing style-specific branching
- Overly broad pattern matching
- Config option not being respected

### Step 5: Add fixtures from the failing lines

For each failing location, distill the pattern into a minimal fixture.

**Where fixtures go depends on whether the regression is in a variant or default config:**

For **default config** regressions (no variant in the CI line):
- FP locations → `tests/fixtures/cops/<dept>/<cop_name>/no_offense.rb`
- FN locations → `tests/fixtures/cops/<dept>/<cop_name>/offense.rb`

For **variant** regressions (CI line shows e.g. `(ternary)`, `(comma)`):
- Check if variant-specific fixtures already exist on the branch
  (e.g., `ternary_offense.rb`, `ternary_no_offense.rb`)
- If yes, add to them
- If no, create them with a `# nitrocop-config:` directive at the top:
  ```ruby
  # nitrocop-config: EnforcedStyle: <variant>
  ```
- FP locations → `<variant>_no_offense.rb` (or main `no_offense.rb` if no
  variant fixture exists and the pattern isn't variant-specific)
- FN locations → `<variant>_offense.rb`

Each fixture case should have a brief comment referencing the repo pattern:
```ruby
# FP fix: modifier if nested inside method call, not a direct child
"#{h(x) + ("..." if cond)}"
```

### Step 6: Run tests (expect failure)

```bash
cargo test --lib -- cop::<dept>::<cop_name_snake>
```

Confirm the new fixtures fail (TDD). If they already pass, the regression
may be pre-existing — note this and skip to Step 8.

### Step 7: Fix the cop

Edit `src/cop/<dept>/<cop_name>.rs` to fix the regression. Keep the fix
minimal and targeted.

Run tests until they pass:
```bash
cargo test --lib -- cop::<dept>::<cop_name_snake>
```

### Step 8: Pre-commit checks

```bash
cargo fmt -- src/cop/<dept>/<cop_name>.rs
cargo clippy --release -- -D warnings
cargo test --release -p nitrocop --lib -- cop::<dept>::<cop_name_snake>
```

### Step 9: Commit and push

```bash
git add src/cop/<dept>/<cop_name>.rs tests/fixtures/cops/<dept>/<cop_name>/
git commit -m "Fix <variant> variant regression: <brief description>

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
git push origin HEAD
```

CI validates the fix. Do NOT run `check_cop.py` locally — let CI do it.

## Key principles

- **CI does the heavy lifting.** Don't run corpus checks locally. Push and let CI validate.
- **Fetch, don't clone.** Use `gh api` + manifest.jsonl to grab specific files at pinned commits.
- **Every failing line becomes a fixture.** The CI report gives you exact locations — turn each into a test case.
- **Start minimal, refactor if needed.** Try the narrowest fix first, but if the
  regression reveals a fundamentally wrong approach (e.g., recursive visitor when
  RuboCop only checks direct children), replace the approach rather than patching
  around it.
- **Don't run corpus tooling to find issues.** The CI failure line already gives
  you exact repo, file, line, and FP/FN type — that IS the diagnosis. Fetching
  the source at those lines and reading the cop/RuboCop source is all you need.
  `investigate_cop.py` has no `--style` flag and is useless here. `check_cop.py
  --style` and `verify_cop_locations.py --style` exist but only help verify a
  fix after the fact — CI already does that when you push. Don't burn time
  running them locally.
