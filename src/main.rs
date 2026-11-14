mod analysis;
mod github;
mod http;
mod model;
mod security;
mod stores;

use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use fs2::FileExt;
use model::{Finding, REPOS, Review, Status};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    version,
    about = "TrainLCD store review triage using official APIs and Codex CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Download official API reviews and optionally import historical Play CSVs.
    Fetch {
        #[command(flatten)]
        source: Source,
        #[arg(long, default_value = "reviews.json")]
        out: PathBuf,
    },
    /// Fetch, investigate current code/Issues/PRs, and write report. Default: dry-run.
    Run {
        #[command(flatten)]
        source: Source,
        /// Use normalized reviews JSON instead of calling either store API.
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long, default_value = ".review-triage")]
        state_dir: PathBuf,
        /// Reanalyze unchanged reviews already handled by a successful --apply run.
        #[arg(long)]
        recheck: bool,
        /// Create evidence-backed issues from validated batches; retain failed batches for retry.
        #[arg(long)]
        apply: bool,
        #[arg(long,default_value_t=10,value_parser=clap::value_parser!(u16).range(1..=50))]
        batch_size: u16,
        #[arg(long)]
        model: Option<String>,
        #[arg(long,default_value_t=900,value_parser=clap::value_parser!(u64).range(1..))]
        timeout_seconds: u64,
    },
}
#[derive(Args)]
struct Source {
    /// App Store territory (Google API has no country filter).
    #[arg(long, default_value = "JPN")]
    territory: String,
    /// Repeat for monthly historical Google Play exports (UTF-8 or UTF-16).
    #[arg(long)]
    play_csv: Vec<PathBuf>,
}

