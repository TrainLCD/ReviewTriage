use crate::{
    analysis, github,
    model::{Analysis, Finding, Review},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path};

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    analysis: Analysis,
    candidates: HashSet<String>,
    discussions_hash: String,
}

pub fn key(
    reviews: &[Review],
    issues: &[github::Issue],
    snapshots: &[github::Snapshot],
    prior: &[Finding],
    model: Option<&str>,
) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&(
        reviews, issues, snapshots, prior, model,
    ))?);
    // Changes to interpretation/validation invalidate older checkpoints automatically.
    for source in [
        include_str!("analysis.rs"),
        include_str!("main.rs"),
        include_str!("github.rs"),
        include_str!("security.rs"),
        include_str!("model.rs"),
        include_str!("checkpoint.rs"),
    ] {
        hash.update(source);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn load(
    path: &Path,
    reviews: &[Review],
    issues: &[github::Issue],
    prior: &[Finding],
    workspace: &Path,
) -> Result<Option<Analysis>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let checkpoint: Checkpoint = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if checkpoint
        .analysis
        .findings
        .iter()
        .any(|f| f.status == crate::model::Status::Uncertain)
    {
        return Ok(None);
    }
    if !checkpoint
        .candidates
        .iter()
        .all(|url| issues.iter().any(|i| &i.url == url))
    {
        return Ok(None);
    }
    let current = serde_json::to_value(github::reference_details(issues, &checkpoint.candidates)?)?;
    if format!("{:x}", Sha256::digest(serde_json::to_vec(&current)?)) != checkpoint.discussions_hash
        || analysis::validate(&checkpoint.analysis, reviews, issues, prior, workspace).is_err()
    {
        return Ok(None);
    }
    Ok(Some(checkpoint.analysis))
}

pub fn save(
    path: &Path,
    analysis: &Analysis,
    candidates: &HashSet<String>,
    discussions: &serde_json::Value,
) -> Result<()> {
    crate::atomic_json(
        path,
        &serde_json::json!({"analysis":analysis,"candidates":candidates,"discussions_hash":format!("{:x}",Sha256::digest(serde_json::to_vec(discussions)?))}),
    )
}

// Incomplete batches are resumable work, never publication-ready cache entries.
#[derive(Serialize, Deserialize)]
pub struct InFlight {
    pub analysis: Analysis,
    pub candidates: HashSet<String>,
    pub inspected: HashSet<String>,
    pub discussions: serde_json::Value,
    pub passes: usize,
    pub repairs_used: usize,
}

pub fn load_in_flight(
    path: &Path,
    reviews: &[Review],
    issues: &[github::Issue],
    prior: &[Finding],
    workspace: &Path,
) -> Result<Option<InFlight>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let saved: InFlight = match serde_json::from_slice(&bytes) {
        Ok(saved) => saved,
        Err(_) => return Ok(None),
    };
    if !saved.inspected.is_subset(&saved.candidates)
        || !saved
            .candidates
            .iter()
            .all(|url| issues.iter().any(|i| &i.url == url))
        || analysis::validate(&saved.analysis, reviews, issues, prior, workspace).is_err()
    {
        return Ok(None);
    }
    let current = serde_json::to_value(github::reference_details(issues, &saved.inspected)?)?;
    if current != saved.discussions {
        return Ok(None);
    }
    Ok(Some(saved))
}
