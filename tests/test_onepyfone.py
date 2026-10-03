import importlib.util
import contextlib
import io
import json
import sys
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET
from pathlib import Path


RUNNER_PATH = Path(__file__).with_name("OnePyFone.py")
SPEC = importlib.util.spec_from_file_location("onepyfone", RUNNER_PATH)
assert SPEC and SPEC.loader
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)


class RunnerTests(unittest.TestCase):
    def setUp(self):
        # Synthetic summary output must not look like a real suite skip.
        self.output = contextlib.redirect_stdout(io.StringIO())
        self.output.__enter__()
        self.addCleanup(self.output.__exit__, None, None, None)

    def test_rust_live_skip_is_not_passed(self):
        selected = next(entry for entry in runner.ALL_TESTS if entry.id == 'pipeline.prompt-live')
        with patch.object(runner, 'run_process', return_value=(0, 'VERENU_LIVE_SKIP: no credential\ntest result: ok', 0.1, False)):
            self.assertEqual(runner.execute(selected, 'http://localhost:1').status, 'skipped')

    def test_native_prerequisites_match_the_owned_test_runners(self):
        self.assertEqual(runner.NativeCapabilityCheck().run().status, 'passed')

    def test_protocol_never_hides_process_failure_or_earlier_skip(self):
        selected = runner.entry('protocol', 'pipeline', 'Protocol', command=['unused'])
        with patch.object(runner, 'run_process', return_value=(1, 'VERENU_TEST_RESULT={"status":"skipped"}', 0.1, False)):
            self.assertEqual(runner.execute(selected, 'http://localhost:1').status, 'failed')
        _, payload = runner._parse_protocol('VERENU_TEST_RESULT={"status":"skipped"}\nVERENU_TEST_RESULT={"status":"passed"}')
        self.assertEqual(payload['status'], 'skipped')

    def test_optional_failure_still_fails(self):
        selected = runner.entry('optional', 'pipeline', 'Optional', required=False)
        self.assertEqual(runner.summary({'optional': runner.TestResult('failed')}, [selected], 0), 1)

    def test_until_pass_returns_success_after_later_clean_loop(self):
        selected = runner.select_tests(['preflight'], 'environment')
        outcomes = [
            {selected[0].id: runner.TestResult('failed', observed='first run failed')},
            {selected[0].id: runner.TestResult('passed')},
        ]
        with patch.object(runner, 'execute_plan', side_effect=outcomes):
            self.assertEqual(runner.main(['--test', 'environment', '--until-pass', '--loops', '2', '--no-json-report']), 0)

    def test_strict_skip_and_flake_fail(self):
        selected = runner.entry('required', 'pipeline', 'Required')
        for result in [runner.TestResult('skipped'), runner.TestResult('passed', regression_status='flaky')]:
            self.assertEqual(runner.summary({'required': result}, [selected], 0, strict=True), 1)

    def test_retry_keeps_original_failure(self):
        selected = runner.entry('retry', 'ui', 'Retry', retries=1)
        with patch.object(runner, 'execute', side_effect=[runner.TestResult('failed', observed='original bug'), runner.TestResult('passed')]):
            result = runner.run_with_retries(selected, 'http://localhost:1', False)
        self.assertEqual(result.previous_failures, ['original bug'])
        self.assertEqual(result.regression_status, 'flaky')

    def test_servers_allocate_ports_and_never_kill_listener(self):
        first, second = runner.ServerManager(), runner.ServerManager()
        self.assertNotEqual(first.port, runner.PORT)
        self.assertNotEqual(second.port, runner.PORT)
        with self.assertRaisesRegex(RuntimeError, 'forbidden'):
            runner.kill_port_owner(runner.PORT)

    def test_protocol_payload_is_removed_from_human_output(self):
        raw = 'before\ntest name ... VERENU_TEST_RESULT={"status":"failed","observed":"broken"}\nafter'
        output, payload = runner._parse_protocol(raw)
        self.assertEqual(output, "before\ntest name ...\nafter")
        self.assertEqual(payload["status"], "failed")

    def test_filter_matches_stable_id(self):
        selected = runner.select_tests(["accessibility"], "settings-focus")
        self.assertEqual([test.id for test in selected], ["accessibility.settings-focus"])

    def test_unknown_suite_is_rejected(self):
        args = type("Args", (), {"suite": "made-up", "profile": "fast"})()
        with self.assertRaisesRegex(ValueError, "Unknown suite"):
            runner.parse_suites(args)

    def test_json_report_contains_agent_fields(self):
        selected = runner.select_tests(["preflight"], "environment")
        result = runner.TestResult(
            "failed",
            expected="tools exist",
            observed="cargo missing",
            regression_area="environment",
            failure_kind="infrastructure",
            measurements={"cargo": False},
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            runner.write_json_report(path, "fast", ["preflight"], selected, {selected[0].id: result}, 0.1)
            payload = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(payload["schema_version"], 2)
        self.assertEqual(payload["required_failures"], ["preflight.environment"])
        test = payload["tests"][0]
        self.assertEqual(test["failure_kind"], "infrastructure")
        self.assertEqual(test["observed"], "cargo missing")
        self.assertEqual(test["measurements"], {"cargo": False})

    def test_junit_escapes_failure_attributes(self):
        selected = runner.select_tests(["preflight"], "environment")
        result = runner.TestResult("failed", observed='expected "quoted" value', output="bad <value>")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.xml"
            runner.write_junit_report(path, selected, {selected[0].id: result})
            root = ET.parse(path).getroot()
        self.assertEqual(root.find("testcase/failure").attrib["message"], 'expected "quoted" value')

    def test_loop_merge_keeps_failure_and_marks_flake(self):
        merged = runner.merge_loop_results(
            {"sample": runner.TestResult("passed", duration_s=0.1)},
            {"sample": runner.TestResult("failed", observed="broke", duration_s=0.2)},
        )
        self.assertEqual(merged["sample"].status, "failed")
        self.assertEqual(merged["sample"].regression_status, "flaky")
        self.assertAlmostEqual(merged["sample"].duration_s, 0.3)


if __name__ == "__main__":
    unittest.main()
