#!/usr/bin/env bash
# Integration tests for tiny-dictate: stubbed recorder, encoder, transcription command,
# presenter, notifier and keyboard injector. No microphone, no notification daemon, no
# layer-shell surface, no network.
#
#   tests/run.sh                              # tests ../tiny-dictate
#   SCRIPT=/path/to/tiny-dictate tests/run.sh
#   OLD_SCRIPT=/path/to/other tests/run.sh    # also compare the recorder race
#   RACE_ITERATIONS=50 tests/run.sh
#   DIAG=1 ONLY=test_normal_flow tests/run.sh # dump state, keep /tmp/td-diag
#
# What makes this safe to run on a live desktop: the tested code is launched with a session
# bus address and a Wayland display that do not exist, so even a missed stub could not pop a
# notification, draw a pill or touch the real clipboard.

SCRIPT="${SCRIPT:-$(cd "$(dirname "$0")/.." && pwd)/tiny-dictate}"
OLD_SCRIPT="${OLD_SCRIPT:-}"
RACE_ITERATIONS="${RACE_ITERATIONS:-25}"
ONLY="${ONLY:-}"
DIAG="${DIAG:-}"
TRACE="${TRACE:-}"

WORK="$(mktemp -d /tmp/td-work.XXXXXX)"
if [ -n "$DIAG" ]; then
    WORK="/tmp/td-diag"
    rm -rf "$WORK"
    mkdir -p "$WORK"
fi

# Safety net: the tested script must never be able to reach the real desktop session, so
# even a missed stub (or a stray process) cannot pop notifications on screen or touch the
# real clipboard. Every child inherits this.
export DBUS_SESSION_BUS_ADDRESS="unix:path=$WORK/no-session-bus"
export WAYLAND_DISPLAY="td-test-no-wayland"
BIN="$WORK/bin"
RUN="$WORK/run"
HOME_DIR="$WORK/home"
mkdir -p "$BIN" "$RUN" "$HOME_DIR"
export WORK BIN RUN HOME_DIR

FAILURES=0
CURRENT_TEST=""

# Resolve the real arecord before the stub directory is prepended to PATH.
REAL_ARECORD="$(command -v arecord)"

# ─────────────────────────── stubs ───────────────────────────

cat > "$BIN/arecord" <<STUB
#!/usr/bin/env bash
if [ -n "\${ARECORD_FAIL:-}" ]; then
    printf 'arecord: main:7226: audio open error: Device or resource busy\n' >&2
    exit 1
fi
# Use the real arecord on the ALSA "null" device: it installs its own SIGINT handler,
# exactly like a real capture. A shell script cannot: a background job of a
# non-interactive shell gets SIGINT set to SIG_IGN, and POSIX forbids a non-interactive
# shell from trapping a signal that was ignored on entry (trap ... INT is inert, and
# env --default-signal=SIGINT does not help either: bash installs its own handler).
# The absolute path is required, otherwise exec would re-enter this very stub.
# Caveat: the null device is not clocked, it produces data as fast as it is read, hence
# the trick in the lame stub below.
printf '%s\\n' "arecord stub pid=\$$ args=\$*" >> "\$WORK/arecord-report"
exec "$REAL_ARECORD" -D null "\$@"
STUB

cat > "$BIN/lame" <<'STUB'
#!/usr/bin/env bash
out="${*: -1}"
# Real lame encodes as fast as it reads and the recorder above is not clocked, so a plain
# copy would fill /tmp and keep the recorder blocked in write() forever. Write a plausible
# amount slowly (like a real recording), then drain and discard the rest until the recorder
# closes the pipe: real lame lives until EOF.
report() { printf 'lame %s at %s\n' "$1" "$(date +%s.%N)" >> "$WORK/lame-report"; }
report "start pid=$$"
: > "$out"
for _ in 1 2 3 4 5; do
    dd bs=1600 count=1 status=none 2>/dev/null >> "$out"
    sleep 0.05
done
report "draining, file=$(wc -c < "$out") bytes"
while :; do
    bytes="$(dd bs=1600 count=1 status=none 2>/dev/null | tee -a "$out" | wc -c)"
    if [ "$bytes" -eq 0 ]; then
        report "saw EOF"
        break
    fi
done
report "exiting"
STUB

cat > "$BIN/transcribe" <<'STUB'
#!/usr/bin/env bash
# Stand-in for the user-provided transcription command: audio file as $1, text on stdout.
cp "${1:?}" "$WORK/transcribe-received" 2>/dev/null
wc -c < "$WORK/transcribe-received" > "$WORK/transcribe-size"
[ -n "${TRANSCRIBE_SLEEP:-}" ] && sleep "$TRANSCRIBE_SLEEP"
if [ -n "${TRANSCRIBE_FAIL:-}" ]; then
    printf 'groq: Error code: 401 - invalid api key\n' >&2
    exit 1
