use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const REPOS: [&str; 2] = ["TrainLCD/MobileApp", "TrainLCD/StationAPI"];
pub const PACKAGE: &str = "me.tinykitten.trainlcd";
pub const APP_ID: &str = "1486355943";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub store: String,
    pub id: String,
    pub title: String,
    pub text: String,
    pub rating: u8,
    pub updated_at: String,
    pub version: String,
    pub locale: String,
    pub url: String,
    pub developer_reply: String,
}
impl Review {
    pub fn key(&self) -> String {
        format!("{}:{}", self.store, self.id)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            ["apple", "google"].contains(&self.store.as_str()),
            "invalid store"
        );
        ensure!(
            !self.id.is_empty() && (1..=5).contains(&self.rating),
            "invalid review"
        );
        chrono::DateTime::parse_from_rfc3339(&self.updated_at)?;
        Ok(())
    }
}

pub fn merge_reviews(reviews: Vec<Review>) -> Result<Vec<Review>> {
    let mut out: BTreeMap<String, Review> = BTreeMap::new();
    for r in reviews {
        r.validate()?;
        if r.text.trim().is_empty() && r.title.trim().is_empty() {
            continue;
        }
        if let Some(old) = out.get(&r.key()) {
            let new_date = chrono::DateTime::parse_from_rfc3339(&r.updated_at)?;
            let old_date = chrono::DateTime::parse_from_rfc3339(&old.updated_at)?;
            if new_date < old_date {
                continue;
            }
        }
        out.insert(r.key(), r);
    }
    Ok(out.into_values().collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    New,
    Existing,
    Resolved,
    Ignore,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub repository: String,
    pub path: String,
    pub line: usize,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub review_keys: Vec<String>,
    pub topic: String,
    pub status: Status,
    pub repository: String,
    pub title: String,
    pub body: String,
    pub reason: String,
    pub evidence: Vec<Evidence>,
    pub related_urls: Vec<String>,
}
impl Finding {
    pub fn marker(&self) -> String {
        let mut keys = self.review_keys.clone();
        keys.sort();
        format!(
            "<!-- trainlcd-review-triage:{} -->",
            digest(&format!(
                "{}\n{}\n{}",
                self.repository,
                keys.join("\n"),
                self.topic
            ))
        )
    }
}
pub fn digest(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub findings: Vec<Finding>,
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn review() -> Review {
        Review {
            store: "google".into(),
            id: "x".into(),
            title: "".into(),
            text: "駅名が違う".into(),
            rating: 4,
            updated_at: "2026-11-01T00:00:00Z".into(),
            version: "1".into(),
            locale: "ja".into(),
            url: "".into(),
            developer_reply: "".into(),
        }
    }
    #[test]
    fn edited_review_wins_regardless_of_input_order() {
        let old = review();
        let mut new = old.clone();
        new.updated_at = "2026-11-02T00:00:00Z".into();
        new.text = "修正済み".into();
        assert_eq!(merge_reviews(vec![new.clone(), old]).unwrap(), vec![new]);
    }
    #[test]
    fn invalid_input_fails_closed() {
        let mut r = review();
        r.rating = 0;
        assert!(merge_reviews(vec![r]).is_err());
    }
}
