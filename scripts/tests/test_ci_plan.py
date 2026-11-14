"""scripts/ci/plan.py: which suites a change selects, and the runners they reserve."""

import json
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

import plan  # noqa: E402


def files(*paths, patch=""):
    return [{"filename": p, "patch": patch} for p in paths]


def selected(suites):
    return {name for name, on in suites.items() if on}


class PlanTests(unittest.TestCase):
    def test_docs_only_runs_nothing(self):
        self.assertEqual(selected(plan.plan(files("README.md", "docs/ci.md"), "fast")), set())

    def test_ui_change_on_a_pull_request_stays_on_linux(self):
        suites = plan.plan(files("crates/ui/src/shell.rs", patch="+let width = 3;"), "fast")
        self.assertEqual(selected(suites), {"core", "ui"})

    def test_ui_change_on_main_stays_on_linux(self):
        suites = plan.plan(files("crates/ui/src/shell.rs", patch="+let width = 3;"), "full")
        self.assertEqual(selected(suites), {"core", "ui"})

    def test_lockfile_runs_every_rust_suite(self):
        suites = plan.plan(files("Cargo.lock"), "fast")
        self.assertEqual(selected(suites), {"core", "ui", "zgx"})

    def test_renamed_from_counts(self):
        f = [{"filename": "docs/x.md", "previous_filename": "apps/zgx/ui/app.luau", "patch": ""}]
        self.assertTrue(plan.plan(f, "fast")["zgx"])

    def test_ci_changes_run_everything(self):
        daytime = set(plan.SUITE_JOBS) - set(plan.NIGHTLY_SUITES)
        self.assertEqual(selected(plan.plan(files("scripts/ci/admit.py"), "fast")), daytime)
        self.assertEqual(selected(plan.plan(None, "fast")), daytime)
        self.assertTrue(all(plan.plan(None, "full", nightly=True).values()))

    def test_voice_runs_only_nightly(self):
        change = files("crates/audio/src/aec.rs", patch="+let x = 1;")
        self.assertFalse(plan.plan(change, "fast")["voice"])
        self.assertFalse(plan.plan(change, "full")["voice"])
        self.assertTrue(plan.plan(change, "full", nightly=True)["voice"])
        # On a pull request only its own files select it, not any Rust change.
        lock = files("Cargo.lock")
        self.assertFalse(plan.plan(lock, "fast", nightly=True)["voice"])
        self.assertTrue(plan.plan(lock, "full", nightly=True)["voice"])

    def test_zgx_runs_for_its_app_and_the_crates_it_builds(self):
        zgx = ["crates/engine/**", "crates/proto/**"]
        self.assertEqual(selected(plan.plan(files("apps/zgx/ui/app.luau"), "fast", zgx=zgx)), {"zgx"})
        self.assertEqual(selected(plan.plan(files("apps/zgx/plugins-sdk/zend.luau"), "fast", zgx=zgx)), {"zgx"})
        self.assertIn("zgx", selected(plan.plan(files("crates/engine/src/plugins/mod.rs"), "fast", zgx=zgx)))
        self.assertNotIn("zgx", selected(plan.plan(files("crates/ui/src/lib.rs"), "fast", zgx=zgx)))
        # Without the workspace's closure: the static list.
        self.assertIn("zgx", selected(plan.plan(files("crates/visual/src/lib.rs"), "fast")))

    def test_installer(self):
        self.assertEqual(selected(plan.plan(files("edge/src/install.sh"), "fast")), {"installer"})
        self.assertEqual(selected(plan.plan(files("edge/src/coordinator.ts"), "fast")), set())

    def test_version_bumps_select_nothing(self):
        bump = [{"filename": "Cargo.toml", "patch": '@@ -37 +37 @@\n-version = "0.2.126"\n+version = "0.2.127"'},
                {"filename": "Cargo.lock", "patch": ' name = "zend-ui"\n-version = "0.2.126"\n+version = "0.2.127"'}]
        self.assertEqual(selected(plan.plan(bump, "full")), set())
        dep = [{"filename": "Cargo.lock",
                "patch": '-version = "1.0.1"\n+version = "1.0.2"\n-checksum = "aa"\n+checksum = "bb"'}]
        self.assertTrue(plan.plan(dep, "fast")["core"])

    def test_steps_for_their_own_files(self):
        change = files("crates/ui/src/shell.rs", patch="+let a = 1;")
        self.assertEqual(plan.gated(change, "fast", plan.plan(change, "fast")), ({}, {}))
        browser = files("crates/ui/src/browser/view.rs", patch='+let a = 1;')
        off, flags = plan.gated(browser, "fast", plan.plan(browser, "fast"))
        self.assertEqual(flags, {})  # the Linux browser fixture runs nightly
        off, flags = plan.gated(browser, "fast", plan.plan(browser, "fast"), nightly=True)
        self.assertEqual(flags, {"ui": ["browser"]})
        self.assertEqual(plan.gated(change, "full", plan.plan(change, "full")), ({}, {}))
        nightly = plan.gated(change, "full", plan.plan(change, "full", nightly=True), nightly=True)
        self.assertEqual(nightly[1], {"ui": ["browser"]})

    def test_reservation(self):
        suites = plan.plan(files("Cargo.lock"), "full", nightly=True)
        counts = plan.reservation(plan.jobs_to_run(suites), "pull_request", "refs/pull/1/merge")
        # result + core + ui + voice offline + zgx-linux; voice runtime
        self.assertEqual(counts, {"linux": 5, "windows": 1, "macos": 0})
        self.assertEqual(plan.reservation(plan.jobs_to_run(suites), "push", "refs/heads/main"), counts)
        self.assertEqual(plan.format_reservation(counts), "linux=5 windows=1 macos=0")
        daytime = plan.reservation(plan.jobs_to_run(plan.plan(files("Cargo.lock"), "full")), "push",
                                   "refs/heads/main")
        self.assertEqual(daytime, {"linux": 4, "windows": 0, "macos": 0})  # no voice
        self.assertEqual(plan.reservation(plan.jobs_to_run({s: False for s in plan.SUITE_JOBS}), "push",
                                          "refs/heads/main"),
                         {"linux": 1, "windows": 0, "macos": 0})


