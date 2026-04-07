---
name: review-agent-prs
description: Review PRs created by the agent cop fix workflow and approve, fix, or close them.
allowed-tools: Bash(*), Read, Write, Edit, Grep, Glob
---

# review-agent-prs

Review PRs created by the agent cop fix workflow. Approve good ones so workflow auto-merge can land them, fix minor issues, or close bad ones.

## Trigger

User runs `/review-agent-prs` (optionally with filters like `--cop Style/*`).

## Workflow

### 1. List open agent PRs with passing CI

```bash
gh pr list --repo 6/nitrocop --label type:cop-fix --state open \
  --json number,title,headRefName,statusCheckRollup,labels,createdAt \
  --jq '[.[] | select(
    (.statusCheckRollup | length > 0) and
    (.statusCheckRollup | all(.status == "COMPLETED")) and
    (.statusCheckRollup | all(.conclusion == "SUCCESS"))
  )] | .[] | "\(.number)\t\(.title)\t\(.labels | map(.name) | join(","))"'
```

This filters to only PRs where all CI checks have passed, skipping drafts with no checks, PRs with pending checks, and PRs with failures — no need to run `gh pr checks` per PR.

Also list PRs with `validation-failed` label separately so they can be closed:

```bash
gh pr list --repo 6/nitrocop --label type:cop-fix,validation-failed --state open \
  --json number,title --jq '.[] | "\(.number)\t\(.title)"'
```

### 2. Check FP/FN impact (required)

Before reviewing code, fetch the PR comments to check actual corpus impact:

```bash
gh pr view <number> --repo 6/nitrocop --json comments --jq '.comments[] | select(.author.login == "github-actions") | .body' | head -40
```

The CI workflow posts cop-check results with FP/FN deltas. Look for the table with `FP Δ` and `FN Δ` columns. If the PR has **zero net FP/FN change** (all deltas are 0), **close it** — the code change adds complexity without improving conformance. Do not trust doc comment claims like "resolves ~N FPs" without verifying against the actual CI comment data.

### 3. Review each PR

For each passing PR that has real FP/FN impact, fetch the diff with `gh pr diff <number>` and review:

**Code correctness:**
- Does the logic match the cop's intended behavior?
- Are edge cases handled?
- Could the fix introduce false positives or false negatives?

**Code quality:**
- Missing explicit parentheses for operator precedence clarity
- Overly verbose or unclear comments
- Unnecessary complexity that could be simplified

**Tests:**
- Are offense and no_offense fixtures adequate?
- Does corrected.rb exist if autocorrect was added?

### 4. Take action on each PR

For each PR, do exactly one of:

**Approve** — code is correct and clean:
```bash
gh pr review <number> --approve --body "Reviewed: logic correct, tests adequate."
```
The workflow enables squash auto-merge, so approval is enough.

**Fix then approve** — code is correct but has quality nits:
1. Check out the PR branch
2. Fix the issues (fmt, readability, simplification)
3. Commit and push
4. Approve the PR:
```bash
gh pr review <number> --approve --body "Reviewed: fixed [description of nits]. Logic correct."
```

**Close** — code is wrong, too complex, or not worth fixing:
```bash
gh pr close <number> --comment "Closing: [reason]. [Specific issues found.]" --delete-branch
```

### 5. Present summary

Show a table of actions taken:
```
| PR   | Cop                         | Action          |
|------|-----------------------------|-----------------|
| #102 | Style/VariableInterpolation | Approved        |
| #105 | Lint/EmptyBlock             | Fixed + approved|
| #106 | Layout/SpaceBeforeComment   | Closed (reason) |
```

## Rules

- Only review PRs with the `type:cop-fix` label
- Agent PRs start as drafts, then the workflow marks them ready after the agent finishes. A draft PR with all CI checks passing is ready for review. Only skip draft PRs that have no checks or pending/failing checks — those are still being worked on.
- Skip PRs with failing or pending CI checks — only review PRs where all checks have passed
- PRs with `validation-failed` label: close with comment explaining why
- PRs with merge conflicts: close with comment, agent can retry on fresh main
- Do not merge PRs — only approve, fix+approve, or close
- When fixing, commit with a clear message explaining what was changed
- When closing, always leave a comment with the specific reason so the dispatch system can learn
- If the diff contains changes to Python files (`.py`), treat this as suspicious — agent cop-fix should only touch Rust code and test fixtures. Flag it to the user before approving.
- If the diff contains only `///` doc comment changes in `.rs` files and no code logic or fixture changes, close the PR. Doc-only PRs indicate the agent couldn't find a code fix — they add git noise without closing any FP/FN gap. The agent should have exited without committing.
- **Model-aware review rigor**: Check the PR labels for `model:*` tags. PRs from lower-quality models (e.g., `model:minimax`) require extra scrutiny:
  - Fix any bad code, bad fixtures (missing trailing newlines, duplicates), or dead code before approving
  - Close PRs that don't actually impact FP/FN counts — if the code change adds complexity without moving the needle, it's not worth merging
  - Close doc-only or doc+test-only PRs (tests that verify already-working behavior without a code fix are not a contribution)
  - Verify the change has a real behavioral impact — refactors that rewrite logic without changing corpus results are noise
- Check for **reimplemented shared infrastructure**: the diff should not add local helper functions that duplicate code in `src/cop/shared/` modules. The shared modules include: `util.rs` (node helpers like `begins_its_line`, `indentation_of`, `is_modifier_if`, `unwrap_parentheses`, etc.), `node_type.rs` (node type constants), `method_identifier_predicates.rs` (method name classification), `literal_predicates.rs` (literal node classification), `access_modifier_predicates.rs` (access modifier detection), `predicate_operator_predicates.rs` (semantic operator checks), `numeric_predicates.rs`, `constant_predicates.rs`, and per-department shared modules (e.g., `style/trailing_comma.rs`, `layout/multiline_literal_brace_layout.rs`). If the agent reimplements any of these locally instead of importing the shared version, this is a "fix then approve" case — deduplicate before approving. The `tests/integration.rs` `shared_module_usage_lint` test enforces this at CI level for known patterns, but reviewers should catch new duplications that CI doesn't yet lint for.
