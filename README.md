# ReviewTriage

TrainLCDのApp Store／Google Playレビューを公式APIから取得し、Codex CLIで現行コードと既存Issue・PRを調べ、未対応の要望・不具合を `TrainLCD/MobileApp`、`TrainLCD/StationAPI`、`TrainLCD/Functions`、`TrainLCD/Website` のいずれかに起票するRust製CLI。

## GitHub Actionsで実行（PC不要）

初回だけ、リポジトリの **Settings → Secrets and variables → Actions** に以下のRepository secretsを登録する。

| Secret | 内容 |
| --- | --- |
| `REVIEW_TRIAGE_GITHUB_TOKEN` | MobileApp／StationAPI／Functions／WebsiteのContents・Pull requestsの読み取りとIssuesの読み書きができるfine-grained PAT |
| `OPENAI_API_KEY` | Codexで使うOpenAI APIキー（API利用料金が発生） |
| `ASC_ISSUER_ID` | App Store ConnectのIssuer ID |
| `ASC_KEY_ID` | App Store ConnectのKey ID |
| `ASC_PRIVATE_KEY` | App Store Connectの.p8ファイル全文（改行を含む） |
| `GOOGLE_SERVICE_ACCOUNT_JSON` | Google Play用サービスアカウントJSON全文 |

既存のfine-grained PATを使う場合は、対象リポジトリにFunctionsとWebsiteも追加し、上記権限を付与する。GitHub Actionsも同じ4リポジトリを調査・起票対象にする。過去の処理済みレビューを追加先も含めて再評価する場合は `--recheck` を使用する。

ストア側の権限設定は下記「App Store Connect」「Google Play」を参照。標準の `GITHUB_TOKEN` では別リポジトリに起票できないため、専用PATを使用する。秘密鍵やJSONをworkflowの入力欄やリポジトリに貼らない。

1. **Actions → Review triage → Run workflow** を開く（スマートフォンのブラウザーでも可）。
2. デフォルトブランチを選ぶ。`apply` をオフにすると解析のみ、オンにすると未対応の要望・不具合を起票する。
3. 必要なら `recheck` をオンにし、App Storeの地域を `territory` で指定する（既定 `JPN`）。
4. 完了後、実行画面の **Artifacts → review-triage-state** をダウンロードし、`report.json` で判定・起票結果を確認する。

workflowがデフォルトブランチにマージされると実行ボタンが表示される。Rust・Codex CLI等の準備から実行までGitHubのクラウドで行う。定期実行はせず、ボタン操作で開始する。Google Playの過去CSV取り込みは引き続きローカルCLIを使う。

### 実行状態の引き継ぎと復旧

前回の実行から状態artifactを復元し、失敗時も復元済み状態と途中の起票記録を保存する。同時実行は最大100件を待機させて直列化する。復元元は実際のジョブ完了順で選び、未開始のままキャンセルされた実行は除外する。新しい依頼が先に実行された場合、古い依頼は最新状態を保存して起票前に停止する。デフォルトブランチ以外での実行はスキップする。**再実行は「Re-run jobs」ではなく「Run workflow」で新しい実行を開始する。** ローカルCLIとの同時実行も避ける。

artifactの保持は90日（リポジトリ側の上限も適用）。期限切れ・削除・キャンセルなどで直前のartifactが取得できない場合は、状態を失ったまま起票しないよう停止する。古いartifactへ自動で戻ることもしない。その場合は直前の起票状況を確認し、手元のバックアップから復旧するための保守が必要。初回実行だけは空の状態から開始する。実行履歴・artifactをすべて消して初回扱いにしないこと。

`pending-issue.json` がある場合も自動再送しない。下記「起票結果が不明になった場合」に従って照合・復旧する。Actionsにはこの記録を強制削除するボタンを設けていない。

**この公開リポジトリではartifactも閲覧者がダウンロードできる。** レビュー本文・返信・解析結果が含まれるため、公開できないデータを扱う場合は非公開リポジトリで運用する。認証ファイルはartifactに含めず、Codexの生ログも公開ログへ出力しない。Codex失敗時はJSON形式の終端エラー（`error`／`turn.failed`）、終了コード・シグナル・経過秒をActionsログに表示する。エラー本文の認証情報・URL・メールアドレス・キャッシュ済みレビュー文字列を伏せ、最大3件・各2,000文字に制限する。通常の回答・推論・コマンド出力は表示しない。構造化エラーが出る前に終了した場合も終了状態は表示するが、詳細がない旨を明示する。

## 準備