fn atomic_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value)?;
    tmp.write_all(b"\n")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn read_reviews(path: &Path) -> Result<Vec<Review>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn fetch(source: &Source) -> Result<Vec<Review>> {
    let client = http::client()?;
    let mut reviews = stores::apple(&client, &source.territory)
        .context("App Store fetch failed; no issues published")?;
    reviews
        .extend(stores::google(&client).context("Google Play fetch failed; no issues published")?);
    for csv in &source.play_csv {
        reviews.extend(stores::play_csv(csv)?);
    }
    model::merge_reviews(reviews)
}
fn all_issues(comments: bool) -> Result<Vec<github::Issue>> {
    let mut out = vec![];
    for repo in REPOS {
        out.extend(github::list_issues(repo, comments)?);
    }
    Ok(out)
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Action::Fetch { source, out } => {
            let reviews = fetch(&source)?;
            atomic_json(&out, &reviews)?;
            eprintln!("Saved {} reviews to {}", reviews.len(), out.display());
        }
        Action::Run {
            source,
            input,
            state_dir,
            apply,
            batch_size,
            model,
            timeout_seconds,
            recheck,
        } => {
            fs::create_dir_all(&state_dir)?;
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(state_dir.join("run.lock"))?;
            lock.try_lock_exclusive()
                .context("another run is using this state directory")?;
            let state_dir = state_dir.canonicalize()?;
            let pending = state_dir.join("pending-issue.json");
            let receipts_path = state_dir.join("created-issues.json");
            let receipts = github::load_receipts(&receipts_path)?;
            ensure!(
                !pending.exists(),
                "pending-issue.json exists: a previous POST may have succeeded. Inspect GitHub and reconcile it before removing the journal."
            );
            let mut reviews = if let Some(path) = input {
                let mut rows = read_reviews(&path)?;
                for p in &source.play_csv {
                    rows.extend(stores::play_csv(p)?);
                }
                rows
            } else {
                fetch(&source)?
            };
            let cache = state_dir.join("reviews.json");
            if cache.exists() {
                let mut old = read_reviews(&cache)?;
                old.extend(reviews);
                reviews = old;
            }
            let reviews = model::merge_reviews(reviews)?;
            atomic_json(&cache, &reviews)?;
            let total_reviews = reviews.len();
            let processed_path = state_dir.join("processed.json");
            let mut processed: std::collections::BTreeMap<String, String> =
                if processed_path.exists() {
                    serde_json::from_slice(&fs::read(&processed_path)?)?
                } else {
                    Default::default()
                };
            let reviews: Vec<_> = reviews
                .into_iter()
                .filter(|r| recheck || processed.get(&r.key()) != Some(&r.fingerprint()))
                .collect();
            let report_path = state_dir.join("report.json");
            if reviews.is_empty() {
                atomic_json(
                    &report_path,
                    &json!({"created_at":chrono::Utc::now(),"findings":[],"created_issues":[],"coverage":"Apple selected territory; Google last 7 days plus supplied CSV/cache"}),
                )?;
                eprintln!(
                    "No new or edited text reviews to analyze (use --recheck to reanalyze processed reviews)."
                );
                return Ok(());
            }
            eprintln!(
                "Preparing source snapshots and all open/closed Issues/PRs for {} reviews…",
                reviews.len()
            );
            let workspace = tempfile::tempdir()?;
            let mut snapshots = vec![];
            for repo in REPOS {
                snapshots.push(github::snapshot(repo, workspace.path())?);
            }
            let issues = all_issues(false)?;
            atomic_json(&workspace.path().join("github-context.json"), &issues)?;
            atomic_json(&workspace.path().join("snapshots.json"), &snapshots)?;
            let mut findings: Vec<Finding> = vec![];
            let mut failed_keys = std::collections::HashSet::new();
            let mut batch_failures = vec![];
            let mut first_failure = None;
            for (index, batch) in reviews.chunks(batch_size.into()).enumerate() {
                eprintln!("Analyzing batch {}…", index + 1);
                let result = (|| -> Result<model::Analysis> {
                    let mut result = analysis::analyze(
                        workspace.path(),
                        batch,
                        &issues,
                        &findings,
                        model.as_deref(),
                        timeout_seconds,
                    )?;
                    // Fetch full discussions/PR metadata only for actual matches, not thousands
                    // of unrelated historical PR threads.
                    let refs: std::collections::HashSet<_> = result
                        .findings
                        .iter()
                        .flat_map(|f| f.related_urls.iter().cloned())
                        .collect();
                    if !refs.is_empty() {
                        let details = github::reference_details(&issues, &refs)?;
                        atomic_json(&workspace.path().join("matched-discussions.json"), &details)?;
                        result = analysis::analyze(
                            workspace.path(),
                            batch,
                            &issues,
                            &findings,
                            model.as_deref(),
                            timeout_seconds,
                        )?;
                        fs::remove_file(workspace.path().join("matched-discussions.json"))?;
                    }
                    Ok(result)
                })();
                // Never let a rejected result enter the prior findings or publication set.
                // Clear discussion context even when the second analysis failed.
                let discussions = workspace.path().join("matched-discussions.json");
                if discussions.exists() {
                    fs::remove_file(discussions)?;
                }
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        let keys: Vec<_> = batch.iter().map(Review::key).collect();
                        failed_keys.extend(keys.iter().cloned());
                        batch_failures.push(json!({"batch":index + 1,"review_keys":keys}));
                        atomic_json(&state_dir.join("failed-batches.json"), &batch_failures)?;
                        eprintln!(
                            "Batch {} failed; retaining reviews for retry: {error:#}",
                            index + 1
                        );
                        if first_failure.is_none() {
                            first_failure = Some(error);
                        }
                        continue;
                    }
                };
                for f in result.findings {
                    if f.status == Status::Existing && f.related_urls.is_empty() {
                        let previous = findings
                            .iter_mut()
                            .find(|p| {
                                p.status == Status::New
                                    && p.topic == f.topic
                                    && p.repository == f.repository
                            })
                            .context("missing prior finding")?;
                        previous.review_keys.extend(f.review_keys);
                        previous.review_keys.sort();
                        previous.review_keys.dedup();
                    } else {
                        findings.push(f);
                    }
                }
            }
            if apply && !failed_keys.is_empty() {
                // A failed recheck invalidates even a previously processed fingerprint.
                for key in &failed_keys {
                    processed.remove(key);
                }
                if processed_path.exists() || !processed.is_empty() {
                    atomic_json(&processed_path, &processed)?;
                }
            }
            atomic_json(&state_dir.join("failed-batches.json"), &batch_failures)?;
            if findings.is_empty()
                && let Some(error) = first_failure
            {
                return Err(error);
            }
            let drafts:Vec<_>=findings.iter().filter(|f|f.status==Status::New).map(|f|Ok(json!({"repository":f.repository,"title":f.title,"body":analysis::issue_body(f,&reviews,&snapshots)?}))).collect::<Result<_>>()?;
            let mut report = json!({"created_at":chrono::Utc::now(),"coverage":"Apple selected territory; Google last 7 days plus supplied CSV/cache. Google API does not expose country.",
                "snapshots":snapshots,"review_count":reviews.len(),"cached_review_count":total_reviews,"failed_review_count":failed_keys.len(),"batch_failures":batch_failures,"findings":findings,"drafts":drafts,"created_issues":[],"apply":apply});
            atomic_json(&report_path, &report)?;
            if apply {
                let matched: std::collections::HashSet<_> = findings
                    .iter()
                    .flat_map(|f| f.related_urls.iter().cloned())
                    .collect();
                let mut created_urls = std::collections::HashSet::new();
                github::check_heads(&snapshots)?;
                let current = all_issues(false)?;
                ensure!(
                    github::same_issues(&issues, &current, &matched, &created_urls),
                    "Issues/PRs changed during analysis; rerun to check for duplicates"
                );
                let mut expected = current;
                let mut created = vec![];
                for f in findings.iter().filter(|f| f.status == Status::New) {
                    // Recheck immediately before each write, including after our own previous issue.
                    let fresh = all_issues(false)?;
                    ensure!(
                        github::same_issues(&expected, &fresh, &matched, &created_urls),
                        "Issues/PRs changed; report contains partial results. Rerun."
                    );
                    if receipts.contains_key(&f.marker())
                        || expected
                            .iter()
                            .chain(&fresh)
                            .any(|i| i.body.contains(&f.marker()))
                    {
                        continue;
                    }
                    github::check_heads(&snapshots)?;
                    let issue = github::create_issue(
                        &f.repository,
                        &f.title,
                        &analysis::issue_body(f, &reviews, &snapshots)?,
                        &pending,
                        &receipts_path,
                        &f.marker(),
                    )?;
                    let url = issue.url.clone();
                    eprintln!("Created {url}");
                    created.push(url.clone());
                    report["created_issues"] = json!(created);
                    atomic_json(&report_path, &report)?;
                    created_urls.insert(url);
                    expected.push(issue);
                }
                // Commit only validated reviews after their publication succeeds.
                // Failed/uncertain reviews remain eligible; successful batches are not repeated.
                for review in &reviews {
                    let key = review.key();
                    if failed_keys.contains(&key)
                        || findings
                            .iter()
                            .any(|f| f.status == Status::Uncertain && f.review_keys.contains(&key))
                    {
                        processed.remove(&key);
                    } else {
                        processed.insert(key, review.fingerprint());
                    }
                }
                atomic_json(&processed_path, &processed)?;
            }
            eprintln!(
                "{} actionable drafts; {} total findings. Report: {}{}",
                drafts.len(),
                findings.len(),
                report_path.display(),
                if apply {
                    ""
                } else {
                    " (dry-run; use --apply to create issues)"
                }
            );
            ensure!(
                batch_failures.is_empty(),
                "some batches failed; validated reviews saved, failed reviews remain pending"
            );
        }
    }
    Ok(())
}
