#!/bin/bash
# AG4 probe: does the installed Claude Code honour a PreToolUse `defer`?
#
# Manual only (it runs `claude -p` four times on the signed-in account, about
# a cent with Haiku); not part of any gate. It makes a scratch folder whose
# project settings hold one PreToolUse hook on Bash. The hook logs each input
# and answers `defer`, except for the one call id written in `answer-id`,
# which gets the decision in `answer`. For allow, then deny, it checks:
#   1. a turn asking for one Bash call ends with stop_reason `tool_deferred`
#      and a `deferred_tool_use` naming Bash with an id, and the command did
#      not run;
#   2. `--resume <session>` fires the hook again for that same call id, keeps
#      the session id and is not an error; allow runs the command
#      (`out.txt`), deny does not and lists that id in `permission_denials`.
# Exit 0 only when every check holds. The scratch folder is kept and printed
# for inspection. Results of the 2026-11-18 run: docs/reference/interrupts.md,
# "Claude tool defer (AG4 probe)".
#
# Usage: scripts/probe-claude-defer.sh [model]   (default haiku)
set -u
model="${1:-haiku}"
work="$(mktemp -d "${TMPDIR:-/tmp}/claude-defer-probe.XXXXXX")"
probe="$work/probe"
mkdir -p "$probe/.claude"
cat > "$work/hook.sh" <<'HOOK'
#!/bin/bash
dir="$(dirname "$0")/probe"
input=$(cat)
printf '%s\n' "$input" >> "$dir/../hook-inputs.jsonl"
id=$(printf '%s' "$input" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("tool_use_id",""))')
d=defer
if [ -f "$dir/answer-id" ] && [ "$id" = "$(cat "$dir/answer-id")" ]; then d=$(cat "$dir/answer"); fi
printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"%s","permissionDecisionReason":"AG4 probe"}}\n' "$d"
HOOK
chmod +x "$work/hook.sh"
python3 - "$work/hook.sh" "$probe/.claude/settings.json" <<'PY'
import json, shlex, sys
hook = {"type": "command", "command": shlex.quote(sys.argv[1])}
json.dump({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [hook]}]}}, open(sys.argv[2], "w"))
PY

claude_p() { # claude_p <out file>, then the claude arguments
  local out="$1"; shift
  (cd "$probe" && claude -p "$@" --output-format json --setting-sources project \
    --permission-mode default --model "$model" < /dev/null > "$work/$out" 2> "$work/$out.err")
}

# check.py <phase> <first.json> [<resume.json> <answer> <ran>]: prints one line,
# exits 1 when a check fails.
check() {
  python3 - "$work" "$@" <<'PY'
import json, sys
work, phase, first = sys.argv[1], sys.argv[2], sys.argv[3]
def load(name):
    try:
        return json.load(open(f"{work}/{name}"))
    except Exception as error:
        sys.exit(f"{name}: no JSON result ({error})")
d = load(first)
deferred = d.get("deferred_tool_use") or {}
if phase == "first":
    print(f"turn 1: stop_reason={d.get('stop_reason')} deferred_tool_use={json.dumps(deferred)}")
    ok = d.get("stop_reason") == "tool_deferred" and deferred.get("name") == "Bash" and deferred.get("id")
    sys.exit(0 if ok else 1)
resume, answer, ran = sys.argv[4], sys.argv[5], sys.argv[6] == "yes"
r = load(resume)
hooks = [json.loads(line) for line in open(f"{work}/hook-inputs.jsonl")]
refired = any(h.get("tool_use_id") == deferred.get("id") and h.get("session_id") == d.get("session_id") for h in hooks[1:])
denied = [x.get("tool_use_id") for x in r.get("permission_denials") or []]
print(f"resume with {answer}: stop_reason={r.get('stop_reason')} is_error={r.get('is_error')} "
      f"same_session={r.get('session_id') == d.get('session_id')} hook_refired_same_id={refired} ran={ran} denials={denied}")
ok = not r.get("is_error") and r.get("session_id") == d.get("session_id") and refired
ok = ok and (ran and not denied if answer == "allow" else not ran and deferred.get("id") in denied)
sys.exit(0 if ok else 1)
PY
}

ask="Use the Bash tool exactly once to run: echo deferred-ok > out.txt . Then reply DONE."
ok=1
for answer in allow deny; do
  rm -f "$probe/answer" "$probe/answer-id" "$probe/out.txt" "$work/hook-inputs.jsonl"
  claude_p "first-$answer.json" "$ask"
  if ! check first "first-$answer.json" || [ -f "$probe/out.txt" ]; then
    echo "turn 1 did not defer ($answer run); not resuming"; ok=0; continue
  fi
  sid=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["session_id"])' "$work/first-$answer.json")
  python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["deferred_tool_use"]["id"])' "$work/first-$answer.json" > "$probe/answer-id"
  echo "$answer" > "$probe/answer"
  claude_p "resume-$answer.json" --resume "$sid"
  ran=no; [ -f "$probe/out.txt" ] && ran=yes
  check resume "first-$answer.json" "resume-$answer.json" "$answer" "$ran" || ok=0
done
echo "claude $(claude --version 2>/dev/null), model $model, scratch kept at $work"
[ "$ok" = 1 ] && echo "defer round trip: works" || { echo "defer round trip: FAILED"; exit 1; }
