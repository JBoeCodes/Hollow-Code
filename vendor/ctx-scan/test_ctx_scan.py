"""Tests for ctx-scan. Standard library only:

    /usr/bin/python3 -m unittest discover -s tools/ctx-scan -p 'test_*.py'

They read the same fixtures/expected.json and fixtures/cases.json that Hollow's
Rust tests read, so the two scanners can't drift apart.
"""

import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from importlib.machinery import SourceFileLoader

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "ctx-scan")
FIXTURES = os.path.join(HERE, "fixtures")
ctx = SourceFileLoader("ctx_scan", SCRIPT).load_module()


def load(name):
    with open(os.path.join(FIXTURES, name), "r", encoding="utf-8") as fh:
        return json.load(fh)


def case_text(case, alphabet):
    out = []
    for part in case["parts"]:
        if part.startswith("@seq:"):
            n = int(part[5:])
            out.append("".join(alphabet[(i * 7 + 3) % len(alphabet)] for i in range(n)))
        else:
            out.append(part)
    return "".join(out)


def run_cli(args):
    out, err = io.StringIO(), io.StringIO()
    code = ctx.run(args, out=out, err=err)
    return code, out.getvalue(), err.getvalue()


class SharedCorpus(unittest.TestCase):
    """The fixture files plus the generated 6 MB file, as expected.json says."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="ctx-scan-test-")
        cls.root = os.path.join(cls.tmp, "fixtures")
        shutil.copytree(FIXTURES, cls.root, ignore=shutil.ignore_patterns("*.json"))
        cls.expected = load("expected.json")
        for gen in cls.expected["generated"]:
            path = os.path.join(cls.root, gen["path"])
            os.makedirs(os.path.dirname(path), exist_ok=True)
            line = b"plain text line for the size limit test\n"
            with open(path, "wb") as fh:
                fh.write(line * (gen["bytes"] // len(line)) + b"x" * (gen["bytes"] % len(line)))
            assert os.path.getsize(path) == gen["bytes"]

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def scan(self):
        code, out, _ = run_cli(["--no-config", "--all-files", "--format", "json", "--layer",
                                self.expected["layer"], self.root])
        return code, json.loads(out)

    def test_each_fixture_reports_exactly_what_expected_json_says(self):
        code, report = self.scan()
        got = {}
        for f in report["findings"]:
            got.setdefault(f["path"], []).append({"line": f["line"], "rule": f["rule"], "severity": f["severity"]})
        want = dict(self.expected["files"])
        for gen in self.expected["generated"]:
            want[gen["path"]] = gen["expect"]
        for path, findings in want.items():
            self.assertEqual(sorted(got.get(path, []), key=json.dumps), sorted(findings, key=json.dumps), path)
        self.assertEqual(set(got) - set(want), set(), "findings in files expected.json doesn't list")
        self.assertEqual(code, 1)

    def test_planted_key_ssn_macbook_and_big_file_each_fail_alone(self):
        for rel in ("dirty/planted-key.md", "dirty/new-hire.md", "dirty/machine.md", "generated/big.md"):
            code, _, _ = run_cli(["--no-config", "--layer", "org", os.path.join(self.root, rel)])
            self.assertEqual(code, 1, rel)

    def test_clean_files_pass(self):
        code, out, _ = run_cli(["--no-config", "--all-files", "--layer", "org", os.path.join(self.root, "clean")])
        self.assertEqual(code, 0, out)
        self.assertIn("PASS: 3 files, 0 blocking, 0 warnings", out)

    def test_big_file_is_reported_and_only_its_first_part_is_read(self):
        rules = ctx.Rules(ctx.default_rules_path())
        scanner = ctx.Scanner(rules, "org")
        limit = rules.max_file_bytes
        key = "sk-" + "ant-api03-" + "Q7x" * 10
        pad = b"plain text line for the size limit test\n" * (limit // 40)
        early = os.path.join(self.tmp, "early.md")
        with open(early, "wb") as fh:
            fh.write(b"KEY " + key.encode() + b"\n" + pad + b"x" * 1024)
        late = os.path.join(self.tmp, "late.md")
        with open(late, "wb") as fh:
            fh.write(pad + b"y" * (limit - len(pad) + 10) + b"\nKEY " + key.encode() + b"\n")
        self.assertEqual(sorted(f.rule for f in scanner.scan_file(early, "early.md")),
                         ["file.too_large", "secret.anthropic"])
        self.assertEqual([f.rule for f in scanner.scan_file(late, "late.md")], ["file.too_large"])

    def test_output_never_contains_the_secret_value(self):
        with open(os.path.join(self.root, "dirty/planted-key.md"), encoding="utf-8") as fh:
            key_line = fh.read().splitlines()[4]
        value = key_line.split("=", 1)[1]
        for fmt in ("text", "json"):
            code, out, err = run_cli(["--no-config", "--all-files", "--format", fmt, "--layer", "org", self.root])
            self.assertEqual(code, 1)
            self.assertNotIn(value, out + err)
            self.assertNotIn(value[:16], out + err)
            self.assertNotIn("123-45-6789", out + err)


class SharedCases(unittest.TestCase):
    """Line cases from cases.json, built at run time."""

    def test_cases(self):
        data = load("cases.json")
        rules = ctx.Rules(ctx.default_rules_path())
        scanners = {}
        for case in data["cases"]:
            layer = case["layer"]
            scanner = scanners.setdefault(layer, ctx.Scanner(rules, layer))
            text = case_text(case, data["alphabet"])
            got = sorted(f.rule for f in scanner.scan_text(text))
            self.assertEqual(got, sorted(case["expect"]), case["name"])

    def test_severity_follows_layer(self):
        rules = ctx.Rules(ctx.default_rules_path())
        text = "Runs on the MacBook Air."
        self.assertEqual([f.severity for f in ctx.Scanner(rules, "org").scan_text(text)], ["block"])
        self.assertEqual([f.severity for f in ctx.Scanner(rules, "project").scan_text(text)], ["warn"])
        self.assertEqual(ctx.Scanner(rules, "home").scan_text(text), [])


class Rules(unittest.TestCase):
    def setUp(self):
        self.rules = ctx.Rules(ctx.default_rules_path())

    def test_every_pattern_stays_in_the_shared_syntax(self):
        # Hollow compiles these with Rust's regex crate: no lookaround,
        # no backreferences, inline flags only at the start or scoped.
        import re
        banned = ["(?=", "(?!", "(?<=", "(?<!", "\\1", "\\2", "(?P=", "*+", "++", "?+"]
        with open(ctx.default_rules_path(), encoding="utf-8") as fh:
            raw = json.load(fh)
        for rule in raw["rules"]:
            for key in ("pattern", "exclude"):
                pattern = rule.get(key)
                if not pattern:
                    continue
                for token in banned:
                    self.assertNotIn(token, pattern, "%s %s" % (rule["id"], token))
                self.assertNotIn("(?i)", pattern[1:], "%s: (?i) only at the start" % rule["id"])
                re.compile(pattern)

    def test_every_repeat_is_bounded(self):
        import re
        with open(ctx.default_rules_path(), encoding="utf-8") as fh:
            raw = json.load(fh)
        for rule in raw["rules"]:
            for key in ("pattern", "exclude"):
                pattern = rule.get(key) or ""
                # `x*`, `x+` and `{n,}` repeat without a bound.
                stripped = re.sub(r"\\.", "", pattern)
                stripped = re.sub(r"\[[^\]]*\]", "C", stripped)
                self.assertNotRegex(stripped, r"[^\\][*+]|\{[0-9]+,\}", "%s has an unbounded repeat" % rule["id"])

    def test_adversarial_lines_finish_fast(self):
        scanner = ctx.Scanner(self.rules, "org")
        bad = [
            "token" * 200000,
            "password=" + "a1" * 300000,
            "-----BEGIN " + "A " * 300000,
            "sk-ant-" * 100000,
            "x://" + "a" * 100000 + ":" + "b" * 100000,
            "bearer " + " " * 200000 + "x",
            "eyJ" + "a" * 100000 + ".eyJ" + "a" * 100000,
            ("1" * 3 + "-") * 100000,
        ]
        for text in bad:
            start = time.time()
            scanner.scan_text(text)
            self.assertLess(time.time() - start, 5.0, text[:20])

    def test_rule_ids_unique_and_layers_known(self):
        layers = set(self.rules.layers) | {"*"}
        for rule in self.rules.rules:
            self.assertTrue(set(rule.severity) <= layers, rule.id)
            self.assertIn(rule.group, self.rules.groups, rule.id)


class Cli(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ctx-scan-cli-")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write(self, rel, text):
        path = os.path.join(self.tmp, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return path

    def test_exit_codes(self):
        self.write("a.md", "fine\n")
        self.assertEqual(run_cli(["--no-config", self.tmp])[0], 0)
        self.assertEqual(run_cli(["--no-config", os.path.join(self.tmp, "missing")])[0], 2)
        self.assertEqual(run_cli(["--no-config", "--layer", "nope", self.tmp])[0], 2)
        self.assertEqual(run_cli(["--rules", os.path.join(self.tmp, "a.md"), self.tmp])[0], 2)

    def test_warnings_pass_unless_fail_on_warn(self):
        self.write("a.md", "Runs on the MacBook Air.\n")
        self.assertEqual(run_cli(["--no-config", "--layer", "project", self.tmp])[0], 0)
        self.assertEqual(run_cli(["--no-config", "--layer", "project", "--fail-on", "warn", self.tmp])[0], 1)

    def test_config_sets_layer_fail_on_disable_and_allow(self):
        self.write("notes.md", "Runs on the MacBook Air.\nrun it through hollow-slot\n")
        self.write("docs/example.md", "Edit ~/.claude/settings.json.\n")
        self.write(".context-scan.yml",
                   "# test config\nlayer: org\nfail_on: warn\ndisable:\n  - machine.hollow_slot\n"
                   "allow:\n  - \"notes.md machine.macbook the doc is about the scanner\"\n"
                   "  - docs/* machine portable examples\n")
        code, out, _ = run_cli(["--all-files", self.tmp])
        self.assertEqual(code, 0, out)
        self.assertIn("layer org", out)

    def test_config_cannot_disable_secret_rules(self):
        self.write("a.md", "fine\n")
        self.write(".context-scan.yml", "disable: [secret.anthropic]\n")
        code, _, err = run_cli(["--all-files", self.tmp])
        self.assertEqual(code, 2)
        self.assertIn("can't be disabled", err)
        self.write(".context-scan.yml", "disable: [secret]\n")
        self.assertEqual(run_cli(["--all-files", self.tmp])[0], 2)

    def test_personal_terms_from_own_file(self):
        self.write("a.md", "Ask Pat Example before merging.\n")
        personal = self.write("personal.json", json.dumps({"names": ["Pat Example"], "handles": ["patex"]}))
        code, out, _ = run_cli(["--no-config", "--layer", "org", "--personal", personal, "--fail-on", "warn",
                                os.path.join(self.tmp, "a.md")])
        self.assertEqual(code, 1)
        self.assertIn("personal.owner_term", out)
        self.assertNotIn("Pat Example", out)

    def test_git_mode_skips_ignored_files_and_scans_untracked(self):
        if shutil.which("git") is None:
            self.skipTest("no git")
        subprocess.run(["git", "init", "-q", self.tmp], check=True)
        self.write(".gitignore", "secret-notes.md\n")
        self.write("secret-notes.md", "Runs on the MacBook Air.\n")
        self.write("tracked.md", "fine\n")
        code, out, _ = run_cli(["--no-config", "--layer", "org", self.tmp])
        self.assertEqual(code, 0, out)
        self.write("new.md", "Runs on the MacBook Air.\n")
        self.assertEqual(run_cli(["--no-config", "--layer", "org", self.tmp])[0], 1)

    def test_binary_and_utf16_files_are_read_without_their_nul_bytes(self):
        path = os.path.join(self.tmp, "dump.db")
        with open(path, "wb") as fh:
            fh.write(b"\x00\x01MacBook")
        code, out, _ = run_cli(["--no-config", "--all-files", "--layer", "org", self.tmp])
        self.assertEqual(code, 1)
        self.assertIn("file.data_type", out)
        self.assertIn("machine.macbook", out)
        os.remove(path)
        with open(os.path.join(self.tmp, "notes.txt"), "wb") as fh:
            fh.write(("KEY sk-" + "ant-api03-" + "Q7x" * 10 + "\n").encode("utf-16"))
        code, out, _ = run_cli(["--no-config", "--all-files", "--layer", "org", self.tmp])
        self.assertEqual(code, 1, out)
        self.assertIn("secret.anthropic", out)

    def test_config_allow_rules(self):
        self.write("a.md", "fine\n")
        for bad in ('allow: ["* * everything"]\n', 'allow: ["* secret reason"]\n',
                    'allow: ["docs/* secret.anthropic reason"]\n', 'allow: ["a.md nope.rule reason"]\n',
                    "disable: [file]\n", "disable: [file.too_large]\n", "disable: ['*']\n",
                    "disable: [pii]\n", "disable: [pii.ssn]\n", 'allow: ["docs/* pii.ssn reason"]\n',
                    'allow: ["docs/* file.too_large reason"]\n'):
            self.write(".context-scan.yml", bad)
            self.assertEqual(run_cli(["--all-files", self.tmp])[0], 2, bad)
        key_file = self.write("docs/key.md", "KEY sk-" + "ant-api03-" + "Q7x" * 10 + "\n")
        self.write(".context-scan.yml", 'allow: ["docs/key.md secret.anthropic fabricated fixture"]\n')
        self.assertEqual(run_cli(["--all-files", self.tmp])[0], 0)
        self.assertTrue(os.path.exists(key_file))
        # A group allow by glob never covers a protected rule.
        self.write("docs/hire.md", "SSN 123-45-6789\n")
        self.write(".context-scan.yml", 'allow: ["docs/key.md secret.anthropic fabricated fixture", "docs/* pii reason"]\n')
        code, out, _ = run_cli(["--all-files", self.tmp])
        self.assertEqual(code, 1)
        self.assertIn("pii.ssn", out)
        self.write("disable-ok.yml", "")
        self.write(".context-scan.yml", "disable: [pii.salary_or_hr]\n" + 'allow: ["docs/key.md secret.anthropic f", "docs/hire.md pii.ssn fabricated"]\n')
        self.assertEqual(run_cli(["--all-files", self.tmp])[0], 0)

    def test_unexpected_failures_exit_2_without_content(self):
        self.write("a.md", "fine\n")
        with open(os.path.join(self.tmp, ".context-scan.yml"), "wb") as fh:
            fh.write(b"layer: \xff\xfe org\n")
        code, out, err = run_cli(["--all-files", self.tmp])
        self.assertEqual(code, 2)
        self.assertIn("UnicodeDecodeError", err)
        personal = self.write("personal.json", "[1, 2]")
        self.assertEqual(run_cli(["--no-config", "--personal", personal, self.tmp])[0], 2)

    def test_symlinks_are_not_followed(self):
        target = self.write("outside/key.md", "Runs on the MacBook Air.\n")
        os.makedirs(os.path.join(self.tmp, "repo"))
        os.symlink(target, os.path.join(self.tmp, "repo", "link.md"))
        self.assertEqual(run_cli(["--no-config", "--all-files", "--layer", "org",
                                  os.path.join(self.tmp, "repo")])[0], 0)

    def test_script_runs_as_a_program(self):
        proc = subprocess.run([sys.executable, SCRIPT, "--version"], stdout=subprocess.PIPE, check=True)
        self.assertIn(b"ctx-scan", proc.stdout)


if __name__ == "__main__":
    unittest.main()
