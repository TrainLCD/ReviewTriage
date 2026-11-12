use crate::model::REPOS;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub fn command(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("start {program}"))?;
    ensure!(
        out.status.success(),
        "{program} failed (exit {}); check installation/authentication",
        out.status
    );
    Ok(String::from_utf8(out.stdout)?)
}

pub fn api(endpoint: &str) -> Result<Value> {
    Ok(serde_json::from_str(&command("gh", &["api", endpoint])?)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub repository: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub body: String,
    pub state: String,
    pub state_reason: String,
    pub updated_at: String,
    pub is_pr: bool,
    pub comments: Vec<Value>,
}

pub fn list_issues(repo: &str, with_comments: bool) -> Result<Vec<Issue>> {
    ensure!(REPOS.contains(&repo), "invalid repository");
    let output = command(
        "gh",
        &[
            "api",
            "--paginate",
            "--slurp",
            &format!("repos/{repo}/issues?state=all&per_page=100&sort=created&direction=asc"),
        ],
    )?;
    let pages: Vec<Vec<Value>> = serde_json::from_str(&output)?;
    let mut out = vec![];
    for item in pages.into_iter().flatten() {
        let number = item["number"].as_u64().context("invalid issue number")?;
        let comments = if with_comments && item["comments"].as_u64().unwrap_or(0) > 0 {
            let data = command(
                "gh",
                &[
                    "api",
                    "--paginate",
                    "--slurp",
                    &format!("repos/{repo}/issues/{number}/comments?per_page=100"),
                ],
            )?;
            serde_json::from_str::<Vec<Vec<Value>>>(&data)?
                .into_iter()
                .flatten()
                .map(|c| json!({"url":c["html_url"],"body":c["body"],"created_at":c["created_at"]}))
                .collect()
        } else {
            vec![]
        };
        out.push(Issue {
            repository: repo.into(),
            number,
            url: item["html_url"]
                .as_str()
                .context("missing issue URL")?
                .into(),
            title: item["title"]
                .as_str()
                .context("missing issue title")?
                .into(),
            body: item["body"].as_str().unwrap_or_default().into(),
            state: item["state"]
                .as_str()
                .context("missing issue state")?
                .into(),
            state_reason: item["state_reason"].as_str().unwrap_or_default().into(),
            updated_at: item["updated_at"]
                .as_str()
                .context("missing issue timestamp")?
                .into(),
            is_pr: item.get("pull_request").is_some(),
            comments,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub repository: String,
    pub branch: String,
    pub sha: String,
}

pub fn snapshot(repo: &str, workspace: &Path) -> Result<Snapshot> {
    let meta = api(&format!("repos/{repo}"))?;
    let branch = meta["default_branch"]
        .as_str()
        .context("missing default branch")?
        .to_owned();
    let name = repo.split('/').next_back().context("invalid repo")?;
    let dest = workspace.join(name);
    // Fresh disposable clone: never reset or modify the user's own checkouts.
    command(
        "gh",
        &[
            "repo",
            "clone",
            repo,
            dest.to_str().context("non UTF-8 workspace")?,
            "--",
            "--depth=100",
            "--single-branch",
            "--branch",
            &branch,
        ],
    )?;
    let sha = command("git", &["-C", dest.to_str().unwrap(), "rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    Ok(Snapshot {
        repository: repo.into(),
        branch,
        sha,
    })
}

pub fn same_issues(
    a: &[Issue],
    b: &[Issue],
    matched: &std::collections::HashSet<String>,
    created: &std::collections::HashSet<String>,
) -> bool {
    let before: std::collections::HashMap<_, _> = a.iter().map(|i| (&i.url, i)).collect();
    let after: std::collections::HashMap<_, _> = b.iter().map(|i| (&i.url, i)).collect();
    if before.len() != a.len() || after.len() != b.len() {
        return false;
    }
    if after.keys().any(|url| !before.contains_key(url)) {
        return false;
    }
    before.iter().all(|(url, a)| match after.get(url) {
        None => created.contains(*url), // POST result is authoritative while the list catches up.
        Some(b) => {
            a.repository == b.repository
                && a.number == b.number
                && a.title == b.title
                && a.body == b.body
                && a.is_pr == b.is_pr
                && (!matched.contains(*url)
                    || (a.state == b.state && a.state_reason == b.state_reason))
        }
    })
}

pub fn reference_details(
    issues: &[Issue],
    urls: &std::collections::HashSet<String>,
) -> Result<Vec<Value>> {
    let mut out = vec![];
    for i in issues.iter().filter(|i| urls.contains(&i.url)) {
        let endpoint = format!(
            "repos/{}/issues/{}/comments?per_page=100",
            i.repository, i.number
        );
        let comments: Value = serde_json::from_str(&command(
            "gh",
            &["api", "--paginate", "--slurp", &endpoint],
        )?)?;
        let resource = if i.is_pr { "pulls" } else { "issues" };
        let detail = api(&format!("repos/{}/{resource}/{}", i.repository, i.number))?;
        out.push(json!({"url":i.url,"detail":detail,"comments":comments}));
    }
    Ok(out)
}

pub fn check_heads(snapshots: &[Snapshot]) -> Result<()> {
    for s in snapshots {
        let branch_encoded = reqwest::Url::parse("https://api.github.com/")?
            .join(&format!("repos/{}/commits/", s.repository))?;
        let mut url = branch_encoded;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid URL"))?
            .pop_if_empty()
            .push(&s.branch);
        let v = api(url.as_str())?;
        ensure!(
            v["sha"].as_str() == Some(&s.sha),
            "{} changed during analysis; rerun before publishing",
            s.repository
        );
    }
    Ok(())
}

pub fn create_issue(repo: &str, title: &str, body: &str, journal: &Path) -> Result<Issue> {
    ensure!(REPOS.contains(&repo), "invalid issue destination");
    crate::security::check_output(title)?;
    crate::security::check_output(body)?;
    // Persist intent BEFORE POST. An ambiguous response must never trigger a blind retry.
    let mut journal_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(journal)?;
    journal_file.write_all(&serde_json::to_vec_pretty(
        &json!({"repository":repo,"title":title,"body":body}),
    )?)?;
    journal_file.sync_all()?;
    let mut child = Command::new("gh")
        .args([
            "api",
            "--method",
            "POST",
            &format!("repos/{repo}/issues"),
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    child
        .stdin
        .take()
        .context("missing stdin")?
        .write_all(&serde_json::to_vec(&json!({"title":title,"body":body}))?)?;
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "Issue creation failed or is ambiguous. Pending journal retained; inspect GitHub before retrying."
    );
    let v: Value = serde_json::from_slice(&output.stdout)?;
    let url = v["html_url"]
        .as_str()
        .context("missing created issue URL; inspect pending journal")?
        .to_owned();
    let issue = Issue {
        repository: repo.into(),
        number: v["number"]
            .as_u64()
            .context("missing issue number; inspect pending journal")?,
        url,
        title: title.into(),
        body: body.into(),
        state: v["state"].as_str().context("missing issue state")?.into(),
        state_reason: v["state_reason"].as_str().unwrap_or_default().into(),
        updated_at: v["updated_at"].as_str().unwrap_or_default().into(),
        is_pr: false,
        comments: vec![],
    };
    fs::remove_file(journal)?;
    Ok(issue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    fn issue() -> Issue {
        Issue {
            repository: REPOS[0].into(),
            number: 1,
            url: "https://github.com/TrainLCD/MobileApp/issues/1".into(),
            title: "title".into(),
            body: "body".into(),
            state: "open".into(),
            state_reason: "".into(),
            updated_at: "old".into(),
            is_pr: false,
            comments: vec![],
        }
    }
    #[test]
    fn unrelated_activity_does_not_invalidate_analysis() {
        let before = issue();
        let mut after = before.clone();
        after.updated_at = "new".into();
        after.state = "closed".into();
        assert!(same_issues(
            std::slice::from_ref(&before),
            std::slice::from_ref(&after),
            &HashSet::new(),
            &HashSet::new()
        ));
        assert!(!same_issues(
            std::slice::from_ref(&before),
            std::slice::from_ref(&after),
            &HashSet::from([before.url.clone()]),
            &HashSet::new()
        ));
        after.body = "changed".into();
        assert!(!same_issues(
            &[before],
            &[after],
            &HashSet::new(),
            &HashSet::new()
        ));
    }
    #[test]
    fn delayed_own_issue_visibility_is_allowed_but_external_additions_are_not() {
        let own = issue();
        let created = HashSet::from([own.url.clone()]);
        assert!(same_issues(
            std::slice::from_ref(&own),
            &[],
            &HashSet::new(),
            &created
        ));
        assert!(!same_issues(
            std::slice::from_ref(&own),
            &[],
            &HashSet::new(),
            &HashSet::new()
        ));
        assert!(same_issues(
            std::slice::from_ref(&own),
            std::slice::from_ref(&own),
            &HashSet::new(),
            &created
        ));
        assert!(!same_issues(&[], &[own], &HashSet::new(), &created));
    }
}
