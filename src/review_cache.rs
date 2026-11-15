//! Per-review results survive batch regrouping and unrelated repository updates.
use crate::{
    analysis, github,
    model::{Analysis, Finding, Review, Status},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

#[derive(Serialize, Deserialize)]
pub struct Context {
    issues: Vec<github::Issue>,
    snapshots: Vec<github::Snapshot>,
    model: Option<String>,
    validator: String,
    #[serde(default)]
    legacy: bool,
}
#[derive(Serialize, Deserialize)]
struct Record {
    review: Review,
    context: String,
    analysis: Analysis,
    candidates: HashSet<String>,
    discussions_hash: String,
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn validator() -> String {
    crate::model::digest(
        &[
            include_str!("analysis.rs"),
            include_str!("security.rs"),
            include_str!("model.rs"),
        ]
        .concat(),
    )
}
impl Context {
    pub fn new(
        issues: &[github::Issue],
        snapshots: &[github::Snapshot],
        model: Option<&str>,
    ) -> Self {
        Self {
            issues: issues.to_vec(),
            snapshots: snapshots.to_vec(),
            model: model.map(str::to_owned),
            validator: validator(),
            legacy: false,
        }
    }
}

pub fn save(
    root: &Path,
    reviews: &[Review],
    analysis: &Analysis,
    context: &Context,
    candidates: &HashSet<String>,
    discussions: &serde_json::Value,
) -> Result<()> {
    let context_id = digest(context)?;
    crate::atomic_json(
        &root.join("contexts").join(format!("{context_id}.json")),
        context,
    )?;
    for review in reviews {
        let path = root.join(format!("{}.json", crate::model::digest(&review.key())));
        let mut findings: Vec<_> = analysis
            .findings
            .iter()
            .filter(|f| f.review_keys.contains(&review.key()))
            .cloned()
            .collect();
        // A review is complete only if ALL of its concerns are definite. Implicit
        // references to a prior draft need that draft and are not independent records.
        if findings.is_empty()
            || findings.iter().any(|f| {
                f.status == Status::Uncertain
                    || (f.status == Status::Existing && f.related_urls.is_empty())
            })
        {
            if path.exists() {
                fs::remove_file(path)?;
            }
            continue;
        }
        for f in &mut findings {
            f.review_keys = vec![review.key()];
        }
        // Existing/resolved/ignore results depend on their own cited discussions,
        // not on candidates discovered for another review in the same batch.
        // New drafts retain all inspected candidates for duplicate safety.
        let review_candidates = if findings.iter().any(|f| f.status == Status::New) {
            candidates.clone()
        } else {
            findings
                .iter()
                .flat_map(|f| f.related_urls.iter().cloned())
                .collect()
        };
        let review_discussions: Vec<_> = discussions
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| {
                d["url"]
                    .as_str()
                    .is_some_and(|url| review_candidates.contains(url))
            })
            .cloned()
            .collect();
        crate::atomic_json(
            &path,
            &Record {
                review: review.clone(),
                context: context_id.clone(),
                analysis: Analysis { findings },
                candidates: review_candidates,
                discussions_hash: digest(&review_discussions)?,
            },
        )?;
    }
    Ok(())
}

#[derive(Default)]
pub struct Restored {
    pub findings: Vec<Finding>,
    pub keys: HashSet<String>,
    pub rejections: std::collections::BTreeMap<String, usize>,
}

pub fn restore(
    root: &Path,
    reviews: &[Review],
    current: &Context,
    workspace: &Path,
) -> Result<Restored> {
    let mut findings: Vec<Finding> = vec![];
    let mut restored = HashSet::new();
    let mut contexts: HashMap<String, (Context, Vec<github::SourceChange>)> = HashMap::new();
    let mut discussion_hashes = HashMap::new();
    let mut rejections = std::collections::BTreeMap::new();
    macro_rules! reject {
        ($reason:literal) => {{
            *rejections.entry($reason.to_owned()).or_insert(0usize) += 1;
            continue;
        }};
    }
    for review in reviews {
        let path = root.join(format!("{}.json", crate::model::digest(&review.key())));
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => reject!("no_definite_record"),
            Err(e) => return Err(e.into()),
        };
        let record: Record = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(_) => reject!("invalid_record"),
        };
        if record.review != *review
            || record.context.len() != 64
            || !record.context.bytes().all(|b| b.is_ascii_hexdigit())
        {
            reject!("review_or_reference_changed");
        }
        if !contexts.contains_key(&record.context) {
            let path = root
                .join("contexts")
                .join(format!("{}.json", record.context));
            let Ok(bytes) = fs::read(path) else {
                reject!("missing_context")
            };
            let Ok(old) = serde_json::from_slice::<Context>(&bytes) else {
                reject!("invalid_context");
            };
            if digest(&old)? != record.context
                || old.model != current.model
                || old.validator != current.validator
            {
                reject!("model_or_validator_changed");
            }
            let mut changes = vec![];
            for s in &old.snapshots {
                let Some(now) = current
                    .snapshots
                    .iter()
                    .find(|now| now.repository == s.repository && now.branch == s.branch)
                else {
                    continue;
                };
                if let Some(change) = github::compare_snapshot(s, &now.sha)? {
                    changes.push(change);
                }
            }
            if old.snapshots.len() != current.snapshots.len()
                || old.snapshots.iter().any(|s| {
                    !current
                        .snapshots
                        .iter()
                        .any(|n| n.repository == s.repository && n.branch == s.branch)
                })
            {
                reject!("snapshot_inventory_changed");
            }
            contexts.insert(record.context.clone(), (old, changes));
        }
        let (old, changes) = &contexts[&record.context];
        if record
            .analysis
            .findings
            .iter()
            .any(|f| f.status == Status::Uncertain || github::source_affected(f, changes))
        {
            reject!("uncertain_or_source_changed");
        }
        if old.legacy
            && record
                .analysis
                .findings
                .iter()
                .any(|f| f.status == Status::New)
        {
            reject!("legacy_new_draft");
        }
        // A new issue must still be checked against the entire issue inventory.
        // For other statuses only referenced issues/discussions affect the decision.
        if record
            .analysis
            .findings
            .iter()
            .any(|f| f.status == Status::New)
            && !github::same_issues(
                &old.issues,
                &current.issues,
                &record.candidates,
                &HashSet::new(),
            )
        {
            reject!("issue_inventory_changed");
        }
        if !record
            .candidates
            .iter()
            .all(|url| current.issues.iter().any(|i| &i.url == url))
        {
            reject!("missing_candidate");
        }
        let before: Vec<_> = old
            .issues
            .iter()
            .filter(|i| record.candidates.contains(&i.url))
            .cloned()
            .collect();
        let after: Vec<_> = current
            .issues
            .iter()
            .filter(|i| record.candidates.contains(&i.url))
            .cloned()
            .collect();
        if !old.legacy && !github::same_issues(&before, &after, &record.candidates, &HashSet::new())
        {
            reject!("referenced_issue_changed");
        }
        let mut urls: Vec<_> = record.candidates.iter().cloned().collect();
        urls.sort();
        let hash = if let Some(hash) = discussion_hashes.get(&urls) {
            hash
        } else {
            let details = serde_json::to_value(github::reference_details(
                &current.issues,
                &record.candidates,
            )?)?;
            discussion_hashes.entry(urls).or_insert(digest(&details)?)
        };
        if hash != &record.discussions_hash {
            reject!("discussion_changed");
        }
        if analysis::validate(
            &record.analysis,
            std::slice::from_ref(review),
            &current.issues,
            &[],
            workspace,
        )
        .is_err()
        {
            reject!("validation_failed");
        }
        // Split records from the same multi-review finding must be rejoined;
        // incompatible NEW drafts for the same topic are reanalyzed, never overwritten.
        // Definite existing/resolved/ignore findings may describe different reviews
        // with different reasons for the same topic, just as across normal batches.
        let mut compatible = true;
        for f in &record.analysis.findings {
            if let Some(previous) = findings.iter().find(|p| {
                p.repository == f.repository
                    && p.topic == f.topic
                    && p.status == Status::New
                    && f.status == Status::New
            }) {
                let mut p = previous.clone();
                p.review_keys.clear();
                let mut n = f.clone();
                n.review_keys.clear();
                if digest(&p)? != digest(&n)? {
                    compatible = false;
                    break;
                }
            }
        }
        if !compatible {
            reject!("conflicting_draft");
        }
        for f in record.analysis.findings {
            let mut normalized = f.clone();
            normalized.review_keys.clear();
            let fingerprint = digest(&normalized)?;
            let index = findings.iter().position(|previous| {
                let mut p = previous.clone();
                p.review_keys.clear();
                digest(&p).is_ok_and(|value| value == fingerprint)
            });
            if let Some(index) = index {
                findings[index].review_keys.extend(f.review_keys);
                findings[index].review_keys.sort();
                findings[index].review_keys.dedup();
            } else {
                findings.push(f);
            }
        }
        restored.insert(review.key());
    }
    Ok(Restored {
        findings,
        keys: restored,
        rejections,
    })
}

