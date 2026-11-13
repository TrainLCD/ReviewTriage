#![cfg(unix)]
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

struct Harness {
    dir: tempfile::TempDir,
}
impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("bin")).unwrap();
        let script = r#"#!/usr/bin/env python3
import sys,os,json,pathlib
root=pathlib.Path(os.environ['MOCK_ROOT']); args=sys.argv[1:]; name=pathlib.Path(sys.argv[0]).name
issues_file=root/'issues.json'
issues=json.loads(issues_file.read_text()) if issues_file.exists() else []
mode=os.environ.get('MOCK_MODE','normal')
if name=='git': print('a'*40)
elif name=='gh':
 if args[:2]==['repo','clone']:
  d=pathlib.Path(args[3]); d.mkdir(); (d/'source.rs').write_text('fn main() {}\n')
 elif '--method' in args:
  payload=json.load(sys.stdin); repo=args[args.index('POST')+1].split('/issues')[0][6:]
  i={'number':len(issues)+1,'html_url':f'https://github.com/{repo}/issues/{len(issues)+1}','title':payload['title'],'body':payload['body'],'state':'open','updated_at':'2026-11-12T00:00:00Z','comments':0}
  issues.append(i);issues_file.write_text(json.dumps(issues))
  if mode=='ambiguous': sys.exit(1)
  if mode=='interrupt-after-post':
   report=root/'state/report.json';report.unlink();report.mkdir()
  print(json.dumps(i))
 elif '--paginate' in args:
  endpoint=args[-1]
  if '/comments?' in endpoint: print('[[]]')
  else:
   repo=endpoint.split('/issues')[0][6:]
   print('[[]]' if mode=='hidden-list' else json.dumps([[i for i in issues if f'/{repo}/' in i['html_url']]]))
 else:
  endpoint=args[-1]
  if '/commits/' in endpoint: print(json.dumps({'sha':('b' if mode=='stale' else 'a')*40}))
  elif '/issues/' in endpoint: print('{}')
  else: print(json.dumps({'default_branch':'main'}))
elif name=='codex':
 assert pathlib.Path(os.environ['HOME']) != root/'original-home'
 assert pathlib.Path(os.environ['XDG_CONFIG_HOME']).is_dir()
 codex_home=pathlib.Path(os.environ['CODEX_HOME'])
 assert codex_home == root/'original-codex'
 assert '--ignore-user-config' in args and '--ignore-rules' in args
 store=next(a for a in args if a.startswith('cli_auth_credentials_store='))
 if mode=='keyring':
  assert store=='cli_auth_credentials_store="keyring"'
  assert not (codex_home/'auth.json').exists()
 else:
  auth=json.loads((codex_home/'auth.json').read_text())
  if mode=='rotate-auth':
   generation=root/'generation'; expected=int(generation.read_text()) if generation.exists() else 0
   assert auth.get('generation',0)==expected
   generation.write_text(str(expected+1))
   (codex_home/'auth.json').write_text(json.dumps({'generation':expected+1}))
  else: assert auth=={'test':'auth-only'}
 assert 'GH_TOKEN' not in os.environ
 assert 'CODEX_REFRESH_TOKEN_URL_OVERRIDE' not in os.environ
 assert 'CODEX_REVOKE_TOKEN_URL_OVERRIDE' not in os.environ
 calls=root/'codex-calls';calls.write_text(str(int(calls.read_text())+1 if calls.exists() else 1))
 prompt=sys.stdin.read()
 reviews=json.loads(prompt.split('REVIEWS (data): ')[-1])
 workspace=pathlib.Path(args[args.index('--cd')+1])
 known=json.loads((workspace/'github-context.json').read_text())
 if mode=='timeout':
  import time;time.sleep(30)
 findings=[]
 destination='TrainLCD/'+mode[6:] if mode.startswith('route-') else 'TrainLCD/StationAPI'
 if mode.startswith('route-'):
  assert (workspace/mode[6:]/'source.rs').is_file()
  schema=json.loads(pathlib.Path(args[args.index('--output-schema')+1]).read_text())
  props=schema['properties']['findings']['items']['properties']
  assert destination in props['repository']['enum']
  assert destination in props['evidence']['items']['properties']['repository']['enum']
 schema=json.loads(pathlib.Path(args[args.index('--output-schema')+1]).read_text())
 allowed=schema['properties']['findings']['items']['properties']['review_keys']['items']['enum']
 assert allowed==[entry['review_key'] for entry in reviews]
 for entry in reviews:
  r=entry['review']
  assert entry['review_key']==r['store']+':'+r['id']
  findings.append({'review_keys':[entry['review_key']], 'topic':'station-name','status':'existing' if known else 'new',
   'repository':destination,'title':'駅名の誤記を修正','body':'報告された駅名を確認して修正する。','reason':'現行コードに誤記がある。',
   'evidence':[{'repository':destination,'path':'source.rs','line':1,'explanation':'駅名の定義'}],
   'related_urls':[known[0]['url']] if known else []})
 if mode.startswith('secret-'):
  field=mode[7:]
  if field=='explanation': findings[0]['evidence'][0][field]='ghp_not_a_real_secret'
  else: findings[0][field]='ghp_not_a_real_secret'
 if mode=='uncertain': findings[0]['status']='uncertain'
 if mode=='omitted': findings=[]
 if mode=='bad-evidence': findings[0]['evidence'][0]['path']='../github-context.json'
 if mode=='new-despite-marker': findings[0]['status']='new';findings[0]['related_urls']=[]
 pathlib.Path(args[args.index('--output-last-message')+1]).write_text(json.dumps({'findings':findings}))