fi
printf 'text transcribed'
STUB

cat > "$BIN/llm" <<'STUB'
#!/usr/bin/env bash
# Only needed by the previous implementation, when comparing the recorder race.
cat > /dev/null
printf 'text transcribed'
STUB

cat > "$BIN/notify-send" <<'STUB'
#!/usr/bin/env bash
# Stand-in for libnotify: logs the notification and the deadline it was given.
urgency=normal
expire=0
msg=""
for arg in "$@"; do
    case "$arg" in
        --urgency=*) urgency="${arg#--urgency=}" ;;
        --expire-time=*) expire="${arg#--expire-time=}" ;;
        *) msg="$arg" ;;
    esac
done
printf 'NOTIFY urgency=%s expire=%s msg=%s\n' "$urgency" "$expire" "$msg" >> "$WORK/events.log"
STUB

cat > "$BIN/present" <<'STUB'
#!/usr/bin/env bash
# Stand-in for the layer-shell presenter: logs the states it is asked to draw, and leaves by
# the three doors the real one has -- the state file disappears, it holds "stop", or the
# owner is gone.
state_file="$1"
owner="$2"
printf 'PRESENT start owner=%s file=%s caller=%s\n' "$owner" "$state_file" "$PPID" >> "$WORK/events.log"
state=""
while :; do
    if ! current="$(cat "$state_file" 2>/dev/null)"; then
        printf 'PRESENT gone\n' >> "$WORK/events.log"
        exit 0
    fi
    if [ "$current" = "stop" ]; then
        printf 'PRESENT stop\n' >> "$WORK/events.log"
        exit 0
    fi
    if ! kill -0 "$owner" 2>/dev/null; then
        printf 'PRESENT orphaned\n' >> "$WORK/events.log"
        exit 0
    fi
    if [ -n "$current" ] && [ "$current" != "$state" ]; then
        state="$current"
        printf 'PRESENT state=%s\n' "$state" >> "$WORK/events.log"
    fi
    sleep 0.05
done
STUB

cat > "$BIN/wl-copy" <<'STUB'
#!/usr/bin/env bash
cat > "$WORK/clipboard"
printf '%s\n' "$*" >> "$WORK/wl-copy.log"
STUB

cat > "$BIN/wl-paste" <<'STUB'
#!/usr/bin/env bash
cat "$WORK/clipboard" 2>/dev/null
STUB

cat > "$BIN/ydotool" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$WORK/ydotool.log"
exit 0
STUB

cat > "$BIN/wpctl" <<'STUB'
#!/usr/bin/env bash
[ -n "${MIC_MUTED:-}" ] && { echo "Volume: 0.50 [MUTED]"; exit 0; }
echo "Volume: 0.50"
STUB

cat > "$BIN/pactl" <<'STUB'
#!/usr/bin/env bash
[ -n "${MIC_MUTED:-}" ] && { echo "Mute: yes"; exit 0; }
echo "Mute: no"
STUB

cat > "$BIN/amixer" <<'STUB'
#!/usr/bin/env bash
echo "[on]"
STUB

