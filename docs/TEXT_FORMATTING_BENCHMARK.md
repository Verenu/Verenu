# Insertion formatting regression benchmark

The corpus in `src-tauri/src/core/text_context_cases.rs` asserts exact inserted
text, including both caret edges. It covers sentence-ending punctuation crossed
with quotes, brackets, Markdown closers, whitespace and right-hand text, plus
abbreviations, decimals, URLs, contractions, protected casing and empty input.
The Rust unit suite also checks idempotence and unreliable-context independence.

Run the standalone harness against the production formatter without compiling
the desktop shell:

```bash
mkdir -p test-results/formatting
VERENU_TEXT_CONTEXT_SOURCE="$PWD/src-tauri/src/core/text_context.rs" \
  rustc -O --edition=2021 scripts/bench-text-formatting.rs \
  -o test-results/formatting/current
test-results/formatting/current --check
```

For a before/after comparison, extract the original `text_context.rs` with
`git show BASE:src-tauri/src/core/text_context.rs` into the ignored results
directory, compile the same harness with that absolute source path, and alternate
the two executables at least five times. Omit `--check` for the old executable
to measure it while still reporting its correctness failures. Report those
failures alongside timing; incorrect output is not a performance win.

Each process checks the corpus before timing six rounds of 1,000 iterations per
case. Round zero warms the executable. Inputs and returned strings pass through
`black_box`; the timed region includes production decisions and output allocation,
but excludes corpus construction and correctness comparisons. Report median and
range for the remaining rounds, compiler version, machine load, and corpus size.
Do not run concurrent builds during measurements.

This measures the deterministic insertion formatter only. It does not measure
speech recognition, model cleanup, accessibility probing or clipboard delivery,
and does not establish end-to-end dictation speed or platform integration.
