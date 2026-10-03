"""Independent outcome grader. Kept outside each agent's disposable checkout."""
import importlib.util
import sys
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("evaluated_runner", Path(sys.argv[1]) / "tests/OnePyFone.py")
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)
entry = runner.entry("probe", "pipeline", "Provider", required=False, command=["unused"], retries=1)
with patch.object(runner, "run_process", return_value=(0, "VERENU_LIVE_SKIP: absent credential", 0.01, False)):
    assert runner.execute(entry, "http://localhost:1").status == "skipped"
assert runner.summary({entry.id: runner.TestResult("failed")}, [entry], 0) == 1
assert runner.summary({entry.id: runner.TestResult("skipped")}, [entry], 0, strict=True) == 1
with patch.object(runner, "execute", side_effect=[runner.TestResult("failed", observed="original"), runner.TestResult("passed")]):
    result = runner.run_with_retries(entry, "http://localhost:1", False)
assert result.regression_status == "flaky"
assert result.previous_failures == ["original"]
assert runner.summary({entry.id: result}, [entry], 0, strict=True) == 1
print("[SUCCESS] Independent acceptance checks passed")
