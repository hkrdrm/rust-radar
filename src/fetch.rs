//! HTTP access behind a trait so pollers can be tested without the network.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context};
use async_trait::async_trait;

#[async_trait]
pub trait Fetcher: Send + Sync {
    /// GET `url` and return the body. Non-2xx responses are errors.
    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>>;
}

pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> anyhow::Result<HttpFetcher> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("rust-radar/0.1 (personal hurricane tracker)")
            .build()
            .context("building HTTP client")?;
        Ok(HttpFetcher { client })
    }
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let response = self.client.get(url).send().await?.error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }
}

/// Test double: canned responses keyed by exact URL.
#[derive(Default)]
pub struct FakeFetcher {
    responses: Mutex<HashMap<String, Result<Vec<u8>, String>>>,
    calls: Mutex<Vec<String>>,
}

impl FakeFetcher {
    pub fn new() -> FakeFetcher {
        FakeFetcher::default()
    }

    pub fn ok(&self, url: &str, body: impl Into<Vec<u8>>) {
        self.responses.lock().unwrap().insert(url.to_string(), Ok(body.into()));
    }

    pub fn fail(&self, url: &str, message: &str) {
        self.responses.lock().unwrap().insert(url.to_string(), Err(message.to_string()));
    }

    pub fn calls_to(&self, url: &str) -> usize {
        self.calls.lock().unwrap().iter().filter(|c| *c == url).count()
    }
}

#[async_trait]
impl Fetcher for FakeFetcher {
    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        self.calls.lock().unwrap().push(url.to_string());
        match self.responses.lock().unwrap().get(url) {
            Some(Ok(body)) => Ok(body.clone()),
            Some(Err(message)) => Err(anyhow!("{url}: {message}")),
            None => Err(anyhow!("FakeFetcher has no response for {url}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_fetcher_serves_canned_responses_and_counts_calls() {
        let fake = FakeFetcher::new();
        fake.ok("https://a.test/x", "hello");
        fake.fail("https://a.test/y", "503");
        assert_eq!(fake.get("https://a.test/x").await.unwrap(), b"hello");
        assert!(fake.get("https://a.test/y").await.is_err());
        assert!(fake.get("https://a.test/unknown").await.is_err());
        assert_eq!(fake.calls_to("https://a.test/x"), 1);
        assert_eq!(fake.calls_to("https://a.test/y"), 1);
    }
}
