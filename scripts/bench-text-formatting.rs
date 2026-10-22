//! Compile with VERENU_TEXT_CONTEXT_SOURCE set to the absolute production source path:
//! rustc -O --edition=2021 scripts/bench-text-formatting.rs -o test-results/bench-format
//! Run with --check to fail on any corpus mismatch; without it, report baseline failures too.
#![allow(dead_code)]
mod formatter {
    include!(env!("VERENU_TEXT_CONTEXT_SOURCE"));
}
mod corpus {
    include!("../src-tauri/src/core/text_context_cases.rs");
}
use formatter::{decide_insertion, CaretTextContext};
use std::{hint::black_box, time::Instant};

fn format(text: &str, left: &str, right: &str) -> String {
    decide_insertion(
        text,
        CaretTextContext {
            left,
            right,
            left_reliable: true,
            right_reliable: true,
            language: "en",
            casing_enabled: true,
            preserve_sentence_case: false,
            protected_initial_case: false,
        },
    )
    .text
}

fn main() {
    let cases = corpus::insertion_cases();
    let mut failures = 0;
    for (text, left, right, expected) in &cases {
        let actual = format(text, left, right);
        if actual != *expected {
            failures += 1;
            if failures <= 12 {
                eprintln!(
                    "left={left:?} text={text:?} right={right:?}: {actual:?} != {expected:?}"
                );
            }
        }
    }
    println!("cases={} failures={failures}", cases.len());
    if failures > 0 && std::env::args().any(|arg| arg == "--check") {
        std::process::exit(1);
    }
    for round in 0..6 {
        let start = Instant::now();
        for _ in 0..1000 {
            for (text, left, right, _) in &cases {
                black_box(format(black_box(text), black_box(left), black_box(right)));
            }
        }
        println!(
            "round={round} ns_per_insertion={:.1}",
            start.elapsed().as_nanos() as f64 / (cases.len() * 1000) as f64
        );
    }
}
