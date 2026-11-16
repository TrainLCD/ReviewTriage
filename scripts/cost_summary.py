"""Allowlisted numeric cost summary. Never print raw model/review/error strings."""
import json
import math
import sys
from pathlib import Path


def summary(value):
    if not isinstance(value, dict):
        return 'API費用: 不明'
    calls = value.get('codex_invocations')
    cost = value.get('estimated_cost_usd')
    valid_cost = type(cost) in (int, float) and math.isfinite(cost) and 0 <= cost <= 1e9
    valid_calls = type(calls) is int and 0 <= calls <= 1e9
    return ('API概算費用: ' + (f'${cost:.6f}' if valid_cost else '不明')
            + ' / Codex呼び出し: ' + (str(calls) if valid_calls else '不明')
            + ' / 実請求未照合・実行中呼び出しの費用上限は保証しない')


if __name__ == '__main__':
    try:
        value = json.loads(Path(sys.argv[1]).read_text())
    except (OSError, ValueError, IndexError):
        value = None
    print(summary(value))
