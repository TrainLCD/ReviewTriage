"""Keep raw CLI/model output private; show only allowlisted numeric progress."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

PHASES = {
    "fetching": "レビュー取得", "preparing": "コード・Issue取得",
    "analyzing": "解析", "discussions": "既存Issueの議論確認",
    "publishing": "再照合・起票", "saving": "処理済み状態保存",
    "complete": "完了", "partial": "部分失敗", "failed": "失敗",
}
FIELDS = ("total", "batch", "batches", "analyzed", "failed", "created")


def read_progress(path):
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(value, dict) or not isinstance(value.get("phase"), str):
        return None
    if value["phase"] not in PHASES:
        return None
    if any(type(value.get(k)) is not int or not 0 <= value[k] <= 1000000000 for k in FIELDS):
        return None
    return {"phase": value["phase"], **{k: value[k] for k in FIELDS}}


def display(value, elapsed):
    if value is None:
        return f"進捗待機 / 経過 {elapsed}秒"
    return (f"{PHASES[value['phase']]} / 解析済み {value['analyzed']}/{value['total']}件"
            f" / 失敗 {value['failed']}件 / バッチ {value['batch']}/{value['batches']}"
            f" / 起票 {value['created']}件 / 経過 {elapsed}秒")


def main():
    log_path, progress_path, *command = sys.argv[1:]
    # Never display restored progress from a previous run.
    Path(progress_path).unlink(missing_ok=True)
    started = time.monotonic()
    with open(log_path, "wb") as log:
        child = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)

        def stop(signum, _frame):
            try:
                os.killpg(child.pid, signum)
            except ProcessLookupError:
                pass

        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        last, printed = object(), 0.0
        while True:
            value = read_progress(progress_path)
            now = time.monotonic()
            code = child.poll()
            if value != last or now - printed >= 30 or code is not None:
                print(display(value, int(now - started)), flush=True)
                last, printed = value, now
            if code is not None:
                print("CLI終了: " + ("成功" if code == 0 else "失敗"), flush=True)
                return code if code >= 0 else 128 - code
            time.sleep(1)


if __name__ == "__main__":
    sys.exit(main())
