#!/usr/bin/env bash
#
# The pill's three states, watched without a microphone.
#
# The presenter reads three things: the word in a state file, the pid of a process it must outlive
# and the newest line of a waveform channel. This script fabricates all three and launches the real
# presenter against them, so the pill can be looked at on a real session without recording
# anything: `recording` with the meter moving, then `transcribing`, then `cancelled`, three seconds
# each.
#
#     scripts/preview-pill.sh
#
# `TINY_DICTATE_PREVIEW_STATE` freezes the preview on one state indefinitely instead of cycling, so
# two states can be compared deliberately rather than caught in a flash:
#
#     TINY_DICTATE_PREVIEW_STATE=transcribing scripts/preview-pill.sh
#
# The value is one of `recording`, `transcribing` or `cancelled`; anything else is refused.
# `TINY_DICTATE_BIN` names another binary than `target/release/tiny-dictate`.
#
# It needs a display and layer-shell, and it is a development aid: nothing asserts on it and the
# integration suite never runs it. Ctrl-C leaves no pill and no child process behind.

set -euo pipefail

readonly ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
readonly BINARY="${TINY_DICTATE_BIN:-$ROOT/target/release/tiny-dictate}"
# The configuration's default waveform colour, passed as the presenter's fifth argument; keep it
# in sync with `config::DEFAULT_COLOR` in `src/config.rs`.
readonly COLOR="#e4e4e7"
# One tick per frame, the cadence `src/pill/mod.rs` draws at (`TICK_MS`): thirty of them are three
# seconds.
readonly TICKS_PER_STATE=30
readonly TICK_SECONDS=0.1
# The waveform line holds one `low high` pair per slice, as many as `audio::SLICES_PER_LINE` in
# `src/audio.rs` writes per line.
readonly SLICES=24

frozen="${TINY_DICTATE_PREVIEW_STATE:-}"
case "$frozen" in
    '' | recording | transcribing | cancelled) ;;
    *)
        printf 'preview: TINY_DICTATE_PREVIEW_STATE must be recording, transcribing or cancelled, not %s\n' "$frozen" >&2
        exit 2
        ;;
esac

if [ ! -x "$BINARY" ]; then
    printf 'preview: %s is not executable; run `cargo build --release` first\n' "$BINARY" >&2
    exit 2
fi

states=(recording transcribing cancelled)
if [ -n "$frozen" ]; then
    states=("$frozen")
fi

# One temporary directory holds the state file the presenter watches, the waveform channel it draws
# and the owner process that must outlive every state change.
workdir="$(mktemp -d "${TMPDIR:-/tmp}/tiny-dictate-preview.XXXXXX")"
readonly state_file="$workdir/state"
readonly state_tmp="$workdir/state.tmp"
readonly waveform="$workdir/waveform"
readonly waveform_tmp="$workdir/waveform.tmp"

presenter_pid=''
owner_pid=''
cleaned=0

# Rewrite the state file by rename, as the session does: the presenter must never read half a word.
write_state() {
    printf '%s\n' "$1" > "$state_tmp"
    mv -f -- "$state_tmp" "$state_file"
}

# The levels the waveform random-walks around, one per slice, so the meter moves rather than
# flickers. Seeded in the open so that `set -u` never reads an unset element.
declare -a levels=()
for ((i = 0; i < SLICES; i++)); do
    levels[i]=$((RANDOM % 6000 + 3000))
done

# One waveform line, replaced wholesale so the channel always holds a whole one: a `low high` pair
# per slice, in the -32768..32767 range the presenter parses.
write_waveform() {
    local i level target line=''
    for ((i = 0; i < SLICES; i++)); do
        target=$((RANDOM % 20000 + 3000))
        level=$(((levels[i] * 3 + target) / 4))
        levels[i]=$level
        if ((i > 0)); then
            line+=' '
        fi
        line+="$((-level)) $level"
    done
    printf '%s\n' "$line" > "$waveform_tmp"
    mv -f -- "$waveform_tmp" "$waveform"
}

# Ask one child to leave, bounded: a wedged process is killed, and either way it is reaped.
stop_process() {
    local pid="$1" state i
    [ -n "$pid" ] || return 0

    kill "$pid" 2>/dev/null || true
    for ((i = 0; i < 40; i++)); do
        state="$(ps -o stat= -p "$pid" 2>/dev/null || true)"
        # Gone, or a zombie waiting to be reaped: both are done.
        if [ -z "$state" ] || [[ "$state" == Z* ]]; then
            break
        fi
        sleep 0.05
    done

    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}

# Leave the display and the process table as they were found, whatever ended the script.
finish() {
    local status="$1"
    trap - EXIT INT TERM
    if [ "$cleaned" -eq 0 ]; then
        cleaned=1
        # The pill first, so it never sees the owner it was given die; the owner then.
        stop_process "$presenter_pid"
        stop_process "$owner_pid"
        rm -rf -- "$workdir"
    fi
    exit "$status"
}
trap 'finish 130' INT
trap 'finish 143' TERM
trap 'finish $?' EXIT

# The owner the presenter outlives: `exec` replaces the subshell, so this pid is the process to
# kill and nothing is orphaned behind it.
(exec sleep infinity) &
owner_pid=$!

# The presenter leaves on its first tick when the state file is missing, so both files exist before
# it is launched.
write_waveform
write_state "${states[0]}"

# The five arguments the session passes: the state file, the owner pid, the waveform channel, the
# command the pill's buttons run -- this same binary -- and the colour.
"$BINARY" _present "$state_file" "$owner_pid" "$waveform" "$BINARY" "$COLOR" &
presenter_pid=$!

last_state=''
while :; do
    for state in "${states[@]}"; do
        write_state "$state"
        if [ "$state" != "$last_state" ]; then
            printf 'preview: %s\n' "$state"
            last_state="$state"
        fi
        # Every state lasts the same number of ticks; only the recording one publishes a waveform.
        for ((tick = 0; tick < TICKS_PER_STATE; tick++)); do
            if [ "$state" = recording ]; then
                write_waveform
            fi
            sleep "$TICK_SECONDS"
        done
    done
done
