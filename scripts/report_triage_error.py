"""Emit fixed CLI diagnostics and redacted terminal Codex errors, never agent output."""
import re
import json
import os
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


def redact_message(message, sensitive=()):
    # Only terminal provider errors are passed here, never agent/tool events.
    # Redact before truncating so a key spanning the cutoff cannot leak.
    for value in sorted(set(sensitive), key=len, reverse=True):
        if value:
            message = message.replace(value, "[REDACTED]")
            for ensure_ascii in (True, False):
                encoded = json.dumps(value, ensure_ascii=ensure_ascii)[1:-1]
                message = message.replace(encoded, "[REDACTED]")
    message = re.sub(r"-----BEGIN[\s\S]*?-----END[^-]*-----", "[REDACTED]", message)
    message = re.sub(r"\b(?:sk-[A-Za-z0-9_-]+|gh[pousr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+)\b",
                     "[REDACTED]", message)
    message = re.sub(r"(?i)\bBearer\s+\S+", "Bearer [REDACTED]", message)
    message = re.sub(r"https?://[^\s\"<>]+", "[URL]", message)
    message = re.sub(r"[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}", "[EMAIL]", message)
    # Strip terminal control sequences and render all remaining controls literally.
    message = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", message)
    return json.dumps(message[:2000], ensure_ascii=False)


def codex_diagnostics(log, sensitive=()):
    errors = []
    status = None
    active = False
    for line in log.splitlines():
        try:
            event = json.loads(line)
        except (ValueError, TypeError):
            continue
        if not isinstance(event, dict):
            continue
        kind = event.get("type")
        if kind == "review_triage.codex.started":
            # Discard errors from earlier successful batches.
            active = True
            errors = []
            status = None
        elif active and kind == "review_triage.codex.exit":
            status = event
            active = False
        elif active and kind in ("error", "turn.failed"):
            error = event.get("error") if kind == "turn.failed" else event
            message = error.get("message") if isinstance(error, dict) else None
            if isinstance(message, str):
                safe = redact_message(message, sensitive)
                if safe not in errors:
                    errors.append(safe)
                errors = errors[-3:]
    details = []
    if status:
        for field, label in (("code", "終了コード"), ("signal", "シグナル"),
                             ("elapsed_seconds", "経過秒")):
            value = status.get(field)
            if type(value) is int:
                details.append(f"{label}={value}")
        if status.get("timed_out") is True:
            details.append("タイムアウト")
    if errors:
        details.append("Codexエラー=" + " / ".join(errors))
    elif status or active:
        details.append("Codexの構造化エラーイベントなし")
    return " / ".join(details)


def sensitive_values():
    values = [value for name, value in os.environ.items()
              if any(word in name for word in ("TOKEN", "KEY", "SECRET", "PASSWORD"))]
    # Provider errors can echo inputs. Remove cached review fields too.
    try:
        reviews = json.loads(Path("state/reviews.json").read_text(encoding="utf-8"))
        for review in reviews:
            if isinstance(review, dict):
                values.extend(value for value in review.values()
                              if isinstance(value, str) and len(value) >= 4)
    except (OSError, ValueError, TypeError):
        pass
    return values


def summarize(log, sensitive=()):
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
            r"(?:gh|git) failed \(exit (?:exit status: [0-9]+|"
            r"signal: [0-9]+(?: \(SIG[A-Z0-9]+\))?(?: \(core dumped\))?)"
            r"\); check installation/authentication",
            cause,
        ):
            details.append("GitHub取得またはGit操作のコマンド失敗")
        elif re.fullmatch(r"start (?:gh|git)", cause):
            details.append("GitHub CLIまたはGitの起動失敗")
    detail = " / ".join(dict.fromkeys(details)) or "詳細は許可済み診断項目に該当しない"
    if stage.startswith("Codex"):
        diagnostic = codex_diagnostics(log, sensitive)
        if diagnostic:
            detail = diagnostic
    return f"{stage}に失敗：{detail}（生ログは非公開）"


def main():
    try:
        log = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
        message = summarize(log, sensitive_values())
    except (OSError, IndexError):
        message = "レビュー解析失敗：診断ログを読み取れない（生ログは非公開）"
    print("::error::" + message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A"))


if __name__ == "__main__":
    main()



