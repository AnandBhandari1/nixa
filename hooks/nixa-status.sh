#!/bin/sh
drain() {
  cat >/dev/null 2>&1
}
[ -n "$NIXA_TERM_ID" ] && [ -n "$NIXA_ENGINE_SOCKET" ] && [ "$TERM_PROGRAM" = "nixa" ] || { drain; exit 0; }
agent=""
if [ "$1" = "--resume" ]; then
  [ $# -ge 2 ] || { drain; exit 0; }
  agent="$2"
  shift 2
fi
[ -n "$1" ] || { drain; exit 0; }
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$NIXA_BIN"
if [ -z "$bin" ]; then
  for candidate in "$root/../MacOS/nixa" "$root/target/gpui/debug/nixa" "$root/target/gpui/release/nixa"; do
    if [ -x "$candidate" ]; then
      bin="$candidate"
      break
    fi
  done
fi
[ -n "$bin" ] || { drain; exit 0; }
if [ -n "$agent" ]; then
  "$bin" hooks session "$agent" >/dev/null 2>&1 || drain
else
  drain
fi
"$bin" term status "$@" >/dev/null 2>&1
exit 0
