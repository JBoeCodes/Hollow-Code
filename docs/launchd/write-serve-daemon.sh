#!/bin/sh
# Writes the hollow serve LaunchDaemon plist for this user and prints the sudo
# steps that install and load it (M1 PR 4; runbook: docs/HEADLESS_SERVE.md).
#
#   sh docs/launchd/write-serve-daemon.sh [--base <folder>] [--binary <path>]
#        [--data-dir <folder>] [--config <file>] [--out <file>]
#
# Run it as the user the daemon runs as (on the mini: runner). It only
# writes files in that user's home: the filled-in plist (default
# <base>/launchd/com.hollow.app.serve.plist), the data folder (0700) and
# <base>/tmp. It never runs sudo, launchctl or anything outside that home;
# the person runs the printed commands (they also make the root-owned log).
# Defaults: base ~/hollow-serve, binary <base>/bin/hollow, data <base>/data,
# config <base>/config.json.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
template="$here/com.hollow.app.serve.plist"
user=$(id -un)
# The account's own home, not whatever $HOME says (sudo -u, a changed HOME).
home=$(dscl . -read "/Users/$user" NFSHomeDirectory 2>/dev/null | sed -n 's/^NFSHomeDirectory: //p')
if [ -z "$home" ]; then
  echo "could not read the home folder of $user from the directory" >&2
  exit 2
fi
if [ "$home" != "$HOME" ]; then
  echo "\$HOME ($HOME) is not $user's home ($home); run this as $user from its own login" >&2
  exit 2
fi
base="$home/hollow-serve"
binary=""
data=""
config=""
out=""

while [ $# -gt 0 ]; do
  case "$1" in
    --base) base=$2; shift 2 ;;
    --binary) binary=$2; shift 2 ;;
    --data-dir) data=$2; shift 2 ;;
    --config) config=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
binary=${binary:-$base/bin/hollow}
data=${data:-$base/data}
config=${config:-$base/config.json}
out=${out:-$base/launchd/com.hollow.app.serve.plist}

for path in "$base" "$binary" "$data" "$config" "$out"; do
  case "$path" in
    /*) ;;
    *) echo "paths must be absolute: $path" >&2; exit 2 ;;
  esac
  case "$path" in
    *[\&\|\<\>\"\'\\]*) printf '%s\n' "path has a character the plist can't take as is: $path" >&2; exit 2 ;;
  esac
  case "$path" in
    *"
"*) echo "a path has a line break" >&2; exit 2 ;;
  esac
done
if [ "$user" = root ]; then
  echo "run this as the user the daemon runs as, not root" >&2
  exit 2
fi

mkdir -p "$(dirname "$out")" "$data" "$base/tmp"
chmod 0700 "$data" "$base/tmp"
sed -e "s|@USER@|$user|g" -e "s|@HOME@|$home|g" -e "s|@BINARY@|$binary|g" \
  -e "s|@DATA_DIR@|$data|g" -e "s|@CONFIG@|$config|g" -e "s|@BASE@|$base|g" \
  "$template" > "$out.tmp"
plutil -lint "$out.tmp" >/dev/null
mv "$out.tmp" "$out"
chmod 0644 "$out"
echo "Wrote $out (plutil: OK)."

missing=""
[ -x "$binary" ] || missing="$missing
  - the serve binary $binary (copy Hollow.app/Contents/MacOS/hollow from a release build)"
[ -f "$config" ] || missing="$missing
  - $config, e.g. {\"vaultPath\": \"/Users/$user/<vault clone>\"}"
if [ -n "$missing" ]; then
  echo "Not there yet (the daemon exits 78 or can't start without them):$missing"
fi

cat <<EOF

To install and start it (an admin runs these; nothing above did):

  sudo touch /var/log/hollow-serve.log
  sudo chown $user:staff /var/log/hollow-serve.log
  sudo chmod 600 /var/log/hollow-serve.log
  sudo install -m 0644 -o root -g wheel "$out" /Library/LaunchDaemons/com.hollow.app.serve.plist
  sudo launchctl bootstrap system /Library/LaunchDaemons/com.hollow.app.serve.plist
  sudo launchctl print system/com.hollow.app.serve | grep -E 'state|pid|last exit'
  sudo tail -f /var/log/hollow-serve.log

Restart after replacing the binary:  sudo launchctl kickstart -k system/com.hollow.app.serve
Stop and remove:                     sudo launchctl bootout system/com.hollow.app.serve
                                     sudo rm /Library/LaunchDaemons/com.hollow.app.serve.plist
EOF
