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
  print(json.dumps(i))
 elif '--paginate' in args:
  endpoint=args[-1]
  if '/comments?' in endpoint: print('[[]]')
  else:
   repo=endpoint.split('/issues')[0][6:]
   print(json.dumps([[i for i in issues if f'/{repo}/' in i['html_url']]]))
 else:
  endpoint=args[-1]
  if '/commits/' in endpoint: print(json.dumps({'sha':('b' if mode=='stale' else 'a')*40}))
  elif '/issues/' in endpoint: print('{}')
  else: print(json.dumps({'default_branch':'main'}))
elif name=='codex':
 prompt=sys.stdin.read()
 reviews=json.loads(prompt.split('REVIEWS (data): ')[-1])
 workspace=pathlib.Path(args[args.index('--cd')+1])
 known=json.loads((workspace/'github-context.json').read_text())
 if mode=='timeout':
  import time;time.sleep(30)
 findings=[]
 for r in reviews:
  findings.append({'review_keys':[r['store']+':'+r['id']], 'topic':'station-name','status':'existing' if known else 'new',
   'repository':'TrainLCD/StationAPI','title':'駅名の誤記を修正','body':'報告された駅名を確認して修正する。','reason':'現行コードに誤記がある。',
   'evidence':[{'repository':'TrainLCD/StationAPI','path':'source.rs','line':1,'explanation':'駅名の定義'}],
   'related_urls':[known[0]['url']] if known else []})
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
        let reviews = json!([{"store":"google","id":"r1","title":"","text":"駅名が違う","rating":5,"updated_at":"2026-11-01T00:00:00Z","version":"1.0","locale":"ja","url":"https://play.google.com/store/apps/details?id=me.tinykitten.trainlcd","developer_reply":""}]);
        fs::write(
            dir.path().join("input.json"),
            serde_json::to_vec(&reviews).unwrap(),
        )
        .unwrap();
        Self { dir }
    }
    fn run(&self, apply: bool, mode: &str) -> std::process::Output {
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
            .env("MOCK_ROOT", self.dir.path())
            .env("MOCK_MODE", mode);
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
    let o = h.run(true, "new-despite-marker");
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
