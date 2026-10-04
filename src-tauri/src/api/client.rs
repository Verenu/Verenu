use std::sync::OnceLock;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Returns the process-wide shared reqwest client.
/// Reusing one client enables TCP connection pooling and TLS session reuse,
/// saving ~200-400ms per request compared to Client::new() each time.
pub fn get() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("shared reqwest client")
    })
}

static HARDENED: OnceLock<reqwest::Client> = OnceLock::new();

/// Client for user-supplied endpoints. It never follows redirects, so a custom
/// provider can't bounce the request (and the key in its auth header) to a
/// different host than the one the user approved.
pub fn hardened() -> &'static reqwest::Client {
    HARDENED.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("hardened reqwest client")
    })
}
