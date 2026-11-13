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
    stage = STAGES.get(lines[0].strip(), "レビュー解析")
    details = []
    for line in lines:
        cause = re.sub(r"^\s*(?:\d+:\s*)?", "", line).strip()
        status = re.fullmatch(
            r"HTTP ([1-5][0-9]{2})[ A-Za-z]*(?:; retry later| \(check credentials and permissions\))",
            cause,
        )
        if status:
            details.append("HTTP " + status.group(1))
        elif cause in CAUSES:
            details.append(CAUSES[cause])
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
