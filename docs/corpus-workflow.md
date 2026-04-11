# Corpus Workflow

## Investigation

Use the cached corpus tools before rerunning expensive checks:

```bash
python3 scripts/check_cop.py Department/CopName --verbose           # per-repo breakdown
python3 scripts/check_cop.py Department/CopName --examples          # FP/FN locations with source context
python3 scripts/reduce_mismatch.py Department/CopName repo_id path/to/file.rb:line
```

`check_cop.py` downloads the latest corpus artifact automatically. Do not manually download artifacts first.

## Regression Checks

Use `check_cop.py` for aggregate corpus validation after a fix:

```bash
python3 scripts/check_cop.py Department/CopName
python3 scripts/check_cop.py Department/CopName --verbose
python3 scripts/check_cop.py Department/CopName --verbose --rerun
python3 scripts/check_cop.py Department/CopName --verbose --rerun --all-repos  # full scan, local only
```

Important:

- `check_cop.py` is count-based. It does not prove exact location matches.
- “file-drop noise” is not an excuse for real FN gaps. Investigate the actual missed examples.

## Corpus Bundle Notes

`check_cop.py --rerun` and `--corpus-check` require the active Ruby bundle under `bench/corpus/vendor/bundle/`.

```bash
cd bench/corpus
BUNDLE_PATH=vendor/bundle bundle install
```

If `bundle info rubocop` fails, config resolution falls back to defaults and counts become unreliable.

## Dispatch And Repair

Issue-backed cop dispatch:

```bash
python3 scripts/dispatch_cops.py issues-sync --binary target/debug/nitrocop
python3 scripts/dispatch_cops.py backend --cop Department/CopName --binary target/debug/nitrocop
```

## Style-Variant Testing

The corpus oracle can test every cop with every supported config style, not
just defaults. This catches bugs in non-default code paths (e.g., `EnforcedStyle: comma`).

### Per-style corpus check (local)

Override a single style parameter for a local corpus check:

```bash
python3 scripts/check_cop.py Department/CopName \
    --style EnforcedStyleForMultiline=comma \
    --rerun --clone --sample 30
```

### Exhaustive local check

Check a specific variant style:

```bash
python3 scripts/check_cop.py Department/CopName --rerun --style EnforcedStyle=value
```

### Variant batch configs (CI)

Variant batch configs (`bench/corpus/variant_batches/`) are auto-generated
from `vendor/rubocop/config/default.yml`:

```bash
python3 bench/corpus/gen_variant_batches.py --output-dir bench/corpus/variant_batches/
```

The corpus oracle runs these when triggered with `run_style_variants: true`.
Results appear as per-variant rows in `docs/corpus.md`.

### Audit coverage

To see which style variants lack test coverage:

```bash
python3 scripts/audit_style_coverage.py
python3 scripts/audit_style_coverage.py --department Style --untested-only
```

## Bench And Conformance

Use the full bench/conformance flow only when you explicitly need repo-wide regeneration:

```bash
cargo run --release --bin bench_nitrocop -- conform
cargo run --release --bin bench_nitrocop -- quick
cargo run --release --bin bench_nitrocop -- autocorrect-conform
```

Avoid regenerating bench outputs during routine cop-fix loops unless the task explicitly calls for it.
