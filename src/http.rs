use anyhow::{Result, bail, ensure};
use reqwest::{
    StatusCode,
    blocking::{Client, RequestBuilder},
    redirect::Policy,
};
use serde_json::Value;
use std::{thread, time::Duration};

pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(Policy::none())
        .user_agent("TrainLCD-ReviewTriage/0.1")
        .build()?)
}
// GETs and OAuth exchanges may be retried; issue creation deliberately is not.
pub fn json(request: RequestBuilder) -> Result<Value> {
    for attempt in 0..4 {
        let response = request
            .try_clone()
            .ok_or_else(|| anyhow::anyhow!("request not clonable"))?
            .send()?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            if attempt == 3 {
                bail!("HTTP {status}; retry later");
            }
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2u64.pow(attempt));
            ensure!(
                delay <= 60,
                "server requested a long retry delay; retry later"
            );
            thread::sleep(Duration::from_secs(delay));
            continue;
        }
        ensure!(
            status.is_success(),
            "HTTP {status} (check credentials and permissions)"
        );
        return Ok(response.json()?);
    }
    unreachable!()
}

pub fn trusted_next(url: &str, host: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url)?;
    ensure!(
        u.scheme() == "https"
            && u.host_str() == Some(host)
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443),
        "untrusted pagination URL"
    );
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pagination_cannot_leak_bearer_token() {
        for s in [
            "https://evil.test/a",
            "http://api.appstoreconnect.apple.com/a",
            "https://api.appstoreconnect.apple.com@evil.test/a",
            "https://api.appstoreconnect.apple.com:8443/a",
        ] {
            assert!(trusted_next(s, "api.appstoreconnect.apple.com").is_err());
        }
    }
}
