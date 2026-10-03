//! Deterministic allocation comparison. No provider calls or credentials.
//! Run: cargo run --manifest-path src-tauri/Cargo.toml --example audio-request-memory

#[path = "../src/api/base64_audio.rs"]
mod base64_audio;

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::Bytes;
use serde::Serialize;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn add(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::SeqCst) + bytes;
    PEAK.fetch_max(live, Ordering::SeqCst);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc_zeroed(layout);
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = System.realloc(pointer, layout, size);
        if !pointer.is_null() {
            if size >= layout.size() {
                add(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::SeqCst);
            }
        }
        pointer
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Serialize)]
struct Request<T> {
    model: &'static str,
    language: &'static str,
    input_audio: Input<T>,
}

#[derive(Serialize)]
struct Input<T> {
    data: T,
    format: &'static str,
}

fn fixture<T>(data: T) -> Request<T> {
    Request {
        model: "test-model",
        language: "en",
        input_audio: Input {
            data,
            format: "wav",
        },
    }
}

fn measured<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    let baseline = LIVE.load(Ordering::SeqCst);
    PEAK.store(baseline, Ordering::SeqCst);
    let result = operation();
    (result, PEAK.load(Ordering::SeqCst).saturating_sub(baseline))
}

fn main() {
    let client = reqwest::Client::new();
    for seconds in [60, 900] {
        let audio = Bytes::from(vec![0u8; 44 + 16_000 * seconds * 2]);
        let legacy_request = client.post("https://example.invalid/transcribe");
        let new_request = client.post("https://example.invalid/transcribe");
        let (legacy, before) = measured(|| {
            let body = fixture(STANDARD.encode(&audio));
            legacy_request.json(&body).build().unwrap()
        });
        let (current, after) = measured(|| {
            let body = fixture(base64_audio::Base64Audio(audio.clone()));
            base64_audio::json_request(
                new_request,
                &body,
                audio.len(),
                "test-model".len() + "en".len(),
            )
            .unwrap()
            .build()
            .unwrap()
        });
        assert_eq!(
            legacy.body().unwrap().as_bytes(),
            current.body().unwrap().as_bytes()
        );
        assert!(
            after < before / 2,
            "JSON request peak allocation should at least halve"
        );
        println!("{seconds}s JSON request peak live requested heap bytes excluding source WAV: before={before} after={after}");
    }
}
