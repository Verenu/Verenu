// Only the workload lives here. The implementations come from Verenu's source files.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rounds: usize = args[1].parse().unwrap();
    let candidates = [
        "Microphone (Yeti Nano)",
        "USB Audio Device",
        "HEADSET MIC",
        "Yeti Nano",
    ];
    let contexts = ["Hello.", "hello", "", "One! ", "éclair", "line\n"];
    let mut checksum: u64 = 0;
    for i in 0..rounds {
        let score =
            device_match::device_name_match_score(candidates[i % candidates.len()], "Yeti Nano");
        let context = text_context::classify_context_tail(contexts[i % contexts.len()]);
        checksum = checksum.wrapping_add(score as u64 + context.as_str().len() as u64);
        if context == text_context::SentenceContext::NewSentence {
            checksum = checksum.wrapping_add(1);
        }
    }
    println!("{}", checksum);
}