"#;
        for name in ["gh", "git", "codex"] {
            let p = dir.path().join("bin").join(name);
            fs::write(&p, script).unwrap();
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::create_dir(dir.path().join("original-codex")).unwrap();
        fs::write(
            dir.path().join("original-codex/auth.json"),
            r#"{"test":"auth-only"}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("original-codex/config.toml"),
            "model = \"not-forwarded\"",
        )
        .unwrap();
        let reviews = json!([{"store":"google","id":"r1","title":"","text":"駅名が違う","rating":5,"updated_at":"2026-11-01T00:00:00Z","version":"1.0","locale":"ja","url":"https://play.google.com/store/apps/details?id=me.tinykitten.trainlcd","developer_reply":""}]);
        fs::write(
            dir.path().join("input.json"),
            serde_json::to_vec(&reviews).unwrap(),
        )
        .unwrap();
        Self { dir }
    }
    fn run(&self, apply: bool, mode: &str) -> std::process::Output {
        let recheck = mode.starts_with("recheck-");
        let mode = mode.strip_prefix("recheck-").unwrap_or(mode);
        let mut c = Command::new(env!("CARGO_BIN_EXE_trainlcd-review-triage"));
        c.args(["run", "--input"])
            .arg(self.dir.path().join("input.json"))
            .arg("--state-dir")
            .arg(self.dir.path().join("state"))
            .args(["--timeout-seconds", "1"])
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.dir.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("HOME", self.dir.path().join("original-home"))
            .env("CODEX_HOME", self.dir.path().join("original-codex"))
            .env("GH_TOKEN", "test-not-forwarded")
            .env(
                "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
                "https://invalid.example",
            )
            .env("CODEX_REVOKE_TOKEN_URL_OVERRIDE", "https://invalid.example")
            .env("MOCK_ROOT", self.dir.path())
            .env("MOCK_MODE", mode);
        if recheck {
            c.arg("--recheck");
        }
        if apply {
            c.arg("--apply");
        }
        c.output().unwrap()
    }
    fn count(&self) -> usize {
        fs::read(self.dir.path().join("issues.json"))
            .ok()
            .map(|b| serde_json::from_slice::<Vec<Value>>(&b).unwrap().len())
            .unwrap_or(0)
    }
}
#[test]
fn dry_run_then_apply_then_rerun_does_not_duplicate() {
    let h = Harness::new();
    let o = h.run(false, "normal");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(h.count(), 0);
    let o = h.run(true, "normal");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(h.count(), 1);
    let o = h.run(true, "normal");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(h.count(), 1);
    let o = h.run(true, "recheck-new-despite-marker");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(h.count(), 1);
}
#[test]
fn incomplete_invalid_stale_and_timed_out_analysis_never_posts() {
    for mode in ["omitted", "bad-evidence", "stale", "timeout"] {
        let h = Harness::new();
        let o = h.run(true, mode);
        assert!(!o.status.success(), "{mode}");
        assert_eq!(h.count(), 0, "{mode}");
    }
}
#[test]
fn ambiguous_post_is_not_retried() {
    let h = Harness::new();
    assert!(!h.run(true, "ambiguous").status.success());
    assert_eq!(h.count(), 1);
    assert!(h.dir.path().join("state/pending-issue.json").exists());
    assert!(!h.run(true, "normal").status.success());
    assert_eq!(h.count(), 1);
}

