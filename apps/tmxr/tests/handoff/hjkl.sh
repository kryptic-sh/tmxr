#!/usr/bin/env bash
# A real hjkl in a real tmxr pane: C-h / C-l first move between hjkl's own
# windows, then, at hjkl's edge, hjkl hands the key to tmxr (`$TMXR`) and
# tmxr moves to the pane on the left. Needs `tmxr` and `hjkl` on PATH.
set -euo pipefail

tmp=$(mktemp -d)
if command -v cygpath >/dev/null; then
  tmp=$(cygpath -m "$tmp")
fi
# An isolated server: its own socket, config and data, nothing restored.
export TMXR_TMPDIR="$tmp/sock" XDG_DATA_HOME="$tmp/data" XDG_STATE_HOME="$tmp/state"
printf '[resurrect]\nrestore-on-start = false\nauto-save-minutes = 0\n' >"$tmp/tmxr.toml"
t() { tmxr -L handoff-ci -f "$tmp/tmxr.toml" "$@"; }
trap 't kill-server >/dev/null 2>&1 || true' EXIT

# Wait up to 20 s for `#{format}` in the session to print `want`.
wait_for() {
  local format=$1 want=$2 got=""
  for _ in $(seq 1 100); do
    got=$(t display-message -p -t h "$format" | tr -d '\r')
    [ "$got" = "$want" ] && return 0
    sleep 0.2
  done
  echo "timed out: $format is '$got', want '$want'" >&2
  t capture-pane -p -t h >&2 || true
  exit 1
}

t new-session -d -s h -x 160 -y 40
t split-window -h -t h -- hjkl
wait_for '#{pane_index} #{pane_current_command}' '1 hjkl'
sleep 2 # hjkl has started; let it draw and read keys

# Two hjkl windows side by side, focus in the left one.
t send-keys -t h Escape ':vsplit' Enter
sleep 1
t send-keys -t h C-l
sleep 1
wait_for '#{pane_index}' '1' # moved inside hjkl, still in its pane
t send-keys -t h C-h
sleep 1
wait_for '#{pane_index}' '1' # back to hjkl's left window
t send-keys -t h C-h
wait_for '#{pane_index}' '0' # at hjkl's edge: tmxr moved
echo "hjkl handoff: ok"
