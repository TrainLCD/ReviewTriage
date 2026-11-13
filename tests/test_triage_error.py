import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "triage_error", Path(__file__).parents[1] / "scripts/report_triage_error.py"
)
diagnostics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(diagnostics)


class DiagnosticsTests(unittest.TestCase):
    def test_signal_failure(self):
        for program in ("gh", "git"):
            for status in ("signal: 9 (SIGKILL)", "signal: 15 (SIGTERM)",
                           "signal: 11 (SIGSEGV) (core dumped)",
                           "signal: 35 (unknown signal)"):
                raw = f"{program} failed (exit {status}); check installation/authentication"
                with self.subTest(program=program, status=status):
                    message = diagnostics.summarize("Error: " + raw)
                    self.assertIn("コマンド失敗", message)
                    self.assertNotIn(raw, message)
                    self.assertNotIn(status, message)

    def test_signal_failure_private_suffix_is_rejected(self):
        raw = ("Error: git failed (exit signal: 9 (SIGKILL)); "
               "check installation/authentication private")
        message = diagnostics.summarize(raw)
        self.assertNotIn("コマンド失敗", message)
        self.assertNotIn("private", message)

    def test_validation_errors(self):
        expected = {
            "unknown related URL": "未知の関連URL",
            "invalid evidence line": "行番号",
            "Codex omitted reviews; no issues published": "未処理レビュー",
            "Codex did not produce a result": "結果ファイルなし",
        }
        for error, detail in expected.items():
            with self.subTest(error=error):
                self.assertIn(detail, diagnostics.summarize("Error: " + error))

    def test_command_failure(self):
        for program in ("gh", "git"):
            message = diagnostics.summarize(
                f"Error: {program} failed (exit exit status: 1); check installation/authentication"
            )
            self.assertIn("コマンド失敗", message)

    def test_command_failure_with_private_suffix_is_rejected(self):
        message = diagnostics.summarize(
            "Error: gh failed (exit exit status: 1); check installation/authentication private"
        )
        self.assertNotIn("コマンド失敗", message)
        self.assertNotIn("private", message)


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