#[test]
fn new_reviews_share_stable_marker_and_edited_reviews_are_reanalyzed() {
    let h = Harness::new();
    assert!(h.run(true, "normal").status.success());
    let path = h.dir.path().join("input.json");
    let mut rows: Vec<Value> = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut second = rows[0].clone();
    second["id"] = json!("r2");
    rows.push(second);
    fs::write(&path, serde_json::to_vec(&rows).unwrap()).unwrap();
    let o = h.run(true, "new-despite-marker");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(h.count(), 1);
    let calls = fs::read_to_string(h.dir.path().join("codex-calls")).unwrap();
    assert!(h.run(true, "normal").status.success());
    assert_eq!(
        calls,
        fs::read_to_string(h.dir.path().join("codex-calls")).unwrap()
    );
    rows[0]["text"] = json!("編集された要望");
    rows[0]["updated_at"] = json!("2026-11-12T00:00:00Z");
    fs::write(path, serde_json::to_vec(&rows).unwrap()).unwrap();
    assert!(h.run(true, "normal").status.success());
    let report: Value =
        serde_json::from_slice(&fs::read(h.dir.path().join("state/report.json")).unwrap()).unwrap();
    assert_eq!(report["review_count"], 1);
    assert_eq!(h.count(), 1);
}
#[test]
fn uncertain_reviews_remain_pending_and_dry_run_does_not_mark_processed() {
    let h = Harness::new();
    assert!(h.run(false, "normal").status.success());
    assert!(!h.dir.path().join("state/processed.json").exists());
    assert!(h.run(true, "uncertain").status.success());
    assert!(h.run(true, "normal").status.success());
    assert_eq!(h.count(), 1);
}
#[test]
fn credential_patterns_in_every_model_field_never_reach_github_or_report() {
    for field in ["title", "body", "reason", "explanation"] {
        let h = Harness::new();
        let o = h.run(true, &format!("secret-{field}"));
        assert!(!o.status.success());
        assert_eq!(h.count(), 0);
        assert!(!String::from_utf8_lossy(&o.stderr).contains("ghp_not_a_real_secret"));
        assert!(!h.dir.path().join("state/report.json").exists());
    }
}

#[test]
fn successful_posts_survive_interruption_and_missing_list_entries() {
    let h = Harness::new();
    let o = h.run(true, "interrupt-after-post");
    assert!(!o.status.success());
    assert_eq!(h.count(), 1);
    assert!(!h.dir.path().join("state/pending-issue.json").exists());
    let receipts: Value =
        serde_json::from_slice(&fs::read(h.dir.path().join("state/created-issues.json")).unwrap())
            .unwrap();
    assert_eq!(receipts.as_object().unwrap().len(), 1);
    assert_eq!(
        receipts.as_object().unwrap().values().next().unwrap(),
        "https://github.com/TrainLCD/StationAPI/issues/1"
    );
    fs::remove_dir(h.dir.path().join("state/report.json")).unwrap();
    let o = h.run(true, "hidden-list");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(h.count(), 1);
}

#[test]
fn recheck_uncertain_removes_previous_processed_fingerprint() {
    let h = Harness::new();
    assert!(h.run(true, "normal").status.success());
    assert!(h.run(true, "recheck-uncertain").status.success());
    let processed: Value =
        serde_json::from_slice(&fs::read(h.dir.path().join("state/processed.json")).unwrap())
            .unwrap();
    assert!(processed.as_object().unwrap().is_empty());
    let before = fs::read_to_string(h.dir.path().join("codex-calls")).unwrap();
    assert!(h.run(true, "normal").status.success());
    assert_ne!(
        before,
        fs::read_to_string(h.dir.path().join("codex-calls")).unwrap()
    );
    assert_eq!(h.count(), 1);
}

#[test]
fn auth_refresh_persists_and_keyring_retains_native_storage_identity() {
    let h = Harness::new();
    for _ in 0..2 {
        let o = h.run(false, "rotate-auth");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    assert_eq!(
        fs::read_to_string(h.dir.path().join("generation")).unwrap(),
        "2"
    );
    let h = Harness::new();
    fs::remove_file(h.dir.path().join("original-codex/auth.json")).unwrap();
    fs::write(
        h.dir.path().join("original-codex/config.toml"),
        "cli_auth_credentials_store = 'keyring'\nmodel = 'not-forwarded'\n",
    )
    .unwrap();
    let o = h.run(false, "keyring");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn added_destinations_support_evidence_and_existing_issue_deduplication() {
    for repo in ["Functions", "Website"] {
        let h = Harness::new();
        let mode = format!("route-{repo}");
        let o = h.run(false, &mode);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(h.count(), 0);
        let o = h.run(true, &mode);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let issues: Vec<Value> =
            serde_json::from_slice(&fs::read(h.dir.path().join("issues.json")).unwrap()).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0]["html_url"],
            format!("https://github.com/TrainLCD/{repo}/issues/1")
        );
        assert!(issues[0]["body"].as_str().unwrap().contains(&format!(
            "https://github.com/TrainLCD/{repo}/blob/{}/source.rs#L1",
            "a".repeat(40)
        )));
        // Recheck forces analysis and fetches the existing issue from the new destination.
        let o = h.run(true, &format!("recheck-{mode}"));
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(h.count(), 1);
        let report: Value =
            serde_json::from_slice(&fs::read(h.dir.path().join("state/report.json")).unwrap())
                .unwrap();
        assert_eq!(report["findings"][0]["status"], "existing");
    }
}
