#!/bin/bash
# Read-only incident capture. Does not reload the app or stop the microphone.
set -euo pipefail
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'This diagnostic is for macOS.' >&2
  exit 1
fi
diagnostic_dir="${1:-${TMPDIR:-/tmp}/selah-live-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$diagnostic_dir"
chmod 700 "$diagnostic_dir"
sw_vers > "$diagnostic_dir/system.txt"
sysctl -n hw.memsize hw.ncpu >> "$diagnostic_dir/system.txt"
ps -axo pid,ppid,etime,%cpu,rss,command | awk '/[s]elah-app|[c]om.apple.WebKit\.(GPU|WebContent|Networking)/' > "$diagnostic_dir/processes.txt"
# Framework logs contain process state, not Selah's transcript event payloads.
/usr/bin/log show --last 30m --style compact --predicate \
  'process == "selah-app" AND subsystem BEGINSWITH "com.apple.WebKit"' \
  > "$diagnostic_dir/webkit.log" 2>&1
diagnostic_pid="$(pgrep -x selah-app | head -1 || true)"
if [[ -n "$diagnostic_pid" ]]; then
  sample "$diagnostic_pid" 3 5 -file "$diagnostic_dir/main-thread-sample.txt" > "$diagnostic_dir/sample-status.txt" 2>&1 || true
  vmmap -summary "$diagnostic_pid" > "$diagnostic_dir/main-memory.txt" 2>&1 || true
fi
diagnostic_log="$HOME/Library/Logs/com.kgu.selah/kwic.log"
if [[ -f "$diagnostic_log" ]]; then
  # Counts/state only. Do not copy course names, captions, emails or tokens.
  awk '/\[frontend-health\]/ { print }' "$diagnostic_log" | tail -120 > "$diagnostic_dir/frontend-health.txt"
fi
awk '/GPUProcessProxy::didBecomeUnresponsive|GPUProcessProxy::gpuProcessExited|WebPageProxy::processDidTerminate|didExceedMemoryLimit/ { print }' \
  "$diagnostic_dir/webkit.log" > "$diagnostic_dir/incidents.txt"
printf 'Saved diagnostic files to %s\n' "$diagnostic_dir"
cat "$diagnostic_dir/incidents.txt"
