import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "triage_error", Path(__file__).parents[1] / "scripts/report_triage_error.py"
)
diagnostics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(diagnostics)


class DiagnosticsTests(unittest.TestCase):
    def test_unknown_http_reason(self):
        message = diagnostics.summarize(
            "Error: App Store fetch failed; no issues published\n"
            "Caused by:\n    HTTP 520 <unknown status code>; retry later\n"
        )
        self.assertIn("HTTP 520", message)
        self.assertNotIn("<unknown status code>", message)

    def test_apostrophe_in_http_reason(self):
        message = diagnostics.summarize(
            "Error: App Store fetch failed; no issues published\n"
            "Caused by:\n    HTTP 418 I'm a teapot (check credentials and permissions)\n"
        )
        self.assertIn("HTTP 418", message)
        self.assertNotIn("I'm a teapot", message)

    def test_apple_http_failure(self):
        message = diagnostics.summarize(
            "Error: App Store fetch failed; no issues published\n\n"
            "Caused by:\n    HTTP 401 Unauthorized (check credentials and permissions)\n"
        )
        self.assertIn("App Storeレビュー取得", message)
        self.assertIn("HTTP 401", message)

    def test_google_numbered_chain(self):
        message = diagnostics.summarize(
            "Error: Google Play fetch failed; no issues published\n\n"
            "Caused by:\n    0: HTTP 403 Forbidden (check credentials and permissions)\n"
        )
        self.assertIn("Google Playレビュー取得", message)
        self.assertIn("HTTP 403", message)

    def test_retry_exhausted(self):
        self.assertIn("HTTP 503", diagnostics.summarize(
            "Error: App Store fetch failed; no issues published\n"
            "Caused by:\n    HTTP 503 Service Unavailable; retry later\n"
        ))

    def test_key_error(self):
        self.assertIn("秘密鍵の形式が不正", diagnostics.summarize(
            "Error: App Store fetch failed; no issues published\n"
            "Caused by:\n    InvalidKeyFormat\n"
        ))

    def test_private_content_and_workflow_commands_are_not_echoed(self):
        secret = "sk-proj-private-review-username"
        message = diagnostics.summarize(
            "Error: " + secret + "\n::warning::" + secret
            + "\nHTTP 401 Unauthorized (check credentials and permissions) " + secret
        )
        self.assertNotIn(secret, message)
        self.assertNotIn("::warning::", message)
        self.assertNotIn("HTTP 401", message)

    def test_last_top_level_error_wins(self):
        message = diagnostics.summarize(
            "Error: App Store fetch failed; no issues published\n"
            "HTTP 401 Unauthorized (check credentials and permissions)\n"
            "Error: Codex analysis failed; no issues published\n"
        )
        self.assertIn("Codex解析", message)
        self.assertNotIn("HTTP 401", message)

    def test_missing_error(self):
        self.assertIn("診断可能なエラーなし", diagnostics.summarize("private text"))

    def test_unknown_error_is_not_echoed(self):
        self.assertNotIn("private", diagnostics.summarize("Error: private"))


if __name__ == "__main__":
    unittest.main()