class WorkflowConsistencyTests(unittest.TestCase):
    """SUITE_JOBS must match the jobs the reusable workflows start."""

    # suite -> (workflow, jobs of that workflow the suite turns on)
    SUITES = {
        "core": ("ui-tests.yml", ["core-tests"]),
        "ui": ("ui-tests.yml", ["ui-tests"]),
        "installer": ("linux-installer.yml", ["desktop-entry"]),
        "voice": ("voice-tests.yml", ["offline-linux", "windows-runtime"]),
        "zgx": ("zgx-tests.yml", ["zgx-linux"]),
    }

    def test_job_runners_match(self):
        """Each suite's jobs and runners, and the name each records its pass under."""
        try:
            import yaml
        except ImportError:
            self.skipTest("PyYAML not installed")
        workflows = ROOT / ".github" / "workflows"
        for suite, (workflow, jobs) in self.SUITES.items():
            text = (workflows / workflow).read_text()
            spec = yaml.safe_load(text)
            runners = []
            for job in jobs:
                runs_on = spec["jobs"][job]["runs-on"]
                if "matrix.os" in runs_on:
                    default = spec.get("on", spec.get(True))["workflow_call"]["inputs"]["native-oses"]["default"]
                    labels = [(f"{job}-{os}", os) for os in json.loads(default)]
                else:
                    labels = [(job, runs_on)]
                for marker, label in labels:
                    runner = "windows" if "windows" in label else "macos" if "macos" in label else "linux"
                    runners.append((marker, runner))
                    recorded = marker if "matrix.os" not in runs_on else f"{job}-${{{{ matrix.os }}}}"
                    self.assertIn(f"key: ${{{{ inputs.pass-key }}}}-{recorded}", text, (suite, marker))
            self.assertEqual(sorted(runners), sorted(plan.SUITE_JOBS[suite]), suite)

    def test_every_suite_is_called(self):
        try:
            import yaml
        except ImportError:
            self.skipTest("PyYAML not installed")
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        for suite in plan.SUITE_JOBS:
            self.assertIn(f"suites).{suite}", ci, suite)
            self.assertIn(f"keys).{suite} ||", ci, suite)


if __name__ == "__main__":
    unittest.main()
