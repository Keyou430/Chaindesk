use crate::config::chain::{ProbeObservation, failure};
use anyhow::Result;
use std::{
    net::Ipv4Addr,
    time::{Duration, Instant},
};

pub(crate) fn client(port: u16) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

pub(crate) async fn probe(port: u16) -> Result<ProbeObservation> {
    let started = Instant::now();
    let client = client(port)?;
    let mut reachable = false;
    for url in [
        "https://www.gstatic.com/generate_204",
        "https://cp.cloudflare.com/generate_204",
    ] {
        if client
            .get(url)
            .send()
            .await
            .is_ok_and(|response| response.status() == reqwest::StatusCode::NO_CONTENT)
        {
            reachable = true;
            break;
        }
    }
    if !reachable {
        return Err(failure(
            "CHAIN_PROBE_FAILED",
            "The complete chain could not complete a verified HTTPS probe. Check the front, exit credentials and network.",
        ));
    }
    for (url, json) in [
        ("https://api.ipify.org?format=json", true),
        ("https://www.cloudflare.com/cdn-cgi/trace", false),
    ] {
        if let Ok(ipv4) = observe(&client, url, json).await {
            return Ok(ProbeObservation {
                ipv4,
                checked_at: chrono::Utc::now().to_rfc3339(),
                service: url.into(),
                elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            });
        }
    }
    Err(failure(
        "EXIT_IP_UNAVAILABLE",
        "HTTPS works, but the IPv4 observation services did not return a valid result. No system proxy was acquired.",
    ))
}

async fn observe(client: &reqwest::Client, url: &str, json: bool) -> Result<String> {
    let mut response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("IP observation failed"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 4096 {
            return Err(anyhow::anyhow!("IP observation too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = if json {
        serde_json::from_slice::<serde_json::Value>(&bytes)?
            .get("ip")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    } else {
        std::str::from_utf8(&bytes)?
            .lines()
            .find_map(|line| line.strip_prefix("ip=").map(str::to_owned))
    }
    .ok_or_else(|| anyhow::anyhow!("missing IPv4 observation"))?;
    Ok(value.parse::<Ipv4Addr>()?.to_string())
}
