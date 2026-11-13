"""Emit only allowlisted diagnostics; never echo the private analysis log."""
import re
import sys
from pathlib import Path


STAGES = {
    "App Store fetch failed; no issues published": "App Storeレビュー取得",
    "Google Play fetch failed; no issues published": "Google Playレビュー取得",
    "Codex analysis failed; no issues published": "Codex解析",
    "Codex analysis timed out; no issues published": "Codex解析（タイムアウト）",
    "install and log in to Codex CLI": "Codex CLI起動",
}
CAUSES = {
    "invalid destination": "解析結果の起票先が不正",
    "invalid topic": "解析結果のトピックが不正",
    "duplicate topic in batch; retry analysis": "同一バッチ内でトピックが重複",
    "missing review or reason": "レビュー参照または判定理由が欠落",
    "unknown review key": "解析結果に未知のレビュー参照",
    "unknown related URL": "解析結果に未知の関連URL",
    "existing finding without source": "対応済み判定に根拠なし",
    "missing code evidence": "コード根拠が欠落",
    "new finding duplicates a prior batch": "以前のバッチと新規指摘が重複",
    "invalid issue draft": "Issue下書きが不正",
    "new issue has existing match": "新規判定に既存Issueの参照あり",
    "invalid evidence repository": "根拠のリポジトリが不正",
    "invalid source path": "根拠のファイルパスが不正",
    "source path escapes checkout": "根拠のパスがチェックアウト外",
    "invalid evidence line": "根拠の行番号または説明が不正",
    "Codex omitted reviews; no issues published": "解析結果に未処理レビューあり",
    "Codex did not produce a result": "Codexの結果ファイルなし",
    "credential-like content detected; output rejected": "解析結果に認証情報に似た文字列を検出",
    "another run is using this state directory": "同じ状態ディレクトリを別プロセスが使用中",
    "InvalidKeyFormat": "秘密鍵の形式が不正",
    "InvalidEcdsaKey": "Apple署名鍵が不正",
    "InvalidRsaKey": "Google署名鍵が不正",
    "No such file or directory (os error 2)": "必要なファイルまたはコマンドが存在しない",
    "Permission denied (os error 13)": "ファイルまたはコマンドへのアクセス権限不足",
}


def summarize(log):
    # Rust's top-level anyhow error is emitted after child output. Restrict
    # inspection to its error chain, and return constants/numeric status only.
    starts = list(re.finditer(r"(?m)^Error: ", log))
    if not starts:
        return "レビュー解析失敗：診断可能なエラーなし（生ログは非公開）"
    block = log[starts[-1].end():]
    lines = block.splitlines()
    stage = STAGES.get(lines[0].strip(), "レビュー処理")
    details = []
    for line in lines:
        cause = re.sub(r"^\s*(?:\d+:\s*)?", "", line).strip()
        status = re.fullmatch(
            r"HTTP ([1-5][0-9]{2})(?:[ A-Za-z']*| <unknown status code>)"
            r"(?:; retry later| \(check credentials and permissions\))",
            cause,
        )
        if status:
            details.append("HTTP " + status.group(1))
        elif cause in CAUSES:
            details.append(CAUSES[cause])
        elif re.fullmatch(
            r"(?:gh|git) failed \(exit exit status: [0-9]+\); check installation/authentication",
            cause,
        ):
            details.append("GitHub取得またはGit操作のコマンド失敗")
        elif re.fullmatch(r"start (?:gh|git)", cause):
            details.append("GitHub CLIまたはGitの起動失敗")
    detail = " / ".join(dict.fromkeys(details)) or "詳細は許可済み診断項目に該当しない"
    return f"{stage}に失敗：{detail}（生ログは非公開）"


def main():
    try:
        log = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
        message = summarize(log)
    except (OSError, IndexError):
        message = "レビュー解析失敗：診断ログを読み取れない（生ログは非公開）"
    print("::error::" + message)


if __name__ == "__main__":
    main()

