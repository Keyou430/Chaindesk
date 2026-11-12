use super::{MAX_SUBSCRIPTION_BYTES, failure};
use crate::utils::retry::{RetryError, RetryPolicy, retry};
use anyhow::Result;
use std::{num::NonZeroUsize, time::Duration};

pub(crate) async fn fetch(url: &str, local_proxy_port: Option<u16>) -> Result<String> {
    if url.len() > 8192 {
        return Err(failure(
            "SUBSCRIPTION_FETCH_FAILED",
            "Subscription URLs are limited to 8192 bytes.",
        ));
    }
    let url = reqwest::Url::parse(url)
        .map_err(|_| failure("SUBSCRIPTION_FETCH_FAILED", "Enter a valid HTTPS subscription URL."))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(failure(
            "SUBSCRIPTION_FETCH_FAILED",
            "Subscription downloads require HTTPS.",
        ));
    }
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" || attempt.previous().len() > 3 {
                attempt.error("subscription redirect is not allowed")
            } else {
                attempt.follow()
            }
        }));
    if let Some(port) = local_proxy_port {
        if port == 0 {
            return Err(failure(
                "SUBSCRIPTION_FETCH_FAILED",
                "Choose a valid local HTTP proxy port.",
            ));
        }
        builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?);
    }
    let client = builder.build()?;
    retry(
        RetryPolicy::exponential(NonZeroUsize::MIN.saturating_add(2), Duration::from_secs(1)),
        |_| {
            let client = &client;
            let url = url.clone();
            async move {
                let error = || {
                    failure(
                        "SUBSCRIPTION_FETCH_FAILED",
                        "The subscription could not be downloaded. The previous subscription is retained.",
                    )
                };
                let mut response = client.get(url).send().await.map_err(|e| {
                    if e.is_timeout() || e.is_connect() {
                        RetryError::Retry(error())
                    } else {
                        RetryError::Stop(error())
                    }
                })?;
                if response.status().is_server_error() || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(RetryError::Retry(error()));
                }
                if !response.status().is_success() {
                    return Err(RetryError::Stop(error()));
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|e| {
                    if e.is_timeout() || e.is_body() {
                        RetryError::Retry(error())
                    } else {
                        RetryError::Stop(error())
                    }
                })? {
                    if bytes.len().saturating_add(chunk.len()) > MAX_SUBSCRIPTION_BYTES {
                        return Err(RetryError::Stop(failure(
                            "SUBSCRIPTION_TOO_LARGE",
                            "Subscriptions are limited to 10 MiB after decoding.",
                        )));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                String::from_utf8(bytes).map_err(|_| {
                    RetryError::Stop(failure(
                        "SUBSCRIPTION_UNSUPPORTED",
                        "The subscription must contain UTF-8 YAML.",
                    ))
                })
            }
        },
    )
    .await
}
