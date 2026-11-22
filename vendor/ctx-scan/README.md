# ctx-scan

Checks a context repo (or any folder) for things that must not be shared: secrets, one computer's facts, one person's facts, employee data, data files and files over the size limit. Python standard library only, so it runs under macOS's own `/usr/bin/python3` (3.9) with nothing to install, and without Hollow.

It never prints a matched value. A finding names the file, line, column and rule.

## Run it

```
tools/ctx-scan/ctx-scan                  # this folder; layer from .context-scan.yml, else project
tools/ctx-scan/ctx-scan --layer org .    # say the layer yourself
tools/ctx-scan/ctx-scan --list-rules     # every rule and its severity per layer
```

In a git repo it scans what git would commit (tracked files plus untracked files that aren't ignored). `--all-files` walks the folder instead. Symlinks are not followed. Binary and UTF-16 files are read with their NUL bytes dropped, so a key inside one is still found. A file over the size limit is reported and only its first part (the limit's worth) is read.

| Exit code | Meaning |
|---|---|
| 0 | Nothing at or above the fail level |
| 1 | Findings at the fail level (default: `block`; `--fail-on warn` fails on warnings too) |
| 2 | Usage or configuration error: bad rules file, unknown layer, missing path, bad config, or any other failure (only the error's type is printed) |

`--format json` prints `{version, layer, fail_on, files, status, findings[], summary}`; each finding has `path`, `line` (0 for a path or size finding), `column`, `rule`, `group`, `severity` and `message`.

## Layers

| Layer | Use for | Checked |
|---|---|---|
| `org` | `org-context` | everything: secrets, computer facts, personal facts, employee data, data files, size |
| `project` | a project's own repo | secrets and SSNs block; computer and personal facts warn; size warns |
| `personal`, `home` | your `personal-context` repo | secrets, SSNs, data files, size (Acme facts are allowed there) |
| `vault` | a Hollow vault repo | secrets, SSNs, size |

The question behind the computer and personal rules, asked in this order: would this be false on a colleague's computer (home)? Is it only about how you work (personal)? Is it true for everyone on one project (project)? For everyone in your organization (org)?

## When it's wrong

- One line: add `context-scan: allow <rule> <reason>` at the end of that line, or on the line above as a line of its own (an optional comment opener such as `#`, `//` or `<!--`, then the allow). An allow at the end of a line of code covers only that line. `<rule>` is a rule id (`machine.macbook`) or a group (`machine`). The reason is required.
- Whole files: `.context-scan.yml` at the repo root:

```
layer: org
fail_on: block
disable:
  - machine.hollow_slot           # never secret, scan or file rules
allow:
  - "docs/examples/* machine portable example paths"   # <glob> <rule or group> <reason>
  - "docs/fixtures/fake-key.md secret.anthropic fabricated test key"
```

A committed config can't switch secret, scan, file or PII rules off. Secret rules, `scan.unreadable`, `file.too_large` and `pii.ssn` can be allowed only by their own id for one exact path (no `*`, `?` or `[`); a group allow by glob never covers them. An allow entry must name a rule or group, never `*`, so a reviewer sees each exception.

Your own name and handles: put them in `~/.config/ctx-scan/personal.json` (`{"names": [...], "handles": [...], "emails": [...]}`), or pass `--personal FILE` or set `CTX_SCAN_PERSONAL`. Never commit that file. Matches warn in org and project repos.

## The rules file

`scan-rules.json` is the one list of patterns. Hollow vendors a copy and reads it with Rust's `regex` crate, so patterns stay in the syntax both share: no lookaround, no backreferences, `(?i)` only at the start or scoped as `(?i:...)`, and every repeat bounded (no `*`, `+` or `{n,}`), which also keeps Python's matcher from backtracking for long. The tests enforce all of that.

A rule has `id`, `group`, `target` (`line`, `path`, `size`, `personal_terms` or `unreadable`), `pattern`, `message` and `severity` per layer (`*` for all; a missing layer means the rule is off there). A rule with `value_group` checks the captured value: `letter_and_digit`, `not_placeholder` (none of `placeholder_words`), `min_entropy` (Shannon bits per character) and `not_reference` (not a path, URL, e-mail address or `process.env`-style lookup). That is how `replace-me-with-a-real-32-byte-secret` and `${DB_PASSWORD}` pass while a real key fails. When two rules match the same span on a line, only the first (in file order) is reported.

## Tests

```
/usr/bin/python3 -m unittest discover -s tools/ctx-scan -p 'test_*.py'
```

`fixtures/` is the shared corpus: clean files that must pass, dirty files (a planted fake `sk-ant-` key, a MacBook line, an SSN shape, a csv) whose findings `fixtures/expected.json` lists, and a 6 MB file the tests write at run time. `fixtures/cases.json` holds line cases built at run time, so no key-shaped string sits in the repo. Hollow's Rust tests read the same two JSON files.

## Changing the rules

By pull request here, with a case in `fixtures/cases.json` for every new or changed pattern. Then re-vendor into Hollow (`scripts/vendor-ctx-scan.sh <this checkout>` in the Hollow repo), whose tests check the copy's hash.
