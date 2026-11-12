use crate::{
    http,
    model::{APP_ID, PACKAGE, Review},
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{collections::HashSet, env, fs, path::Path};

fn required_env(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("set {name}"))
}
fn string<'a>(v: &'a Value, name: &str) -> Result<&'a str> {
    v[name]
        .as_str()
        .with_context(|| format!("missing {name} in store response"))
}
fn optional(v: &Value, name: &str) -> String {
    v[name].as_str().unwrap_or_default().to_owned()
}

fn apple_token() -> Result<String> {
    if let Ok(token) = env::var("APP_STORE_CONNECT_TOKEN") {
        return Ok(token);
    }
    let key = fs::read(required_env("ASC_PRIVATE_KEY_PATH")?)?;
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(required_env("ASC_KEY_ID")?);
    let now = Utc::now().timestamp();
    Ok(encode(
        &header,
        &json!({"iss":required_env("ASC_ISSUER_ID")?, "iat":now, "exp":now+900, "aud":"appstoreconnect-v1"}),
        &EncodingKey::from_ec_pem(&key)?,
    )?)
}

pub fn apple(client: &Client, territory: &str) -> Result<Vec<Review>> {
    ensure!(
        territory.len() == 3 && territory.bytes().all(|b| b.is_ascii_uppercase()),
        "territory must be ISO alpha-3, e.g. JPN"
    );
    let mut url = Some(format!(
        "https://api.appstoreconnect.apple.com/v1/apps/{APP_ID}/customerReviews?limit=200&sort=-createdDate&filter%5Bterritory%5D={territory}&include=response"
    ));
    let mut seen = HashSet::new();
    let mut reviews = vec![];
    while let Some(next) = url {
        ensure!(seen.insert(next.clone()), "Apple repeated a pagination URL");
        let v = http::json(
            client
                .get(http::trusted_next(&next, "api.appstoreconnect.apple.com")?)
                .bearer_auth(apple_token()?),
        )?;
        let rows = v["data"].as_array().context("missing Apple data array")?;
        for row in rows {
            let a = &row["attributes"];
            let reply_id = row
                .pointer("/relationships/response/data/id")
                .and_then(Value::as_str);
            let reply = v["included"]
                .as_array()
                .and_then(|rows| {
                    rows.iter().find(|r| {
                        r["id"].as_str() == reply_id && r["type"] == "customerReviewResponses"
                    })
                })
                .map(|r| optional(&r["attributes"], "responseBody"))
                .unwrap_or_default();
            reviews.push(Review {
                store: "apple".into(),
                id: string(row, "id")?.into(),
                title: optional(a, "title"),
                text: string(a, "body")?.into(),
                rating: a["rating"]
                    .as_u64()
                    .context("missing Apple rating")?
                    .try_into()?,
                updated_at: string(a, "createdDate")?.into(),
                version: String::new(),
                locale: optional(a, "territory"),
                url: format!("https://apps.apple.com/app/id{APP_ID}?see-all=reviews"),
                developer_reply: reply,
            });
        }
        url = v
            .pointer("/links/next")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    Ok(reviews)
}

pub fn google_token(client: &Client) -> Result<String> {
    if let Ok(token) = env::var("GOOGLE_ACCESS_TOKEN") {
        return Ok(token);
    }
    let credentials: Value =
        serde_json::from_slice(&fs::read(required_env("GOOGLE_APPLICATION_CREDENTIALS")?)?)?;
    let now = Utc::now().timestamp();
    let assertion = encode(
        &Header::new(Algorithm::RS256),
        &json!({"iss":string(&credentials,"client_email")?,
        "scope":"https://www.googleapis.com/auth/androidpublisher https://www.googleapis.com/auth/devstorage.read_only",
        "aud":"https://oauth2.googleapis.com/token", "iat":now, "exp":now+3600}),
        &EncodingKey::from_rsa_pem(string(&credentials, "private_key")?.as_bytes())?,
    )?;
    let v = http::json(client.post("https://oauth2.googleapis.com/token").form(&[
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", assertion.as_str()),
    ]))?;
    Ok(string(&v, "access_token")?.into())
}

pub fn parse_google(v: &Value) -> Result<Vec<Review>> {
    let mut out = vec![];
    let rows = match v.get("reviews") {
        None => return Ok(out),
        Some(v) => v.as_array().context("invalid Google reviews")?,
    };
    for r in rows {
        let comments = r["comments"]
            .as_array()
            .context("missing review comments")?;
        let Some(u) = comments.iter().find_map(|c| c.get("userComment")) else {
            continue;
        };
        let seconds = string(&u["lastModified"], "seconds")?.parse::<i64>()?;
        let nanos = u["lastModified"]["nanos"]
            .as_u64()
            .unwrap_or(0)
            .try_into()?;
        let timestamp =
            DateTime::from_timestamp(seconds, nanos).context("invalid review timestamp")?;
        let text = optional(u, "originalText");
        let text = if text.is_empty() {
            string(u, "text")?.to_owned()
        } else {
            text
        };
        let (title, body) = text.split_once('\t').unwrap_or(("", &text));
        let id = string(r, "reviewId")?;
        let mut url = reqwest::Url::parse("https://play.google.com/store/apps/details")?;
        url.query_pairs_mut()
            .append_pair("id", PACKAGE)
            .append_pair("reviewId", id);
        out.push(Review {
            store: "google".into(),
            id: id.into(),
            title: title.into(),
            text: body.into(),
            rating: u["starRating"]
                .as_u64()
                .context("missing Google rating")?
                .try_into()?,
            updated_at: timestamp.to_rfc3339(),
            version: optional(u, "appVersionName"),
            locale: optional(u, "reviewerLanguage"),
            url: url.to_string(),
            developer_reply: comments
                .iter()
                .find_map(|c| c.get("developerComment"))
                .map(|d| optional(d, "text"))
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

pub fn google(client: &Client) -> Result<Vec<Review>> {
    eprintln!(
        "Google Play API coverage: reviews created/updated in the last 7 days; use --play-csv for history."
    );
    let token = google_token(client)?;
    let mut page = String::new();
    let mut seen = HashSet::new();
    let mut out = vec![];
    loop {
        ensure!(
            seen.insert(page.clone()),
            "Google repeated a pagination token"
        );
        let v=http::json(client.get(format!("https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{PACKAGE}/reviews"))
            .bearer_auth(&token).query(&[("maxResults","100"),("token",&page)]))?;
        out.extend(parse_google(&v)?);
        match v
            .pointer("/tokenPagination/nextPageToken")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            Some(s) => page = s.into(),
            None => break,
        }
    }
    Ok(out)
}

pub fn play_csv(path: &Path) -> Result<Vec<Review>> {
    let bytes = fs::read(path)?;
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let encoding = if bytes[0] == 0xff {
            encoding_rs::UTF_16LE
        } else {
            encoding_rs::UTF_16BE
        };
        let (text, _, errors) = encoding.decode(&bytes);
        ensure!(!errors, "invalid UTF-16 CSV");
        text.into_owned()
    } else {
        String::from_utf8(bytes)?
            .trim_start_matches('\u{feff}')
            .to_owned()
    };
    parse_csv(&text)
}

pub fn parse_csv(text: &str) -> Result<Vec<Review>> {
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let headers = reader.headers()?.clone();
    for h in [
        "Package Name",
        "Star Rating",
        "Review Submit Date and Time",
        "Review Text",
    ] {
        ensure!(headers.iter().any(|s| s == h), "missing CSV column: {h}");
    }
    let mut out = vec![];
    for (index, row) in reader.records().enumerate() {
        let row = row?;
        let get = |name: &str| {
            headers
                .iter()
                .position(|h| h == name)
                .and_then(|i| row.get(i))
                .unwrap_or_default()
        };
        ensure!(get("Package Name") == PACKAGE, "CSV contains another app");
        let title = get("Review Title");
        let body = get("Review Text");
        if title.trim().is_empty() && body.trim().is_empty() {
            continue;
        }
        let link = get("Review Link");
        let id = reqwest::Url::parse(link)
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "reviewId")
                    .map(|(_, v)| v.into_owned())
                    .or_else(|| {
                        u.fragment()
                            .and_then(|f| f.strip_prefix("ReviewPlace:id="))
                            .map(str::to_owned)
                    })
            })
            .filter(|id| !id.trim().is_empty())
            .with_context(|| format!("CSV row {} has no stable review ID in Review Link; import refused (no content-derived IDs)", index + 2))?;
        let date = get("Review Last Update Date and Time");
        let date = if date.is_empty() {
            get("Review Submit Date and Time")
        } else {
            date
        };
        out.push(Review {
            store: "google".into(),
            id,
            title: title.into(),
            text: body.into(),
            rating: get("Star Rating").parse()?,
            updated_at: date.into(),
            version: get("App Version Name").into(),
            locale: get("Reviewer Language").into(),
            url: format!("https://play.google.com/store/apps/details?id={PACKAGE}"),
            developer_reply: get("Developer Reply Text").into(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unidentified_csv_rows_are_rejected_and_edits_keep_the_real_id() {
        let csv = format!(
            "Package Name,Star Rating,Review Submit Date and Time,Review Text,Review Link\n{PACKAGE},4,2026-11-01T00:00:00Z,old,https://play.google.com/store/apps/details?reviewId=r1\n"
        );
        let old = parse_csv(&csv).unwrap();
        let new = parse_csv(&csv.replace(",old,", ",edited,")).unwrap();
        assert_eq!(old[0].id, new[0].id);
        let no_link = csv.replace("https://play.google.com/store/apps/details?reviewId=r1", "");
        assert!(
            parse_csv(&no_link)
                .unwrap_err()
                .to_string()
                .contains("CSV row 2")
        );
        assert!(parse_csv(&no_link.replace(",old,", ",edited,")).is_err());
    }
    #[test]
    fn google_uses_user_comment_not_developer_reply() {
        let v = json!({"reviews":[{"reviewId":"r","comments":[{"developerComment":{"text":"修正しました"}},
            {"userComment":{"text":"title\tbody","starRating":5,"lastModified":{"seconds":"1793491200","nanos":1}}}]}]});
        let r = parse_google(&v).unwrap();
        assert_eq!(r[0].text, "body");
        assert_eq!(r[0].rating, 5);
        assert_eq!(r[0].developer_reply, "修正しました");
    }
    #[test]
    fn csv_handles_commas_and_rejects_wrong_app() {
        let csv = format!(
            "Package Name,Star Rating,Review Submit Date and Time,Review Text,Review Link\n{PACKAGE},4,2026-11-01T00:00:00Z,\"a,b\",https://play.google.com/store/apps/details?reviewId=r1\n"
        );
        assert_eq!(parse_csv(&csv).unwrap()[0].text, "a,b");
        assert!(parse_csv(&csv.replace(PACKAGE, "wrong.app")).is_err());
    }
    #[test]
    fn empty_google_page_is_valid_but_malformed_is_not() {
        assert!(parse_google(&json!({})).unwrap().is_empty());
        assert!(parse_google(&json!({"reviews":{}})).is_err());
    }
}