- Rust stable（検証: 1.99.1）、Git、GitHub CLI (`gh`)、Codex CLI、`rg`。
- `gh auth login`：対象4リポジトリ（MobileApp／StationAPI／Functions／Website）の読み取りとIssue作成権限。
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
- `processed.json`：成功した `--apply` で処理を確定したレビューのIDと内容ハッシュ。
- `report.json`：レビュー件数、取得範囲、解析時のコミットSHA、判定理由、起票本文、作成済みIssue URL。
- `created-issues.json`：起票成功直後に永続化する識別マーカーとIssue URL。中断後や一覧反映前の再実行でも重複を防ぐ。
- `pending-issue.json`：起票結果が不明な場合の復旧用記録。
- `failed-batches.json`：解析に失敗したバッチ番号とレビュー参照。本文や未検証の解析結果は含めない。
- `progress.json`：現在の処理段階、対象・解析済み・失敗件数、バッチ番号、起票件数。Actionsでは変更時と30秒ごとに件数と経過時間を表示する。レビュー本文やモデルの生出力は公開ログに出さない。解析済み件数は起票・処理済み保存の完了件数ではない。

**`run --apply` は未処理・編集済み・判断保留のレビューを最新の情報で解析する。前回のreportをそのまま投稿するコマンドではない。** 解析に失敗したバッチは記録して残りを続行する。検証済みバッチの起票が成功したら、そのレビューだけ処理済み状態を保存する。失敗・判断保留分は次回の対象に残り、一部失敗時は保存後も終了コードを失敗として返す。dry-runは処理済みにしない。処理済みレビューも再評価する場合は `run --recheck` または `run --recheck --apply` を使う。レビューの本文・更新日時・返信等が変わった場合は自動で再解析する。コード変更・Issue一覧の競合・起票の通信障害など、公開の安全性に関わるエラーは実行を停止する。

## 取得範囲

- 対象は本番アプリ：Apple `1486355943`、Android `me.tinykitten.trainlcd`。
- App Storeは既定 `JPN`。`--territory USA` のようにISO alpha-3で変更でき、ページ末尾まで取得する。
- Google Play APIは**直近1週間に投稿・更新された文章レビューのみ**。国別フィルターはないため、全言語を原文で取得する。`--territory` はGoogleには適用されない。
- Googleの過去分はPlay Consoleの月別レビューCSV（UTF-8／UTF-16）を指定する。公式CSV以外の列形式や別アプリのCSVはエラーにする。
- 星だけの評価は解析対象外。高評価でも要望・不具合が書かれていれば対象。
- CSVのReview LinkからレビューIDを抽出してAPI取得分と統合する。リンクがない／IDを抽出できない行は、行番号を示して取り込み全体を中止する。本文や投稿時刻から代替IDを作ることはしない。IDのある公式エクスポートで補完する。
- 定期実行の設定は含まない。7日を超えて取得が空いた場合、CSVで欠落分を補う。

## 判定と起票

同一バッチ内のトピック重複、根拠の行番号が範囲外・説明が空、新規判定に既存Issue/PR参照が付いた矛盾は、最大2回再解析する（修正理由を問わず合計2回）。重複にはレビュー参照の統合、根拠の不備には全不正箇所の添字・実ファイル行数を返して再調査を指示する。既存参照との矛盾は対応範囲を再確認し、既存または判断保留に再判定させる。検証を通すためにURLを消すことや、行番号の丸め・検証の省略は行わない。各回に通常のタイムアウトが適用され、追加のAPI利用料金が発生し得る。解消しないバッチは起票対象から除外し、他のバッチを続行する。

| 判定 | 動作 |
| --- | --- |
| `new` | 現行コード上の根拠があり、既存チケットで扱われていない具体的な要望・不具合。`--apply` 時のみ起票 |
| `existing` | 既存Issue／PRまたは同一実行中の起票案に該当。新規起票しない |
| `resolved` | 実装で対応を確認できたもの。ストアへリリース済みとは限らない |
| `ignore` | 感想のみ等、対応対象なし |
| `uncertain` | 再現性、対応状況、担当先、証拠が不足。起票せずレポートに残す |

アプリUI・測位・クライアント側TTS再生・ナビゲーションはMobileApp、駅・路線・種別・GTFSデータとAPIはStationAPI、サーバー側TTS生成・認証・フィードバック処理などのバックエンドはFunctions、公式サイトのページ・コンテンツ・Web UIはWebsiteへ振り分ける。TTSなど複数リポジトリに関係する報告は症状だけで決めず、コードの根拠で担当を判定し、特定できなければuncertainとする。1レビュー内の複数要望を分け、同じ問題のレビューはまとめる。返信済み／Issueが閉じた事実だけでは修正済みとしない。

対象4リポジトリ（MobileApp／StationAPI／Functions／Website）のデフォルトブランチを一時ディレクトリに取得し、全状態のIssue・PR本文をページネーションで収集する。該当するチケットのコメント・PR詳細は追加取得して再評価する。Git履歴は各100コミットまで。足りない証拠から完了や未対応を断定せず、判断保留にする。既存のローカル作業ツリーは変更しない。

Codexは読み取り専用で呼び出し、JSON Schemaによる出力をさらにRust側で検証する。全レビューの判定漏れ、未知のレビューID、対象外リポジトリ、存在しないコード位置、未知のIssue URLを拒否する。モデルによる意味的判断自体の正しさは保証できないため、初回はdry-runのレポートを確認する。

