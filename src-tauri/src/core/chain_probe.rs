use crate::config::chain::{ChainFailure, IpObservation, ProbeObservation, failure};
use anyhow::Result;
use clash_verge_logging::{Type, logging};
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

pub(crate) async fn probe(port: u16) -> Result<Option<ProbeObservation>> {
    let started = Instant::now();
    let client = client(port)?;
    https(port).await?;
    let (ipify, cloudflare) = tokio::join!(
        observe(&client, "https://api.ipify.org?format=json", true),
        observe(&client, "https://www.cloudflare.com/cdn-cgi/trace", false),
    );
    let observations: [_; 2] = (ipify, cloudflare).into();
    for (service, result) in ["ipify", "cloudflare"].into_iter().zip(&observations) {
        if let Err(error) = result {
            let code = error
                .downcast_ref::<ChainFailure>()
                .map(|failure| failure.code)
                .unwrap_or_else(|| {
                    error
                        .downcast_ref::<reqwest::Error>()
                        .map_or("INVALID_RESPONSE", request_failure_code)
                });
            logging!(
                warn,
                Type::Core,
                "{}",
                serde_json::json!({"event":"IP_OBSERVATION_FAILED", "service":service, "code":code})
            );
        }
    }
    let sources = observations.into_iter().filter_map(Result::ok).collect::<Vec<_>>();
    let Some(first) = sources.first() else {
        return Ok(None);
    };
    Ok(Some(ProbeObservation {
        ipv4: first.ipv4.clone(),
        checked_at: first.checked_at.clone(),
        service: first.service.clone(),
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        consistent: sources.iter().all(|source| source.ipv4 == first.ipv4),
        sources,
    }))
}

pub(crate) fn verify(observation: Option<&ProbeObservation>, expected: Option<&str>) -> Result<()> {
    if expected.is_none() {
        return Ok(());
    }
    let observation = observation.ok_or_else(|| {
        failure(
            "EXIT_IP_UNAVAILABLE",
            "The HTTPS chain works, but the IPv4 services are unavailable. The confirmed baseline cannot be checked.",
        )
    })?;
    if !observation.consistent {
        return Err(failure(
            "EXIT_IP_INCONSISTENT",
            "The IPv4 services disagree. Review both observations and retry before confirming a baseline.",
        ));
    }
    if expected.is_some_and(|expected| expected != observation.ipv4) {
        return Err(failure(
            "EXIT_IP_CHANGED",
            "The observed exit IPv4 differs from the confirmed baseline. Review the new observation before changing the baseline.",
        ));
    }
    Ok(())
}

pub(crate) async fn https(port: u16) -> Result<()> {
    let client = client(port)?;
    let request = async |service, url| match client.get(url).send().await {
        Ok(response) if response.status() == reqwest::StatusCode::NO_CONTENT => true,
        Ok(response) => {
            logging!(
                warn,
                Type::Core,
                "{}",
                serde_json::json!({"event":"HTTPS_PROBE_FAILED", "service":service, "code":"UNEXPECTED_HTTP_STATUS", "status":response.status().as_u16()})
            );
            false
        }
        Err(error) => {
            logging!(
                warn,
                Type::Core,
                "{}",
                serde_json::json!({"event":"HTTPS_PROBE_FAILED", "service":service, "code":request_failure_code(&error)})
            );
            false
        }
    };
    let result = futures::future::select(
        Box::pin(request("gstatic", "https://www.gstatic.com/generate_204")),
        Box::pin(request("cloudflare", "https://cp.cloudflare.com/generate_204")),
    )
    .await;
    let reachable = match result {
        futures::future::Either::Left((first, pending)) => first || pending.await,
        futures::future::Either::Right((first, pending)) => first || pending.await,
    };
    if !reachable {
        return Err(failure(
            "CHAIN_PROBE_FAILED",
            "The complete chain could not complete a verified HTTPS probe. Check the front, exit credentials and network.",
        ));
    }
    Ok(())
}

fn request_failure_code(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        return "TIMEOUT";
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(cause) = source {
        if let Some(tls) = cause.downcast_ref::<rustls::Error>() {
            return if matches!(tls, rustls::Error::InvalidCertificate(_)) {
                "TLS_CERTIFICATE_INVALID"
            } else {
                "TLS_PROTOCOL_FAILED"
            };
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::ConnectionRefused => return "CONNECTION_REFUSED",
                std::io::ErrorKind::ConnectionReset => return "CONNECTION_RESET",
                std::io::ErrorKind::ConnectionAborted => return "CONNECTION_ABORTED",
                std::io::ErrorKind::UnexpectedEof => return "UNEXPECTED_EOF",
                _ => {}
            }
        }
        source = cause.source();
    }
    if error.is_connect() {
        "CONNECT_OR_TLS_FAILED"
    } else {
        "RESPONSE_FAILED"
    }
}

async fn observe(client: &reqwest::Client, url: &str, json: bool) -> Result<IpObservation> {
    let mut response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(failure(
            "IP_SERVICE_HTTP_FAILED",
            format!("IPv4 query service returned HTTP {}.", response.status().as_u16()),
        ));
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
    Ok(IpObservation {
        ipv4: value.parse::<Ipv4Addr>()?.to_string(),
        checked_at: chrono::Utc::now().to_rfc3339(),
        service: url.split('?').next().unwrap_or(url).into(),
    })
}

#[cfg(test)]
mod tests {
    use super::{ProbeObservation, verify};

    #[test]
    fn only_an_explicit_baseline_blocks_a_changed_or_inconsistent_observation() {
        let observation = ProbeObservation {
            ipv4: "203.0.113.1".into(),
            consistent: true,
            ..ProbeObservation::default()
        };
        assert!(verify(Some(&observation), None).is_ok());
        assert!(verify(Some(&observation), Some("203.0.113.1")).is_ok());
        assert!(verify(Some(&observation), Some("203.0.113.2")).is_err());
        let inconsistent = ProbeObservation {
            consistent: false,
            ..observation
        };
        assert!(verify(Some(&inconsistent), None).is_ok());
        assert!(verify(Some(&inconsistent), Some("203.0.113.1")).is_err());
        assert!(verify(None, None).is_ok());
        assert!(verify(None, Some("203.0.113.1")).is_err());
    }
}
