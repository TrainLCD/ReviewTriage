import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('cost_summary', Path(__file__).parents[1] / 'scripts/cost_summary.py')
cost = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cost)


class CostSummaryTests(unittest.TestCase):
    def test_untrusted_fields_are_never_echoed(self):
        for value in [None, [], {'estimated_cost_usd': 'PRIVATE', 'codex_invocations': 'SECRET'},
                      {'estimated_cost_usd': float('nan'), 'codex_invocations': True}]:
            text = cost.summary(value)
            self.assertIn('不明', text)
            self.assertNotIn('PRIVATE', text)
            self.assertNotIn('SECRET', text)

    def test_only_numeric_fields_are_shown(self):
        text = cost.summary({'estimated_cost_usd': .123456, 'codex_invocations': 3,
                             'calls': [{'review': 'PRIVATE'}]})
        self.assertIn('$0.123456', text)
        self.assertIn('3', text)
        self.assertNotIn('PRIVATE', text)