解析中のコード変更、Issue/PRの追加・削除・タイトル／本文の変更、照合対象チケットの状態変更を検出した場合は起票を中止する。無関係なコメントやラベルの更新日時は比較しない。自分が作成したIssueはPOST応答から照合対象へ追加し、一覧への反映遅延を許容する。リポジトリとtopicだけから生成した識別マーカー（レビュー集合に依存しない）と既存Issue照合、同じstate-dirの排他ロックで再実行時の重複を抑制する。topic表現自体が変わる場合の意味的照合はCodexに依存する。複数マシン・別state-dirからの同時実行は避ける（GitHub Issue APIに原子的な重複防止機構はない）。

### 起票結果が不明になった場合

通信切断等で `pending-issue.json` が残ったら自動再送しない。その本文の識別マーカーを対象4リポジトリ（MobileApp／StationAPI／Functions／Website）で確認し、作成済みか未作成かを確定する。確認後にこのファイルを退避して再実行する。作成済みIssueは次回の照合対象になる。成功したPOSTのURLとマーカーは、pendingを削除する前に `created-issues.json` へ原子的に保存する。途中まで作成できたURLはreportにも記録する。state-dirの成功記録は削除せず保持する。

### データと認証

レビューとGitHubの本文・コードはCodexの解析に渡る。ユーザー名は取得データに保持せず、生のレビュー本文は起票時に自動転載しない。起票本文はAIによる要約なので、個人情報が含まれないかdry-runで確認する。APIトークン・秘密鍵の環境変数はCodex子プロセスへ渡さない。`HOME`・`XDG_CONFIG_HOME` は一時ディレクトリに分離する。`CODEX_HOME` は認証ストレージの識別に必要な元のパスを使い、認証情報をコピー・書き戻ししない。トークン更新はCodex自身が元の保存先へ反映するため、次のバッチ・実行でも更新済み認証を利用する。`config.toml` からは `cli_auth_credentials_store` の `file` / `keyring` / `auto` だけを抽出して明示指定する（省略時はCodexの既定 `file`）。それ以外のユーザー設定とルールは読み込ませず、MCP設定も空にする。非対話実行では `CODEX_API_KEY` も利用できる。Actionsでは `OPENAI_API_KEY` Secretをこの環境変数にも渡す。認証更新・失効先URLの上書き環境変数は除去する。元プロファイルのファイルを読み取り不能にする仕組みではなく、他のCodexプロセスとの認証更新の競合はCodex本体の動作に従う。生成結果および起票直前のタイトル・本文でGitHubトークン接頭辞、秘密鍵ヘッダー、private_key、access_token等を検出したら内容を表示せず中止する。HOMEの分離と文字列検査は多層防御であり、絶対パスの読み取りや未知の秘密形式を完全に遮断するものではない。設定ファイル内のモデルを使わないので、モデル指定は `--model` を使う。

## 開発・検証

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

E2EテストはPython 3製の偽 `gh` / `git` / `codex` を使用し、外部通信・課金・実際のIssue作成を行わない。dry-run、起票・再実行、識別マーカー、解析漏れ、不正証拠、古いコード、タイムアウト、起票の応答不明に加え、増分解析、レビュー編集、判断保留の再評価、レビュー集合変更時のマーカー、起票成功後の中断と一覧反映遅延、再確認で判断保留となったレビュー、認証保存先の維持と更新引継ぎ、keyring設定引継ぎ、秘密情報の出力拒否を検証する。実ストア／Codexによるライブ検証には各認証が必要。

## 参照

- [Apple Customer Reviews API](https://developer.apple.com/documentation/appstoreconnectapi/get-v1-apps-_id_-customerreviews)
- [Google Play Reviews APIと1週間の制約](https://developers.google.com/android-publisher/reply-to-reviews)
- [公式レビューCSV形式](https://support.google.com/googleplay/android-developer/answer/6135870)
- [Codex非対話実行](https://developers.openai.com/codex/noninteractive/)

### 解析環境の起動確認

Actionsではシステムの `bubblewrap` をインストールし、認証情報を渡す前に `codex sandbox` で読み取り専用コマンドの起動を確認する。サンドボックスの制限は解除しない。
Codexが正常終了しても、JSONイベントに成功したコマンド実行が一件もない場合や、失敗したコマンドに `bwrap` の起動エラーがある場合は解析失敗とする。Issue作成・処理済み状態の更新より前に停止し、次回実行で再解析する。通常の証拠不足による `uncertain` は引き続き有効な判定として扱う。

解析コマンドには `inherit=none` を維持しつつ `PATH` のみ明示的に渡す。PATHまで除去するとシステムのbubblewrapを選べず、Ubuntuで同梱バイナリの起動が拒否される場合がある。事前検証も同じ環境変数方針で実行する。