chmod +x "$BIN"/*

# A broken stub would silently test nothing: check them before running anything.
for stub in "$BIN"/*; do
    if ! bash -n "$stub"; then
        printf 'ERROR: stub %s is not valid bash\n' "$stub"
        exit 99
    fi
done

# ─────────────────────────── helpers ───────────────────────────

td() {
    if [ -n "$TRACE" ]; then
        TINY_DICTATE_TRANSCRIBE="${TINY_DICTATE_TRANSCRIBE:-$BIN/transcribe}" \
            TINY_DICTATE_PRESENT="${TINY_DICTATE_PRESENT:-$BIN/present}" PATH="$BIN:$PATH" \
            XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" bash -x "$SCRIPT" "$@" 2>>"$WORK/trace.log"
    else
        TINY_DICTATE_TRANSCRIBE="${TINY_DICTATE_TRANSCRIBE:-$BIN/transcribe}" \
            TINY_DICTATE_PRESENT="${TINY_DICTATE_PRESENT:-$BIN/present}" PATH="$BIN:$PATH" \
            XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
    fi
}

# Same, without any environment override: exercises the fallback to the command names looked
# up on PATH.
td_default() {
    PATH="$BIN:$PATH" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
}

dump() {
    printf '\n---- dump: %s ----\n' "$1"
    printf '%s\n' '-- processes matching the stubs --'
    pgrep -af "$WORK/bin/" || true
    printf '%s\n' '-- ps (interesting processes) --'
    ps -eo pid,ppid,stat,etime,cmd 2>/dev/null | grep -E 'arecord|present|tiny-dictate|llm|wl-copy|ydotool' | grep -v grep || true
    printf '%s\n' '-- pidfiles and their process state --'
    for f in "$RUN"/tiny-dictate/session/*.pid; do
        [ -f "$f" ] || continue
        pid="$(cat "$f")"
        printf '%s = %s -> ' "$(basename "$f")" "$pid"
        ps -o stat=,cmd= -p "$pid" 2>/dev/null || printf 'gone\n'
    done
    printf '%s\n' '-- session directory --'
    ls -la "$RUN/tiny-dictate/session" 2>&1
    printf '%s\n' '-- arecord stderr --'
    cat "$RUN/tiny-dictate/session/arecord.err" 2>&1
    printf '%s\n' '-- run directory --'
    ls -la "$RUN/tiny-dictate" 2>&1
    printf '%s\n' '-- events.log --'
    cat "$WORK/events.log" 2>&1
    printf '%s\n' '-- ydotool.log --'
    cat "$WORK/ydotool.log" 2>&1
    printf '%s\n' '-- arecord-report --'
    cat "$WORK/arecord-report" 2>&1
    printf '%s\n' '-- lame-report --'
    cat "$WORK/lame-report" 2>&1
    printf '%s\n' '-- transcribe-size --'
    cat "$WORK/transcribe-size" 2>&1
    printf '%s\n\n' '---- end dump ----'
}

ok() { printf '    ok   %s\n' "$*"; }
ko() {
    FAILURES=$((FAILURES + 1))
    printf '    FAIL %s\n' "$*"
    printf '         last events: %s\n' "$(tail -3 "$WORK/events.log" 2>/dev/null | tr '\n' '|')"
}

check() { # check <description> <condition...>
    local desc="$1"
    shift
    if "$@"; then ok "$desc"; else ko "$desc"; fi
}

start_test() {
    CURRENT_TEST="$1"
    printf '\n== %s\n' "$CURRENT_TEST"
    reset_run
}

reset_run() {
    cleanup_processes
    rm -rf "$RUN/tiny-dictate"
    : > "$WORK/events.log"
    : > "$WORK/ydotool.log"
    : > "$WORK/wl-copy.log"
    rm -f "$WORK/transcribe-size" "$WORK/transcribe-received" "$WORK/clipboard" \
        "$WORK/arecord-report" "$WORK/lame-report"
}

cleanup_processes() {
    pkill -f "$WORK/bin/" 2>/dev/null
    # The arecord stub execs the real binary, so its command line no longer mentions
    # the stub directory.
    pkill -f 'arecord -D null' 2>/dev/null
    sleep 0.1
}

wait_idle() {
    local i
    for i in $(seq 1 100); do
        [ "$(td status 2>/dev/null)" = "idle" ] && return 0
        sleep 0.05
    done
    return 1
}

audio_size() { cat "$WORK/transcribe-size" 2>/dev/null || echo "none"; }

pasted_text() { cat "$WORK/clipboard" 2>/dev/null; }

pasted() { [ -n "$(pasted_text)" ]; }

# The stub presenter polls, so the state it was asked to draw is not in the log the instant
# the command that wrote it returns.
wait_for_event() {
    local pattern="$1"
    local i
    for i in $(seq 1 40); do
        grep -Eq "$pattern" "$WORK/events.log" && return 0
        sleep 0.05
    done
    return 1
}

# The happy path is silent: the pill carried the whole story, nothing needed notifying.
no_notification_at_all() {
    ! grep -q '^NOTIFY ' "$WORK/events.log"
}

# The pill left, and the log says how. A presenter that outlives its session is a pill
# frozen on screen: the exact symptom the notification spinner could leave behind. The loop
# gives the process the time to finish exiting after it logged that it was leaving.
presenter_left() {
    local i

    for i in $(seq 1 20); do
        pgrep -f "$WORK/bin/present" >/dev/null 2>&1 || break
        sleep 0.05
    done
    pgrep -f "$WORK/bin/present" >/dev/null 2>&1 && return 1

    awk '
        BEGIN { open_pill = 0 }
        /^PRESENT start/ { open_pill = 1 }
        /^PRESENT (stop|gone|orphaned)/ { open_pill = 0 }
        END { exit open_pill }
    ' "$WORK/events.log"
}

# The pill was asked to draw both live states, recording first.
pill_showed_both_states() {
    awk '
        /^PRESENT state=recording/ { recording = NR }
        /^PRESENT state=transcribing/ { transcribing = NR }
        END { exit !(recording && transcribing && recording < transcribing) }
    ' "$WORK/events.log"
}

# The recording failed before there was anything to transcribe, so the pill must never
# have been asked to show the later state.
pill_never_transcribed() {
    ! grep -q '^PRESENT state=transcribing' "$WORK/events.log"
}

# Nothing is sticky any more: the live state is painted, not notified, so the notifications
# left are terminal events, each with a deadline of its own.
notifications_expire() {
    ! grep -q 'expire=0' "$WORK/events.log"
}

# The pill is off screen before a notification lands: the guarantee the notification id and
# its spinner used to carry, now read off one append-only log both stubs write.
pill_gone_before_notification() {
    awk '
        /^PRESENT start/ { started = 1; gone = 0 }
        /^PRESENT (stop|gone|orphaned)/ { gone = 1 }
        /^NOTIFY / && started && !gone { print "notification while the pill is up: " $0; bad = 1 }
        END { exit bad }
    ' "$WORK/events.log"
}

no_leftover_process() {
    ! pgrep -f "$WORK/bin/" >/dev/null 2>&1 && ! pgrep -f 'arecord -D null' >/dev/null 2>&1
}

session_dir_gone() { [ ! -e "$RUN/tiny-dictate/session" ]; }

# ─────────────────────────── tests ───────────────────────────

test_normal_flow() {
    start_test "normal flow: start, record, stop, paste"
    # The transcription command takes a moment, so the pill has time to be seen in the
    # transcribing state: the stub presenter polls far slower than the real one draws.
    TRANSCRIBE_SLEEP=0.3 td start
    sleep 0.4
    [ -n "$DIAG" ] && dump "while recording"
    check "status is working while recording" [ "$(td status)" = "working" ]
    td stop
    check "session finishes" wait_idle
    [ -n "$DIAG" ] && dump "after stop"
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "audio given to the transcriber is not empty (was $(audio_size) bytes)" \
        [ "$(audio_size)" -gt 0 ] 2>/dev/null
    check "the pill showed recording then transcribing" pill_showed_both_states
    check "the happy path notifies nothing" no_notification_at_all
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

test_stop_immediately() {
    start_test "stop immediately after start (the race that was reported)"
    td start
    sleep 0.05
    td stop
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "audio given to the transcriber is not empty (was $(audio_size) bytes)" \
        [ "$(audio_size)" -gt 0 ] 2>/dev/null
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
}

test_recorder_failure() {
    start_test "the recorder fails at startup (microphone busy)"
    ARECORD_FAIL=1 td start
    check "session finishes" wait_idle
    check "nothing pasted" [ -z "$(pasted_text)" ]
    check "recording failure reported" grep -q 'Recording failed: no audio was captured' "$WORK/events.log"
    check "recorder error shown to the user" grep -q 'Device or resource busy' "$WORK/events.log"
    check "the pill never claimed to transcribe" pill_never_transcribed
    check "the notification waited for the pill to go" pill_gone_before_notification
    check "every notification expires on its own" notifications_expire
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

test_transcription_failure() {
    start_test "the transcription command fails"
    TRANSCRIBE_SLEEP=0.3 TRANSCRIBE_FAIL=1 td start
    sleep 0.3
    td stop
    check "session finishes" wait_idle
    check "nothing pasted" [ -z "$(pasted_text)" ]
    check "transcription failure reported" grep -q 'Transcription failed: groq' "$WORK/events.log"
    check "the pill showed transcribing first" grep -q '^PRESENT state=transcribing' "$WORK/events.log"
    check "the notification waited for the pill to go" pill_gone_before_notification
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
}

test_no_transcribe_command() {
    start_test "no transcription command configured"
    TINY_DICTATE_TRANSCRIBE=does-not-exist td start 2> "$WORK/no-command.err"
    check "start is refused" [ "$?" -ne 0 ]
    check "reason printed on stderr" grep -q 'No transcription command' "$WORK/no-command.err"
    check "nothing was recorded" [ "$(td status)" = "idle" ]
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

test_no_presenter() {
    start_test "no presenter configured"
    TINY_DICTATE_PRESENT=does-not-exist td start 2> "$WORK/no-presenter.err"
    check "start is refused" [ "$?" -ne 0 ]
    check "reason printed on stderr" grep -q 'No presenter' "$WORK/no-presenter.err"
    check "nothing was recorded" [ "$(td status)" = "idle" ]
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

test_default_commands() {
    start_test "the commands fall back to the names on PATH"
    cp "$BIN/transcribe" "$BIN/tiny-dictate-transcribe"
    cp "$BIN/present" "$BIN/tiny-dictate-present"
    td_default start
    sleep 0.3
    td_default stop
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "no presentation left behind" presenter_left
    rm -f "$BIN/tiny-dictate-transcribe" "$BIN/tiny-dictate-present"
}

test_cancel() {
    start_test "cancel while recording"
    td start
    sleep 0.3
    td cancel
    check "cancellation reported" grep -q '🛑 Recording cancelled' "$WORK/events.log"
    check "session finishes" wait_idle
    check "the pill left" wait_for_event '^PRESENT (stop|gone|orphaned)'
    check "nothing pasted" [ -z "$(pasted_text)" ]
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

test_second_start_refused() {
    start_test "a second start is refused while recording"
    td start
    sleep 0.2
    td start 2> "$WORK/second-start.err"
    check "second start exits non-zero" [ "$?" -ne 0 ]
    check "reason printed on stderr" grep -q 'already busy' "$WORK/second-start.err"
    check "recording still running" [ "$(td status)" = "working" ]
    td cancel
    check "session finishes" wait_idle
    check "no presentation left behind" presenter_left
}

test_toggle_during_transcription() {
    start_test "toggle while transcribing"
    TRANSCRIBE_SLEEP=1 td start
    sleep 0.3
    td stop
    sleep 0.2
    td toggle 2> "$WORK/toggle.err"
    check "toggle reports the state on stderr" grep -q 'already in progress' "$WORK/toggle.err"
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "no presentation left behind" presenter_left
}

# The reported bug: `start` wrote the recorder PID from a subshell that raced with
# the worker, so the worker could believe recording was over and transcribe an empty
# file. Run the same scenario many times and count the failures.
race_loop() {
    local script="$1"
    local iterations="$2"
    local failures=0
    local i

    for i in $(seq 1 "$iterations"); do
        reset_run
        PATH="$BIN:$PATH" TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT="$BIN/present" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" \
            "$script" start >/dev/null 2>&1
        sleep 0.05
        PATH="$BIN:$PATH" TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT="$BIN/present" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" \
            "$script" stop >/dev/null 2>&1
        for _ in $(seq 1 100); do
            [ "$(PATH="$BIN:$PATH" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$script" status 2>/dev/null)" = "idle" ] && break
            sleep 0.05
        done
        if [ "$(pasted_text)" != "text transcribed" ] || [ "$(audio_size)" = "0" ]; then
            failures=$((failures + 1))
        fi
    done
    cleanup_processes
    printf '%s' "$failures"
}

test_race_loop() {
    start_test "race loop x$RACE_ITERATIONS (new implementation)"
    local failures
    failures="$(race_loop "$SCRIPT" "$RACE_ITERATIONS")"
    if [ "$failures" -eq 0 ]; then
        ok "0/$RACE_ITERATIONS failures"
    else
        ko "$failures/$RACE_ITERATIONS iterations lost the audio"
    fi
}

test_race_loop_old() {
    start_test "race loop x$RACE_ITERATIONS (previous implementation, for comparison)"
    local failures
    failures="$(race_loop "$OLD_SCRIPT" "$RACE_ITERATIONS")"
    printf '    info %s/%s iterations lost the audio\n' "$failures" "$RACE_ITERATIONS"
}

# ─────────────────────────── main ───────────────────────────

printf 'script under test: %s\n' "$SCRIPT"
printf 'work directory:    %s\n' "$WORK"

run_test() {
    if [ -n "$ONLY" ] && [ "$ONLY" != "$1" ]; then
        return 0
    fi
    "$1"
    [ -n "$DIAG" ] && dump "after $1"
    return 0
}

run_test test_normal_flow
run_test test_stop_immediately
run_test test_recorder_failure
run_test test_transcription_failure
run_test test_no_transcribe_command
run_test test_no_presenter
run_test test_default_commands
run_test test_cancel
run_test test_second_start_refused
run_test test_toggle_during_transcription
run_test test_race_loop
if [ -n "$OLD_SCRIPT" ] && [ -z "$ONLY" ]; then
    test_race_loop_old
fi

cleanup_processes

printf '\n'
if [ "$FAILURES" -eq 0 ]; then
    printf 'ALL TESTS PASSED\n'
    exit 0
fi
printf '%s FAILURE(S)\n' "$FAILURES"
exit 1