/// Recover independent, definite results from the previous on-disk state BEFORE
/// reviews.json/report.json are replaced. No raw report is accepted as a result:
/// the same finding must also exist in a completed analysis-cache entry.
pub fn import_legacy(state: &Path) -> Result<()> {
    let root = state.join("review-cache");
    if root.join("legacy-imported.json").exists() {
        return Ok(());
    }
    let (Ok(report), Ok(reviews)) = (
        fs::read(state.join("report.json")),
        fs::read(state.join("reviews.json")),
    ) else {
        return Ok(());
    };
    let (Ok(report), Ok(reviews)) = (
        serde_json::from_slice::<serde_json::Value>(&report),
        serde_json::from_slice::<Vec<Review>>(&reviews),
    ) else {
        return Ok(());
    };
    let (Ok(findings), Ok(snapshots)) = (
        serde_json::from_value::<Vec<Finding>>(report["findings"].clone()),
        serde_json::from_value::<Vec<github::Snapshot>>(report["snapshots"].clone()),
    ) else {
        return Ok(());
    };
    if snapshots.len() != crate::model::REPOS.len()
        || crate::model::REPOS
            .iter()
            .any(|repo| snapshots.iter().filter(|s| s.repository == *repo).count() != 1)
    {
        return Ok(());
    }
    let context = Context {
        issues: vec![],
        snapshots,
        model: None,
        validator: validator(),
        legacy: true,
    };
    let context_id = digest(&context)?;
    let uncertain: HashSet<_> = findings
        .iter()
        .filter(|f| f.status == Status::Uncertain)
        .flat_map(|f| f.review_keys.iter().cloned())
        .collect();
    let Ok(entries) = fs::read_dir(state.join("analysis-cache")) else {
        return Ok(());
    };
    #[derive(Deserialize)]
    struct Legacy {
        analysis: Analysis,
        candidates: HashSet<String>,
        discussions_hash: String,
    }
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(saved) = serde_json::from_slice::<Legacy>(&bytes) else {
            continue;
        };
        for review in &reviews {
            if uncertain.contains(&review.key()) {
                continue;
            }
            let path = root.join(format!("{}.json", crate::model::digest(&review.key())));
            if path.exists() {
                continue;
            }
            let expected: Vec<_> = findings
                .iter()
                .filter(|f| f.review_keys.contains(&review.key()))
                .collect();
            let mut matched: Vec<_> = saved
                .analysis
                .findings
                .iter()
                .filter(|f| f.review_keys.contains(&review.key()))
                .cloned()
                .collect();
            if expected.is_empty()
                || matched.len() != expected.len()
                || matched.iter().any(|f| {
                    matches!(f.status, Status::New | Status::Uncertain)
                        || (f.status == Status::Existing && f.related_urls.is_empty())
                })
            {
                continue;
            }
            if !matched.iter().all(|f| {
                expected
                    .iter()
                    .any(|e| serde_json::to_value(f).ok() == serde_json::to_value(e).ok())
            }) {
                continue;
            }
            for f in &mut matched {
                f.review_keys = vec![review.key()];
            }
            crate::atomic_json(
                &root.join("contexts").join(format!("{context_id}.json")),
                &context,
            )?;
            crate::atomic_json(
                &path,
                &Record {
                    review: review.clone(),
                    context: context_id.clone(),
                    analysis: Analysis { findings: matched },
                    candidates: saved.candidates.clone(),
                    discussions_hash: saved.discussions_hash.clone(),
                },
            )?;
        }
    }
    crate::atomic_json(&root.join("legacy-imported.json"), &true)?;
    Ok(())
}
