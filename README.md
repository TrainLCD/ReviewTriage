# ReviewTriage

TrainLCDのApp Store／Google Playレビューを公式APIから取得し、Codex CLIで現行コードと既存Issue・PRを調べ、未対応の要望・不具合を `TrainLCD/MobileApp` または `TrainLCD/StationAPI` に起票するRust製CLI。

## 準備

- Rust stable（検証: 1.99.1）、Git、GitHub CLI (`gh`)、Codex CLI、`rg`。
- `gh auth login`：両リポジトリの読み取りとIssue作成権限。
- `codex login`：使用するモデルにアクセスできるアカウント。
- App Store Connect APIキーと、Google Play Developer API用サービスアカウント。

```sh
cargo install --path . --locked
trainlcd-review-triage --help
```

### App Store Connect

```sh
export ASC_ISSUER_ID='your-issuer-id'
export ASC_KEY_ID='your-key-id'
export ASC_PRIVATE_KEY_PATH='/secure/AuthKey.p8'
```

チームAPIキー（ES256）でJWTを生成する。レビューを読み取れる権限が必要。既に生成した短期JWTを使う場合は `APP_STORE_CONNECT_TOKEN` を設定できる。キーをリポジトリに置かない。

### Google Play

```sh
export GOOGLE_APPLICATION_CREDENTIALS='/secure/play-service-account.json'
```

Google Play Developer APIを有効にし、サービスアカウントをPlay Consoleに招待してTrainLCDの「レビューへの返信」権限を与える。このツールはレビューへの返信は行わない。サービスアカウントJSONからOAuthアクセストークンを取得する。既存の短期トークンを使う場合は `GOOGLE_ACCESS_TOKEN` でもよい。

## 実行

```sh
# 取得・解析して起票案を保存（GitHubへの書き込みなし）
trainlcd-review-triage run

# 取得・解析し、未対応と判定できたものを起票
trainlcd-review-triage run --apply

# 過去のGoogle Playレビューも含める（月別CSVを複数指定可）
trainlcd-review-triage run \
  --play-csv reviews_me.tinykitten.trainlcd_202610.csv \
  --play-csv reviews_me.tinykitten.trainlcd_202611.csv

# 取得だけ行う
trainlcd-review-triage fetch --out reviews.json

# 取得済みJSONを解析。ストアAPIの認証不要（GitHub/Codex認証は必要）
trainlcd-review-triage run --input reviews.json --apply

# モデルと解析上限時間を指定（各バッチ、既定10レビュー）
trainlcd-review-triage run --model YOUR_MODEL --batch-size 10 --timeout-seconds 900
```

既定の保存先は `.review-triage/`。`--state-dir` で変更可能。

- `reviews.json`：取得履歴。次回の取得内容とID・更新日時で統合する。
- `report.json`：レビュー件数、取得範囲、解析時のコミットSHA、判定理由、起票本文、作成済みIssue URL。
- `pending-issue.json`：起票結果が不明な場合の復旧用記録。

**`run --apply` は常に最新の情報で再解析する。前回のreportをそのまま投稿するコマンドではない。** 保留や対応済みのレビューも次回再評価する。履歴が多いほどCodexの利用量・実行時間が増える。

## 取得範囲

- 対象は本番アプリ：Apple `1486355943`、Android `me.tinykitten.trainlcd`。
- App Storeは既定 `JPN`。`--territory USA` のようにISO alpha-3で変更でき、ページ末尾まで取得する。
- Google Play APIは**直近1週間に投稿・更新された文章レビューのみ**。国別フィルターはないため、全言語を原文で取得する。`--territory` はGoogleには適用されない。
- Googleの過去分はPlay Consoleの月別レビューCSV（UTF-8／UTF-16）を指定する。公式CSV以外の列形式や別アプリのCSVはエラーにする。
- 星だけの評価は解析対象外。高評価でも要望・不具合が書かれていれば対象。
- CSVにReview Linkがある場合、レビューIDを抽出してAPI取得分と統合する。リンクのない行は内容由来のIDになるため、API分との同一性や編集前後の同一性を厳密には保証できない。既存Issueとの意味的照合で重複を抑える。
- 定期実行の設定は含まない。7日を超えて取得が空いた場合、CSVで欠落分を補う。

## 判定と起票

| 判定 | 動作 |
| --- | --- |
| `new` | 現行コード上の根拠があり、既存チケットで扱われていない具体的な要望・不具合。`--apply` 時のみ起票 |
| `existing` | 既存Issue／PRまたは同一実行中の起票案に該当。新規起票しない |
| `resolved` | 実装で対応を確認できたもの。ストアへリリース済みとは限らない |
| `ignore` | 感想のみ等、対応対象なし |
| `uncertain` | 再現性、対応状況、担当先、証拠が不足。起票せずレポートに残す |

UI・測位・TTS・クライアントはMobileApp、駅・路線・種別・GTFSデータとAPIはStationAPIへ振り分ける。1レビュー内の複数要望を分け、同じ問題のレビューはまとめる。返信済み／Issueが閉じた事実だけでは修正済みとしない。

両リポジトリのデフォルトブランチを一時ディレクトリに取得し、全状態のIssue・PR本文をページネーションで収集する。該当するチケットのコメント・PR詳細は追加取得して再評価する。Git履歴は各100コミットまで。足りない証拠から完了や未対応を断定せず、判断保留にする。既存のローカル作業ツリーは変更しない。

Codexは読み取り専用で呼び出し、JSON Schemaによる出力をさらにRust側で検証する。全レビューの判定漏れ、未知のレビューID、対象外リポジトリ、存在しないコード位置、未知のIssue URLを拒否する。モデルによる意味的判断自体の正しさは保証できないため、初回はdry-runのレポートを確認する。

解析中にコードやIssueが変われば起票を中止する。本文の識別マーカーと既存Issue照合、同じstate-dirの排他ロックで再実行時の重複を抑制する。複数マシン・別state-dirからの同時実行は避ける（GitHub Issue APIに原子的な重複防止機構はない）。

### 起票結果が不明になった場合

通信切断等で `pending-issue.json` が残ったら自動再送しない。その本文の識別マーカーを両リポジトリで確認し、作成済みか未作成かを確定する。確認後にこのファイルを退避して再実行する。作成済みIssueは次回の照合対象になる。途中まで作成できたURLはreportに記録する。

### データと認証

レビューとGitHubの本文・コードはCodexの解析に渡る。ユーザー名は取得データに保持せず、生のレビュー本文は起票時に自動転載しない。起票本文はAIによる要約なので、個人情報が含まれないかdry-runで確認する。APIトークン・秘密鍵の環境変数はCodex子プロセスへ渡さない。Codexのユーザー設定は読み込まず、MCP設定も空にし、通常のCodex認証のみ使用する。設定ファイル内のモデルを使わないので、モデル指定は `--model` を使う。

## 開発・検証

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

E2EテストはPython 3製の偽 `gh` / `git` / `codex` を使用し、外部通信・課金・実際のIssue作成を行わない。dry-run、起票・再実行、識別マーカー、解析漏れ、不正証拠、古いコード、タイムアウト、起票の応答不明を検証する。実ストア／Codexによるライブ検証には各認証が必要。

## 参照

- [Apple Customer Reviews API](https://developer.apple.com/documentation/appstoreconnectapi/get-v1-apps-_id_-customerreviews)
- [Google Play Reviews APIと1週間の制約](https://developers.google.com/android-publisher/reply-to-reviews)
- [公式レビューCSV形式](https://support.google.com/googleplay/android-developer/answer/6135870)
- [Codex非対話実行](https://developers.openai.com/codex/noninteractive/)
