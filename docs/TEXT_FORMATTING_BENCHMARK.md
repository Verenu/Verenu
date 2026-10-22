# Insertion formatting regression benchmark

The corpus in `src-tauri/src/core/text_context_cases.rs` asserts exact inserted
text, including both caret edges. It covers sentence-ending punctuation crossed
with quotes, brackets, Markdown closers, whitespace and right-hand text, plus
abbreviations, decimals, URLs, contractions, protected casing and empty input.
The Rust unit suite also checks idempotence and unreliable-context independence.

At a prose boundary, a dotted `@domain` fragment joins an existing lowercase
email-like token, and a `#fragment` joins a slash-delimited URL or path prefix.
Handles such as `@alice` and hashtags in prose still receive a separating
space. A single-dash fragment such as `well` + `-known` is ambiguous with an
option such as `-help`; the formatter keeps the prose-boundary space instead
of guessing from a word list. Use a space or a complete lexical item when the
intended boundary needs to be unambiguous.

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

## Fragment fix measurement (2026-10-21)

The final formatter at `e3285d69001a9082a7082ff51058d9377aa34740` was compared with the pre-fix source at `0136168058a1d96abd560e928feb5661d3cbde78` using the 5,172-case corpus. The pre-fix source had 16 expected-output mismatches; the fixed source had none. Five optimized processes per side alternated, with five measured rounds per process and the warmup round excluded (25 samples per side).

| Source | Median | Range |
| --- | ---: | ---: |
| Before | 218.8 ns/insertion | 208.2–259.3 ns/insertion |
| After | 221.4 ns/insertion | 218.4–275.1 ns/insertion |

The fixed source median is 2.6 ns (1.2%) higher, but the sample ranges overlap
and vary substantially, so this result is inconclusive and does not establish a
measurable latency change. `rustc 1.99.1` compiled both sides with `-O`. The
host had 12 cores and load averages of `6.50, 6.03, 5.24` at the start and
`6.72, 6.24, 5.38` at the end; no concurrent builds were launched by this task.
`perf` was unavailable, so no call-level profile or exact instruction-level
limiter is claimed.
