import importlib.util
from pathlib import Path
import unittest
import json

spec = importlib.util.spec_from_file_location(
    "triage_error", Path(__file__).parents[1] / "scripts/report_triage_error.py"
)
diagnostics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(diagnostics)


class DiagnosticsTests(unittest.TestCase):
    def test_context_drift_is_identified_without_raw_output(self):
        for error in ("Issues/PRs changed during analysis; rerun to check for duplicates",
                      "TrainLCD/StationAPI changed during analysis; rerun before publishing",
                      "Issue context changed; only affected reviews deferred, other validated reviews saved"):
            self.assertNotIn("許可済み診断項目に該当しない", diagnostics.summarize("Error: " + error))

    def test_partial_failure_retains_allowlisted_failure_reason(self):
        event = {"type":"review_triage.batch_failed", "invalid_lines":0, "empty_explanations":0,
                 "failure_code":"new issue has existing match"}
        error = "\nError: some batches failed; validated reviews saved, failed reviews remain pending"
        self.assertIn("新規判定に既存Issueの参照あり", diagnostics.summarize(json.dumps(event) + error))
        event["failure_code"] = "PRIVATE REVIEW"
        self.assertNotIn("PRIVATE REVIEW", diagnostics.summarize(json.dumps(event) + error))

    def test_partial_failure_counts_only_failed_batches(self):
        events = [
            {"type":"review_triage.invalid_evidence", "problems":[{"supplied_line":999,"file_line_count":1}]},
            {"type":"review_triage.batch_failed", "invalid_lines":2,"empty_explanations":1,"text":"PRIVATE REVIEW"},
            {"type":"review_triage.batch_failed", "invalid_lines":3,"empty_explanations":0},
        ]
        message = diagnostics.summarize("\n".join(map(json.dumps, events)) + "\nError: some batches failed; validated reviews saved, failed reviews remain pending\n")
        self.assertIn("行番号範囲外=5件", message)
        self.assertIn("説明空欄=1件", message)
        self.assertNotIn("PRIVATE REVIEW", message)

    def test_evidence_diagnostics_disambiguate_without_echoing_model_text(self):
        event = {"type": "review_triage.invalid_evidence", "problems": [
            {"supplied_line": 999, "file_line_count": 1, "empty_explanation": False},
            {"supplied_line": 1, "file_line_count": 1, "empty_explanation": True,
             "path": "PRIVATE REVIEW", "explanation": "PRIVATE REVIEW"},
        ]}
        message = diagnostics.summarize(json.dumps(event) + "\nError: invalid evidence line\n")
        self.assertIn("行番号範囲外=1件", message)
        self.assertIn("説明空欄=1件", message)
        self.assertNotIn("PRIVATE REVIEW", message)

    def test_sandbox_failure_has_fixed_diagnostic(self):
        for error in ("analysis sandbox failed; no issues published",
                      "analysis commands never succeeded; no issues published"):
            message = diagnostics.summarize("Error: " + error)
            self.assertNotIn("許可済み診断項目に該当しない", message)

    def test_japanese_multiline_review_json_is_redacted(self):
        private = "駅名が違う\n修正して"
        for ensure_ascii in (True, False):
            with self.subTest(ensure_ascii=ensure_ascii):
                body = json.dumps({"text": private}, ensure_ascii=ensure_ascii)
                message = diagnostics.summarize(self.codex_log(body), [private])
                self.assertIn("[REDACTED]", message)
                self.assertNotIn("駅名が違う", message)
                self.assertNotIn("修正して", message)
                self.assertNotIn(r"\u99c5", message)


    def codex_log(self, message, code=1):
        return "\n".join(json.dumps(event) for event in [
            {"type": "review_triage.codex.started"},
            {"type": "turn.failed", "error": {"message": message}},
            {"type": "review_triage.codex.exit", "code": code, "elapsed_seconds": 12},
        ]) + "\nError: Codex analysis failed; no issues published\n"

    def test_codex_terminal_error_survives_outer_error(self):
        message = diagnostics.summarize(self.codex_log("Rate limit reached (429)"))
        self.assertIn("Rate limit reached (429)", message)
        self.assertIn("終了コード=1", message)
        self.assertIn("経過秒=12", message)

    def test_unknown_provider_error_is_preserved(self):
        self.assertIn("new provider failure",
                      diagnostics.summarize(self.codex_log("new provider failure")))

    def test_codex_redacts_credentials_and_review_content(self):
        private = "レビュー固有の本文"
        message = diagnostics.summarize(self.codex_log(
            private + " sk-proj-test-secret Bearer xyz person@example.com https://example.com/token?q=secret"
        ), [private])
        for value in (private, "sk-proj-test-secret", "xyz", "person@example.com", "q=secret"):
            self.assertNotIn(value, message)
        self.assertIn("REDACTED", message)

    def test_codex_ignores_agent_and_tool_output(self):
        log = "\n".join(json.dumps(event) for event in [
            {"type": "review_triage.codex.started"},
            {"type": "item.completed", "item": {"type": "agent_message", "text": "private"}},
            {"type": "item.completed", "item": {"type": "command_execution",
                                               "aggregated_output": "private"}},
            {"type": "review_triage.codex.exit", "signal": 9, "elapsed_seconds": 2},
        ])
        message = diagnostics.codex_diagnostics(log)
        self.assertNotIn("private", message)
        self.assertIn("シグナル=9", message)
        self.assertIn("構造化エラーイベントなし", message)

    def test_codex_discards_errors_from_prior_batches(self):
        log = self.codex_log("old failure") + self.codex_log("new failure", 42)
        message = diagnostics.summarize(log)
        self.assertIn("new failure", message)
        self.assertIn("終了コード=42", message)
        self.assertNotIn("old failure", message)

    def test_codex_error_event_and_malformed_events(self):
        log = "\n".join([
            '{"type":"review_triage.codex.started"}', "broken JSON", "[]",
            '{"type":"error","message":"stream disconnected"}',
            '{"type":"turn.failed","error":null}',
        ])
        self.assertIn("stream disconnected", diagnostics.codex_diagnostics(log))

    def test_codex_control_characters_are_escaped(self):
        message = diagnostics.summarize(self.codex_log("failure\n::warning::injected"))
        self.assertNotIn("\n", message)
        self.assertIn(r"\n", message)


    def test_signal_failure(self):
        for program in ("gh", "git"):
            for status in ("signal: 9 (SIGKILL)", "signal: 15 (SIGTERM)",
                           "signal: 11 (SIGSEGV) (core dumped)",
                           "signal: 35", "signal: 35 (core dumped)"):
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



