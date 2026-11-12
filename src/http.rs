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
            let delay = retry_delay(
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok()),
                chrono::Utc::now(),
                2u64.pow(attempt),
            )?;
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

fn retry_delay(
    value: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
    fallback: u64,
) -> Result<u64> {
    let delay = match value {
        Some(s) => match s.trim().parse::<u64>() {
            Ok(n) => n,
            Err(_) => chrono::DateTime::parse_from_rfc2822(s)
                .map(|date| {
                    let ms = (date.with_timezone(&chrono::Utc) - now).num_milliseconds();
                    if ms <= 0 {
                        0
                    } else {
                        (ms as u64).div_ceil(1000)
                    }
                })
                .unwrap_or(fallback),
        },
        None => fallback,
    };
    ensure!(
        delay <= 60,
        "server requested a long retry delay; retry later"
    );
    Ok(delay)
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
    fn retry_after_seconds_dates_and_limits() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-12T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(retry_delay(Some("12"), now, 1).unwrap(), 12);
        assert_eq!(
            retry_delay(Some("Thu, 12 Nov 2026 12:00:30 GMT"), now, 1).unwrap(),
            30
        );
        assert_eq!(
            retry_delay(Some("Thu, 12 Nov 2026 11:59:00 GMT"), now, 1).unwrap(),
            0
        );
        assert!(retry_delay(Some("Thu, 12 Nov 2026 12:01:01 GMT"), now, 1).is_err());
        assert!(retry_delay(Some("61"), now, 1).is_err());
        assert_eq!(retry_delay(Some("invalid"), now, 4).unwrap(), 4);
    }
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
