pub mod collection_db;
pub mod download;
pub mod filter;
pub mod i18n;
pub mod job;
pub mod lazer;
pub mod lazer_scan;
pub mod md5lookup;
pub mod osucollector;
pub mod storage;
pub mod trim;

use std::time::Duration;

pub const USER_AGENT: &str = concat!(
    "CollectorToLazer/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/X4vi3/osu-collector-lazer)"
);

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client")
}
