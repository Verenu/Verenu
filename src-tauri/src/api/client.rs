use std::sync::OnceLock;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

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

static DOWNLOAD: OnceLock<reqwest::Client> = OnceLock::new();

pub fn download() -> &'static reqwest::Client {
    DOWNLOAD.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Verenu/0.15.0")
            .build()
            .expect("local model download client")
    })
}
