import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).parents[1] / "scripts/run_triage.py"
spec = importlib.util.spec_from_file_location("progress", SCRIPT)
progress = importlib.util.module_from_spec(spec)
spec.loader.exec_module(progress)


class ProgressTests(unittest.TestCase):
    def test_only_known_phases_and_numeric_counts_can_be_displayed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "progress.json"
            valid = dict(phase="analyzing", total=138, batch=2, batches=14,
                         analyzed=10, failed=0, created=0, pending=0, text="PRIVATE REVIEW")
            path.write_text(json.dumps(valid))
            message = progress.display(progress.read_progress(path), 60)
            self.assertIn("10/138件", message)
            self.assertIn("2/14", message)
            self.assertNotIn("PRIVATE REVIEW", message)
            for invalid in (dict(valid, phase="PRIVATE REVIEW"), dict(valid, total="SECRET"),
                            dict(valid, failed=True), dict(valid, batch=-1), []):
                path.write_text(json.dumps(invalid))
                self.assertIsNone(progress.read_progress(path))

    def test_wrapper_preserves_exit_code_and_keeps_raw_output_private(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log, state = root / "raw.log", root / "progress.json"
            state.write_text(json.dumps(dict(phase="complete", total=999, batch=1, batches=1,
                                            analyzed=999, failed=0, created=99, pending=0)))
            code = "import sys;print('PRIVATE REVIEW');print('SECRET',file=sys.stderr);sys.exit(7)"
            result = subprocess.run([sys.executable, str(SCRIPT), str(log), str(state),
                                     sys.executable, "-c", code], capture_output=True, text=True)
            self.assertEqual(result.returncode, 7)
            self.assertIn("CLI終了: 失敗", result.stdout)
            self.assertNotIn("999", result.stdout)
            self.assertNotIn("PRIVATE REVIEW", result.stdout + result.stderr)
            self.assertNotIn("SECRET", result.stdout + result.stderr)
            self.assertIn("PRIVATE REVIEW", log.read_text())
            self.assertIn("SECRET", log.read_text())
