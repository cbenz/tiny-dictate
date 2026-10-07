#!/usr/bin/env bash
# Integration tests for tiny-dictate: stubbed recorder, encoder, transcription command,
# presenter, notifier and keyboard injector. No microphone, no notification daemon, no
# layer-shell surface, no network.
#
#   tests/run.sh                              # tests ../target/release/tiny-dictate
#   SCRIPT=/path/to/tiny-dictate tests/run.sh
#   OLD_SCRIPT=/path/to/other tests/run.sh    # also compare the recorder race
#   RACE_ITERATIONS=50 tests/run.sh
#   DIAG=1 ONLY=test_normal_flow tests/run.sh # dump state, keep /tmp/td-diag
#
# What makes this safe to run on a live desktop: the tested code is launched with a session
# bus address and a Wayland display that do not exist, so even a missed stub could not pop a
# notification, draw a pill or touch the real clipboard.

SCRIPT="${SCRIPT:-$(cd "$(dirname "$0")/.." && pwd)/target/release/tiny-dictate}"
OLD_SCRIPT="${OLD_SCRIPT:-}"
RACE_ITERATIONS="${RACE_ITERATIONS:-25}"
ONLY="${ONLY:-}"
DIAG="${DIAG:-}"

# The suite runs the built binary rather than building it: silently compiling here would test
# something the user did not ask for, and a stale binary would not say so.
if [ ! -x "$SCRIPT" ]; then
    printf 'ERROR: %s is not executable.\n' "$SCRIPT"
    printf 'Build it first: cargo build --release\n'
    exit 99
fi

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
# The durable sessions live under the state directory. Pointing it at the work directory
# keeps a run hermetic: reset_run wipes it like the runtime directory.
STATE_HOME="$WORK/state"
mkdir -p "$BIN" "$RUN" "$HOME_DIR" "$STATE_HOME"
export WORK BIN RUN HOME_DIR STATE_HOME
export XDG_STATE_HOME="$STATE_HOME"

FAILURES=0
CURRENT_TEST=""

# Resolve the real arecord before the stub directory is prepended to PATH.
REAL_ARECORD="$(command -v arecord)"

# ─────────────────────────── stubs ───────────────────────────

cat > "$BIN/arecord" <<STUB
#!/usr/bin/env bash
# Real arecord banners on stderr. The tool must keep the recorder's chatter out of the journal, so
# the stub says something the suite can look for while the dictation runs.
printf 'arecord: stub chatter\n' >&2
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
# Real lame banners on stderr. The tool must keep the encoder's chatter out of the journal, so the
# stub says something the suite can look for while the dictation runs.
printf 'LAME 3.100 64bits (http://lame.sf.net)\n' >&2
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
# Stand-in for the user-provided transcription command: the audio file as $1, the durable
# session directory as $2, the text on stdout. Writing nothing into the session directory is
# valid; TRANSCRIBE_RAW makes it leave a raw response behind, as the reference one does, and
# TRANSCRIBE_TEXT changes the text it prints.
# The audio-limit probe is answered from TRANSCRIBE_LIMIT (a byte count on stdout, exit 0);
# an absent or empty value prints nothing, which the tool reads as no limit, and
# TRANSCRIBE_LIMIT_FAIL makes the probe fail instead.
if [ "${1:-}" = "--print-audio-limit" ]; then
    [ -n "${TRANSCRIBE_LIMIT_FAIL:-}" ] && exit 1
    [ -n "${TRANSCRIBE_LIMIT:-}" ] && printf '%s\n' "$TRANSCRIBE_LIMIT"
    exit 0
fi
cp "${1:?}" "$WORK/transcribe-received" 2>/dev/null
wc -c < "$WORK/transcribe-received" > "$WORK/transcribe-size"
[ -n "${TRANSCRIBE_SLEEP:-}" ] && sleep "$TRANSCRIBE_SLEEP"
if [ -n "${TRANSCRIBE_FAIL:-}" ]; then
    printf 'groq: Error code: 401 - invalid api key\n' >&2
    exit 1
fi
if [ -n "${TRANSCRIBE_RAW_TEXT:-}" ]; then
    printf '%s' "$TRANSCRIBE_RAW_TEXT" > "${2:?}/transcript.raw.json"
elif [ -n "${TRANSCRIBE_RAW:-}" ]; then
    printf '%s' '{"raw":"kept verbatim"}' > "${2:?}/transcript.raw.json"
fi
printf '%s' "${TRANSCRIBE_TEXT:-text transcribed}"
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
waveform_file="$3"
printf 'PRESENT start owner=%s file=%s waveform=%s caller=%s limit=%s command=%s\n' "$owner" "$state_file" "$waveform_file" "$PPID" "${5:-}" "${4:-}" >> "$WORK/events.log"
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

# A meter that dies on its first slice instead of reading it. The scenario is kept as a regression
# net: the waveform channel must never be able to end a recording. The in-process slicer calls no
# external meter any more, so this stub no longer bites; what the case still checks is the shape,
# not the stub.
mkdir -p "$BIN-broken-meter"
cat > "$BIN-broken-meter/od" <<'STUB'
#!/usr/bin/env bash
exit 1
STUB

chmod +x "$BIN"/* "$BIN-broken-meter"/*

# A broken stub would silently test nothing: check them before running anything.
for stub in "$BIN"/* "$BIN-broken-meter"/*; do
    if ! bash -n "$stub"; then
        printf 'ERROR: stub %s is not valid bash\n' "$stub"
        exit 99
    fi
done

# ─────────────────────────── helpers ───────────────────────────

td() {
    TINY_DICTATE_TRANSCRIBE="${TINY_DICTATE_TRANSCRIBE:-$BIN/transcribe}" \
        TINY_DICTATE_PRESENT_COMMAND="${TINY_DICTATE_PRESENT_COMMAND:-$BIN/present}" PATH="$BIN:$PATH" \
        XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
}

# Same, without any environment override: exercises the fallback to the command names looked
# up on PATH.
td_default() {
    PATH="$BIN:$PATH" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
}

# Same as td, with a waveform meter that dies instead of reading the stream.
td_broken_meter() {
    PATH="$BIN-broken-meter:$BIN:$PATH" \
        TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT_COMMAND="$BIN/present" \
        XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
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
    rm -rf "$STATE_HOME/tiny-dictate"
    : > "$WORK/events.log"
    : > "$WORK/ydotool.log"
    : > "$WORK/wl-copy.log"
    rm -f "$WORK/transcribe-size" "$WORK/transcribe-received" "$WORK/clipboard" \
        "$WORK/arecord-report" "$WORK/lame-report" "$WORK/journal.err"
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

waveform_file() { printf '%s/tiny-dictate/session/waveform' "$RUN"; }

# The waveform file holds one line per 96 ms of audio: 24 slices, each the lowest and highest
# sample of the slice, so 48 integers. Only the last line may be short: a read can catch the
# writer between the fields of the line it is appending.
waveform_file_is_a_slice_series() {
    local file

    file="$(waveform_file)"
    [ -s "$file" ] || return 1
    awk '
        {
            if (NF != 48) short_line = NR
            for (i = 1; i <= NF; i++)
                if ($i !~ /^-?[0-9]+$/ || $i < -32768 || $i > 32767) bad = 1
        }
        END { exit !(bad != 1 && (short_line == 0 || short_line == NR)) }
    ' "$file"
}

# The pill is handed the waveform file as $3, so the waveform it draws is the recording it
# belongs to.
presenter_got_waveform_file() {
    grep -q "^PRESENT start .*waveform=$(waveform_file) " "$WORK/events.log"
}

# The pill is handed the command that re-invokes the tool as $4, so the buttons it draws (the cross
# and the check of the reference presenter) end a dictation through the path the keyboard uses.
presenter_got_command() {
    grep -q "^PRESENT start .*command=$(readlink -f "$SCRIPT")$" "$WORK/events.log"
}

# The pill is handed the limit in seconds as $5 when the transcriber declared one, and an empty
# argument otherwise: the presenter draws the bar from it, so both sides are asserted here.
presenter_got_limit() {
    grep -q "^PRESENT start .*limit=$1 command=" "$WORK/events.log"
}

presenter_got_no_limit() {
    grep -q "^PRESENT start .*limit= command=" "$WORK/events.log"
}

# The transcriber declared no limit, so nothing stopped the recording before the shortcut did.
limit_never_enforced() {
    ! grep -q 'audio limit' "$WORK/no-limit.err"
}

# The recorder the core recorded, checked for liveness: the waveform channel is beside the voice
# path, so nothing that happens in it may end a recording that is still going on.
recorder_alive() {
    local pid

    pid="$(cat "$RUN/tiny-dictate/session/arecord.pid" 2>/dev/null)" || return 1
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

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

# Nothing is left reading the waveform channel once the recording is over. The fifo path pins the
# match to this run's own copy: a dictation the user happens to be running for real, on the real
# runtime directory, is none of this test's business. The meter is the tee's child and goes when the
# tee does, so looking for the tee is enough.
no_waveform_channel_process() {
    local i

    for i in $(seq 1 20); do
        pgrep -f "$RUN/tiny-dictate/session/audio.fifo" >/dev/null 2>&1 || return 0
        sleep 0.05
    done
    return 1
}

session_dir_gone() { [ ! -e "$RUN/tiny-dictate/session" ]; }

# ─────────────────────── the journal, on stderr ───────────────────────

# The journal of one dictation: the tool's own lines, written to stderr
# (`docs/adr/0008-the-log.md`). The tests redirect a command's stderr into this file, and the
# session process inherits the descriptor, so the whole timeline lands here.
JOURNAL="$WORK/journal.err"

journal_has() { grep -Eq "$1" "$JOURNAL"; }
journal_lacks() { ! grep -Eq "$1" "$JOURNAL"; }

# Every line is the tool's own: a collaborator whose output reached the journal writes a line the
# tool's prefix does not open.
journal_holds_only_tool_lines() {
    [ -s "$JOURNAL" ] || return 1
    ! grep -qv '^tiny-dictate: ' "$JOURNAL"
}

# One event, named by the session it concerns.
journal_line_names_session() { # journal_line_names_session <session id> <event>
    grep -Eq "^tiny-dictate: $1: $2\$" "$JOURNAL"
}

# Every line of a dictation names its session: everything a dictation logs concerns it.
journal_every_line_names_session() { # journal_every_line_names_session <session id>
    [ -s "$JOURNAL" ] || return 1
    ! grep -Evq "^tiny-dictate: $1: " "$JOURNAL"
}

# A line that concerns no session carries no id. The id is the local start timestamp, so its
# absence from every line of a file is what "carries none" means.
journal_no_line_names_a_session() { # journal_no_line_names_a_session <file>
    [ -s "$1" ] || return 1
    ! grep -Eq '^tiny-dictate: [0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}:' "$1"
}

# The text of the failure line is the error the metadata holds: the journal and the durable
# trace agree on what went wrong (`docs/features/dictation-lifecycle.md`).
journal_failure_matches_metadata() { # journal_failure_matches_metadata <session dir> <session id>
    local expected

    expected="$(metadata_error "$1")"
    expected="${expected#\"}"
    expected="${expected%\"}"

    [ -n "$expected" ] && [ "$expected" != "null" ] &&
        grep -qF "tiny-dictate: $2: failure: $expected" "$JOURNAL"
}

# The stages of a dictation appear in the order they happened: the journal is a timeline, not a
# bag of lines.
journal_stages_in_order() { # journal_stages_in_order <session id>
    local id="$1"
    local previous=0 line

    while IFS= read -r stage; do
        line="$(grep -n -m1 -F "tiny-dictate: $id: $stage" "$JOURNAL" | cut -d: -f1)"
        [ -n "$line" ] && [ "$line" -gt "$previous" ] || return 1
        previous="$line"
    done <<STAGES
start: recording started
recording started
stop: recorder signalled
recording ended duration=
transcription started
transcription ended duration=
text written chars=
paste succeeded
STAGES
    return 0
}

# ─────────────────────── the durable session, on disk ───────────────────────

sessions_dir() { printf '%s/tiny-dictate/sessions' "$STATE_HOME"; }

# The one session directory of the run, when there is exactly one.
single_session_dir() {
    local ids count

    ids="$(ls -1 "$(sessions_dir)" 2>/dev/null)"
    count="$(printf '%s\n' "$ids" | grep -c .)"
    [ "$count" -eq 1 ] || return 1
    printf '%s/%s' "$(sessions_dir)" "$ids"
}

# How many sessions are kept: the count of directories under the sessions root.
session_count() { ls -1 "$(sessions_dir)" 2>/dev/null | grep -c .; }

# The newest and the oldest session id: the ids are timestamps, so sorting their names sorts
# them in time, and the newest is the last.
newest_session_id() { ls -1 "$(sessions_dir)" 2>/dev/null | sort | tail -1; }
oldest_session_id() { ls -1 "$(sessions_dir)" 2>/dev/null | sort | head -1; }

# One field of a session's metadata, raw JSON: `"transcribed"`, `true`, `null`, `1759855522.5`.
metadata_value() { # metadata_value <session directory> <key>
    awk -v key="\"$2\":" '$1 == key { value = $2; sub(/,$/, "", value); print value; exit }' \
        "$1/metadata.json"
}

# The `error` field as raw JSON: `null`, or the quoted one-line message. Unlike metadata_value,
# it keeps a value that holds spaces, which is every real error.
metadata_error() { # metadata_error <session directory>
    sed -n 's/^  "error": //p' "$1/metadata.json"
}

# Kill the whole run the way a crash would: the session process first, then the pipeline and
# the pill. SIGKILL, so nothing gets a chance to tidy up after itself.
kill_run() {
    local pid

    pid="$(cat "$RUN/tiny-dictate/session/session.pid" 2>/dev/null)"
    [ -n "$pid" ] && kill -9 "$pid" 2>/dev/null
    pkill -9 -f "$RUN/tiny-dictate/session/audio.fifo" 2>/dev/null
    cleanup_processes
    sleep 0.1
}

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

test_waveform_channel() {
    start_test "the waveform channel carries the recording to the pill"
    td start
    sleep 0.4
    check "the pill is given the waveform file" presenter_got_waveform_file
    check "the pill is given the command to run" presenter_got_command
    check "the waveform file is a series of slices" waveform_file_is_a_slice_series
    td stop
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "no waveform channel process left behind" no_waveform_channel_process
    check "session directory removed" session_dir_gone
}

# The transcriber declares the largest audio file it accepts; the recording then stops on its
# own at the duration the encoder's 64 kbit/s makes of that byte count, and the text is
# transcribed and pasted exactly as if the shortcut had been pressed.
test_audio_limit() {
    start_test "the transcriber's audio limit ends the recording by itself"
    # 8000 bytes at 8000 bytes per second (64 kbit/s) is one second. The transcription takes a
    # moment so the pill has time to be seen leaving the recording state, as in the normal flow.
    TRANSCRIBE_LIMIT=8000 TRANSCRIBE_SLEEP=0.3 td start 2> "$WORK/limit.err"
    check "session finishes by itself" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "audio given to the transcriber is not empty (was $(audio_size) bytes)" \
        [ "$(audio_size)" -gt 0 ] 2>/dev/null
    check "the pill is handed the limit in seconds" presenter_got_limit 1
    check "the pill showed recording then transcribing" pill_showed_both_states
    check "the log says why the recording stopped" grep -q 'audio limit' "$WORK/limit.err"
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

# A transcriber that does not answer the probe declares no limit: the dictation is otherwise
# unchanged, and the pill is handed no limit so it draws no bar.
test_no_audio_limit() {
    start_test "a transcriber that declares no limit"
    TRANSCRIBE_LIMIT_FAIL=1 td start 2> "$WORK/no-limit.err"
    sleep 0.3
    check "the pill is handed no limit" presenter_got_no_limit
    td stop
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "no limit was enforced" limit_never_enforced
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

# The waveform channel is a copy, and this is what that buys: a meter that dies on its first
# slice cannot close the pipe under the recorder. The guarantee `tee -p` used to carry is the
# pump's now: a failed waveform write is dropped and the stream keeps draining.
test_broken_meter() {
    start_test "the waveform meter dies mid-recording"
    td_broken_meter start
    sleep 0.4
    check "the recorder outlives a dead meter" recorder_alive
    td stop
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "audio given to the transcriber is not empty (was $(audio_size) bytes)" \
        [ "$(audio_size)" -gt 0 ] 2>/dev/null
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
    check "no waveform channel left behind" no_waveform_channel_process
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

# The pill is no longer a command the user installs: it is a mode of the tool, and the session
# spawns the presenter itself. A presenter that cannot start therefore costs the pill only -- the
# dictation must run end to end without one, pasting its text as usual.
test_no_presenter() {
    start_test "a presenter that cannot start"
    TINY_DICTATE_PRESENT_COMMAND="$WORK/does-not-exist" td start 2> "$WORK/no-presenter.err"
    check "start is not blocked by the missing pill" [ "$?" -eq 0 ]
    sleep 0.3
    td stop
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
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
    check "the pill shows the verdict" wait_for_event '^PRESENT state=cancelled'
    check "cancelling notifies nothing" no_notification_at_all
    check "session finishes" wait_idle
    check "nothing pasted" [ -z "$(pasted_text)" ]
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

# `toggle` started a new dictation instead of refusing it: the journal line proves the command
# ran, and the refusal itself is absent from what it wrote to stderr.
chain_has_no_busy_message() { ! grep -q 'already busy' "$WORK/chain.err"; }

test_chain_after_cancel() {
    start_test "a new dictation starts before the cancelled verdict is over"
    td start
    sleep 0.3
    td cancel
    check "the pill shows the verdict" wait_for_event '^PRESENT state=cancelled'
    # No waiting for the pill to leave: the next dictation asks for the microphone now.
    td toggle 2> "$WORK/chain.err"
    check "the new dictation is recording" [ "$(td status)" = "working" ]
    # The journal now writes the command's line to stderr, so empty stderr is no longer what "no
    # busy message" means: the refusal itself is.
    check "no busy message" chain_has_no_busy_message
    check "the toggle was logged rather than refused" \
        grep -Eq '^tiny-dictate: .*: toggle: recording started$' "$WORK/chain.err"
    td cancel
    check "session finishes" wait_idle
    check "the cancelled recording is never transcribed" [ -z "$(pasted_text)" ]
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

test_cancel_during_transcription() {
    start_test "cancel while transcribing"
    TRANSCRIBE_SLEEP=1 td start
    sleep 0.3
    td stop
    sleep 0.2
    td cancel
    check "the pill shows the verdict" wait_for_event '^PRESENT state=cancelled'
    check "cancelling notifies nothing" no_notification_at_all
    check "session finishes" wait_idle
    check "the transcription is dropped" [ -z "$(pasted_text)" ]
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

# A completed dictation leaves its session under the state directory: the audio the encoder
# wrote, the transcript the core wrote, and the metadata saying what happened to it.
test_session_on_disk() {
    start_test "a completed dictation leaves its session on disk"
    td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir
    dir="$(single_session_dir)"
    check "exactly one session was left" test -n "$dir"
    check "audio.mp3 is there and not empty" test -s "$dir/audio.mp3"
    check "the core wrote transcript.txt" [ "$(cat "$dir/transcript.txt")" = "text transcribed" ]
    check "metadata.json is there" test -f "$dir/metadata.json"
    check "no temporary metadata survived" [ -z "$(find "$dir" -name '*.tmp.*' -print -quit 2>/dev/null)" ]
    check "the outcome is transcribed" [ "$(metadata_value "$dir" outcome)" = '"transcribed"' ]
    check "the text was pasted" [ "$(metadata_value "$dir" pasted)" = "true" ]
    check "the transcriber is recorded" [ "$(metadata_value "$dir" transcriber)" = "\"$BIN/transcribe\"" ]
    check "no error is recorded" [ "$(metadata_value "$dir" error)" = "null" ]
    check "audio_bytes is the file's size" bash -c '[ -n "$1" ] && [ "$1" = "$2" ]' _ \
        "$(metadata_value "$dir" audio_bytes)" "$(stat -c%s "$dir/audio.mp3" 2>/dev/null)"
    check "the duration is positive" awk -v d="$(metadata_value "$dir" duration)" 'BEGIN { exit !(d > 0) }'
    check "the end is after the start" awk -v s="$(metadata_value "$dir" started)" -v e="$(metadata_value "$dir" ended)" 'BEGIN { exit !(e > s) }'
    check "the runtime claim is removed" session_dir_gone
    check "no leftover process" no_leftover_process
}

# The transcriber leaves the provider's own answer in the directory it is handed, and the
# core stores it verbatim: it is never rewritten, and the transcript is the core's own file.
test_session_raw_response() {
    start_test "the transcriber's raw response is kept verbatim"
    TRANSCRIBE_RAW=1 td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir
    dir="$(single_session_dir)"
    check "the raw response is left in the session" test -f "$dir/transcript.raw.json"
    check "it is stored verbatim" [ "$(cat "$dir/transcript.raw.json")" = '{"raw":"kept verbatim"}' ]
    check "the core still wrote the transcript" [ "$(cat "$dir/transcript.txt")" = "text transcribed" ]
    check "the text was pasted" [ "$(pasted_text)" = "text transcribed" ]
}

# A transcriber that leaves nothing in the session directory is still a transcriber: the text
# it printed is what the core writes and pastes.
test_session_transcriber_writes_nothing() {
    start_test "a transcriber that writes nothing into the session"
    TRANSCRIBE_TEXT="text from stdout" td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir
    dir="$(single_session_dir)"
    check "no raw response was written" [ ! -e "$dir/transcript.raw.json" ]
    check "the core wrote the text the transcriber printed" [ "$(cat "$dir/transcript.txt")" = "text from stdout" ]
    check "the printed text is what was pasted" [ "$(pasted_text)" = "text from stdout" ]
}

# A session left on `recording` is one that died mid-recording: its metadata has no end and
# no duration, and nothing was transcribed or pasted.
test_session_recording_killed() {
    start_test "a session killed mid-recording stays on recording"
    td start
    sleep 0.4

    local dir
    dir="$(single_session_dir)"
    check "the metadata is written while recording" test -f "$dir/metadata.json"
    check "the outcome is recording" [ "$(metadata_value "$dir" outcome)" = '"recording"' ]

    kill_run

    check "the session directory survives the kill" test -d "$dir"
    check "the metadata still says recording" [ "$(metadata_value "$dir" outcome)" = '"recording"' ]
    check "the recording has no end" [ "$(metadata_value "$dir" ended)" = "null" ]
    check "the recording has no duration" [ "$(metadata_value "$dir" duration)" = "null" ]
    check "no transcript was written" [ ! -e "$dir/transcript.txt" ]
}

# The recorder writes the audio from the first frame, so killing the whole run costs at most
# what has not been written: the bytes already flushed are still on disk afterwards.
test_session_audio_survives_kill() {
    start_test "the audio survives killing the whole run"
    td start
    sleep 0.4

    local dir before
    dir="$(single_session_dir)"
    cp "$dir/audio.mp3" "$WORK/audio-before-kill"
    before="$(stat -c%s "$WORK/audio-before-kill" 2>/dev/null || echo 0)"
    check "the recording reached disk before the kill (was $before bytes)" test "$before" -gt 0

    kill_run
    head -c "$before" "$dir/audio.mp3" > "$WORK/audio-after-kill"

    check "the file is still there" test -s "$dir/audio.mp3"
    check "the flushed audio is still there, byte for byte" cmp -s "$WORK/audio-before-kill" "$WORK/audio-after-kill"
    check "the transcriber can still read it" bash -c "\"$BIN/transcribe\" \"$dir/audio.mp3\" \"$dir\" >/dev/null 2>&1"
}

# Cancelling drops the text, never the recording: the session keeps its audio and is marked
# cancelled, and nothing is transcribed or pasted.
test_session_cancelled_keeps_recording() {
    start_test "a cancelled dictation keeps its recording"
    td start
    sleep 0.4
    td cancel
    check "the pill shows the verdict" wait_for_event '^PRESENT state=cancelled'
    check "session finishes" wait_idle

    local dir
    dir="$(single_session_dir)"
    check "the recording is kept" test -s "$dir/audio.mp3"
    check "the outcome is cancelled" [ "$(metadata_value "$dir" outcome)" = '"cancelled"' ]
    check "nothing was transcribed" [ ! -e "$dir/transcript.txt" ]
    check "nothing was pasted" [ -z "$(pasted_text)" ]
    check "the runtime claim is removed" session_dir_gone
    check "no leftover process" no_leftover_process
}

# ──────────────────── the rescue surface: history, transcribe, session delete ────────────────────

# `history` lists the kept sessions newest first, and each line carries the id, the duration,
# the outcome and the first words of the transcript or the error.
test_history_lists_sessions_newest_first() {
    start_test "history lists the sessions newest first with their fields"
    # The first dictation fails at the transcriber, the second succeeds: both the transcript
    # and the error are on the lines, each on its own session.
    TRANSCRIBE_FAIL=1 td start
    sleep 0.4
    td stop
    check "the failed session finishes" wait_idle

    TRANSCRIBE_TEXT="second text" td start
    sleep 0.4
    td stop
    check "the transcribed session finishes" wait_idle

    check "both dictations left a session" [ "$(session_count)" -eq 2 ]

    local newest oldest lines first last
    newest="$(newest_session_id)"
    oldest="$(oldest_session_id)"
    lines="$(td history)"
    check "history prints one line per session" [ "$(printf '%s\n' "$lines" | grep -c .)" -eq 2 ]
    first="$(printf '%s\n' "$lines" | head -1)"
    last="$(printf '%s\n' "$lines" | tail -1)"

    check "the newest session is first" bash -c '[[ "$1" == "$2 "* ]]' _ "$first" "$newest"
    check "the oldest session is last" bash -c '[[ "$1" == "$2 "* ]]' _ "$last" "$oldest"
    check "the newest line carries its outcome" bash -c '[[ "$1" == *" transcribed "* ]]' _ "$first"
    check "the newest line carries the first words of the transcript" bash -c '[[ "$1" == *" second text" ]]' _ "$first"
    check "the newest line carries a duration" bash -c '[[ "$1" =~ [0-9]+\.[0-9]s ]]' _ "$first"
    check "the oldest line carries its outcome" bash -c '[[ "$1" == *" failed "* ]]' _ "$last"
    check "the oldest line carries the error" bash -c '[[ "$1" == *"groq: Error code: 401 - invalid api key" ]]' _ "$last"

    # The failed session is what the rescue surface exists for: transcribe it again with the
    # transcriber working, and the text lands in the session that had none.
    TRANSCRIBE_TEXT="rescued text" td transcribe "$oldest" > /dev/null 2>&1
    check "transcribing the failed session by id succeeds" [ "$?" -eq 0 ]
    check "it now holds the text" [ "$(cat "$(sessions_dir)/$oldest/transcript.txt")" = "rescued text" ]
    check "its outcome became transcribed" [ "$(metadata_value "$(sessions_dir)/$oldest" outcome)" = '"transcribed"' ]
}

# `transcribe last` re-runs the transcriber on the newest session, rewrites the transcript and
# the raw response, copies the text to the clipboard and says so.
test_transcribe_copies_and_rewrites() {
    start_test "transcribe re-runs the transcriber, rewrites the session and copies the text"
    TRANSCRIBE_TEXT="first text" TRANSCRIBE_RAW_TEXT='{"raw":"first"}' td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir id
    dir="$(single_session_dir)"
    id="$(basename "$dir")"
    rm -f "$WORK/clipboard"

    TRANSCRIBE_TEXT="second text" TRANSCRIBE_RAW_TEXT='{"raw":"second"}' \
        td transcribe last 2> "$WORK/transcribe.err"
    check "transcribe succeeds" [ "$?" -eq 0 ]
    check "the transcript is overwritten" [ "$(cat "$dir/transcript.txt")" = "second text" ]
    check "the raw response is overwritten" [ "$(cat "$dir/transcript.raw.json")" = '{"raw":"second"}' ]
    check "the text is on the clipboard" [ "$(pasted_text)" = "second text" ]
    check "the command says the text is on the clipboard" \
        grep -q "^tiny-dictate: $id: transcript copied to the clipboard$" "$WORK/transcribe.err"
    check "the metadata says transcribed" [ "$(metadata_value "$dir" outcome)" = '"transcribed"' ]
    check "no error is recorded" [ "$(metadata_value "$dir" error)" = "null" ]
    check "the transcriber is recorded" [ "$(metadata_value "$dir" transcriber)" = "\"$BIN/transcribe\"" ]
    check "no leftover process" no_leftover_process
}

# `transcribe --print` writes the text to standard output instead of the clipboard.
test_transcribe_print() {
    start_test "transcribe --print writes the text to stdout instead of the clipboard"
    TRANSCRIBE_TEXT="first text" td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir out
    dir="$(single_session_dir)"
    rm -f "$WORK/clipboard"

    out="$(TRANSCRIBE_TEXT="printed text" td transcribe last --print 2> "$WORK/print.err")"
    check "the text is on stdout" [ "$out" = "printed text" ]
    check "the transcript is rewritten" [ "$(cat "$dir/transcript.txt")" = "printed text" ]
    check "nothing was copied to the clipboard" [ ! -e "$WORK/clipboard" ]
    check "nothing claims the clipboard" bash -c '! grep -q clipboard "$1"' _ "$WORK/print.err"
}

# `session delete` removes exactly one session and leaves the others alone.
test_session_delete_removes_one() {
    start_test "session delete removes exactly one session"
    TRANSCRIBE_TEXT="first text" td start
    sleep 0.4
    td stop
    check "the first session finishes" wait_idle

    local first_dir
    first_dir="$(single_session_dir)"

    TRANSCRIBE_TEXT="second text" td start
    sleep 0.4
    td stop
    check "the second session finishes" wait_idle

    check "two sessions were kept" [ "$(session_count)" -eq 2 ]
    local newest
    newest="$(newest_session_id)"
    check "the newest is the second dictation" [ "$newest" != "$(basename "$first_dir")" ]

    td session delete last 2> "$WORK/delete.err"
    check "session delete succeeds" [ "$?" -eq 0 ]
    check "one session is left" [ "$(session_count)" -eq 1 ]
    check "the newest session is gone" [ ! -e "$(sessions_dir)/$newest" ]
    check "the other session is untouched" test -d "$first_dir"
    check "the deletion is logged without an id" \
        grep -q '^tiny-dictate: session deleted$' "$WORK/delete.err"

    # The id form deletes a named session, the way `last` deletes the newest.
    td session delete "$(basename "$first_dir")" 2> "$WORK/delete-id.err"
    check "deleting by id succeeds" [ "$?" -eq 0 ]
    check "the sessions directory is empty" [ "$(session_count)" -eq 0 ]
}

# An unknown id is an error on stderr, exit non-zero, and nothing is removed or written.
test_unknown_session_id() {
    start_test "an unknown session id is reported and changes nothing"
    TRANSCRIBE_TEXT="kept text" td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir
    dir="$(single_session_dir)"

    td transcribe does-not-exist 2> "$WORK/unknown-transcribe.err"
    check "transcribe exits non-zero" [ "$?" -ne 0 ]
    check "transcribe reports the unknown id" \
        grep -q 'unknown session: does-not-exist' "$WORK/unknown-transcribe.err"

    td session delete does-not-exist 2> "$WORK/unknown-delete.err"
    check "session delete exits non-zero" [ "$?" -ne 0 ]
    check "session delete reports the unknown id" \
        grep -q 'unknown session: does-not-exist' "$WORK/unknown-delete.err"

    check "the session is still there" test -d "$dir"
    check "the transcript is untouched" [ "$(cat "$dir/transcript.txt")" = "kept text" ]
}

# `history` and `transcribe` need neither the encoder nor the clipboard tools: the dependency
# checks live in the commands that use them, so the rescue surface runs on a bare machine.
test_rescue_without_encoder_or_clipboard() {
    start_test "history and transcribe run without the encoder or the clipboard tools"
    TRANSCRIBE_TEXT="rescued text" td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    # A PATH with what the stubs themselves need, and none of the tools a dictation uses.
    local bare="$WORK/bare-bin" tool path
    rm -rf "$bare"
    mkdir -p "$bare"
    for tool in bash env cp wc sleep cat dd tee date stat awk grep sed ls; do
        path="$(command -v "$tool")" && ln -sf "$path" "$bare/$tool"
    done

    check "the bare PATH has no arecord" bash -c '! PATH="$1" command -v arecord >/dev/null 2>&1' _ "$bare"
    check "the bare PATH has no lame" bash -c '! PATH="$1" command -v lame >/dev/null 2>&1' _ "$bare"
    check "the bare PATH has no wl-copy" bash -c '! PATH="$1" command -v wl-copy >/dev/null 2>&1' _ "$bare"
    check "the bare PATH has no ydotool" bash -c '! PATH="$1" command -v ydotool >/dev/null 2>&1' _ "$bare"

    local bare_env=(PATH="$bare" TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TRANSCRIBE_TEXT="rescued text" \
        XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR")
    local out

    out="$(env "${bare_env[@]}" "$SCRIPT" history)"
    check "history runs without the tools" [ "$(printf '%s\n' "$out" | grep -c .)" -eq 1 ]
    check "history still shows the transcript" bash -c '[[ "$1" == *"rescued text" ]]' _ "$out"

    out="$(env "${bare_env[@]}" "$SCRIPT" transcribe last --print)"
    check "transcribe --print runs without the tools" [ "$out" = "rescued text" ]

    out="$(env "${bare_env[@]}" "$SCRIPT" status)"
    check "status runs without the tools" [ "$out" = "idle" ]

    env "${bare_env[@]}" "$SCRIPT" help > "$WORK/bare-help.out"
    check "help runs without the tools" grep -q 'Usage: tiny-dictate' "$WORK/bare-help.out"
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
        PATH="$BIN:$PATH" TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT_COMMAND="$BIN/present" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" \
            "$script" start >/dev/null 2>&1
        sleep 0.05
        PATH="$BIN:$PATH" TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT_COMMAND="$BIN/present" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" \
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

# A dictation writes one line per stage: the commands with the effect they had, the recording's
# start and end with its duration and bytes, the transcription's start and end with its duration
# and the size of the text, the text being written, and the paste.
test_journal_timeline() {
    start_test "a dictation's journal is its timeline"
    # Both commands append: the session process writes to the same file through a descriptor it
    # inherited from `start`, so a truncating `2>` would leave the process that opened first
    # writing at a stale offset, over the lines the other appended.
    td start 2>> "$JOURNAL"
    sleep 0.4
    td stop 2>> "$JOURNAL"
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]

    local id
    id="$(basename "$(single_session_dir)")"

    check "the start command names its effect" journal_line_names_session "$id" 'start: recording started'
    check "the recording's start is logged" journal_line_names_session "$id" 'recording started'
    check "the stop command names its effect" journal_line_names_session "$id" 'stop: recorder signalled'
    check "the recording's end carries duration and bytes" \
        journal_line_names_session "$id" 'recording ended duration=[0-9.]+s bytes=[0-9]+'
    check "the transcription's start is logged" journal_line_names_session "$id" 'transcription started'
    check "the transcription's end carries duration and text size" \
        journal_line_names_session "$id" 'transcription ended duration=[0-9.]+s chars=16'
    check "the text being written is logged" journal_line_names_session "$id" 'text written chars=16'
    check "the paste's success is logged" journal_line_names_session "$id" 'paste succeeded'
    check "the stages are in order" journal_stages_in_order "$id"
    check "every line names the session" journal_every_line_names_session "$id"
    check "no collaborator wrote into the journal" journal_holds_only_tool_lines
}

# A line carries a session id only when the event concerns a session: the commands that find no
# dictation name none, and neither does a `start` refused before any session exists.
test_journal_nameless_lines() {
    start_test "a line carries a session id only when the event concerns a session"

    td cancel >/dev/null 2> "$WORK/nameless-cancel.err"
    check "cancel says there was nothing to cancel" \
        grep -q '^tiny-dictate: cancel: nothing to cancel$' "$WORK/nameless-cancel.err"
    check "the cancel line carries no id" journal_no_line_names_a_session "$WORK/nameless-cancel.err"

    td stop 2> "$WORK/nameless-stop.err"
    check "stop says there was nothing to stop" \
        grep -q '^tiny-dictate: stop: nothing to stop$' "$WORK/nameless-stop.err"
    check "the stop line carries no id" journal_no_line_names_a_session "$WORK/nameless-stop.err"

    TINY_DICTATE_TRANSCRIBE=does-not-exist td start 2> "$WORK/nameless-start.err"
    check "the refused start is logged as a failure" \
        grep -q '^tiny-dictate: failure: No transcription command' "$WORK/nameless-start.err"
    check "the failure line carries no id" journal_no_line_names_a_session "$WORK/nameless-start.err"
}

# A failure is one line, and the line is the same error the session's metadata holds: the journal
# and the durable trace agree on what went wrong.
test_journal_failure() {
    start_test "a transcription failure is one line, the same the metadata and the notification keep"
    TRANSCRIBE_SLEEP=0.3 TRANSCRIBE_FAIL=1 td start 2>> "$JOURNAL"
    sleep 0.3
    td stop 2>> "$JOURNAL"
    check "session finishes" wait_idle

    local dir id error
    dir="$(single_session_dir)"
    id="$(basename "$dir")"
    error='groq: Error code: 401 - invalid api key'

    check "the failure names the session" journal_line_names_session "$id" "failure: $error"
    check "the line is the metadata's error" journal_failure_matches_metadata "$dir" "$id"
    check "the metadata keeps the transcriber's stderr on one line" \
        [ "$(metadata_error "$dir")" = "\"$error\"" ]
    check "the notification shows the same error" grep -q "Transcription failed: $error" "$WORK/events.log"
    check "nothing was pasted" [ -z "$(pasted_text)" ]
}

# The recorder failing is a failure line too, carrying the session it concerns.
test_journal_recording_failure() {
    start_test "a recording failure is a failure line naming the session"
    ARECORD_FAIL=1 td start 2>> "$JOURNAL"
    check "session finishes" wait_idle

    local dir id
    dir="$(single_session_dir)"
    id="$(basename "$dir")"

    check "the failure names the session" \
        grep -Eq "^tiny-dictate: $id: failure: (no audio was captured|the audio encoder)" "$JOURNAL"
    check "the line is the metadata's error" journal_failure_matches_metadata "$dir" "$id"
    check "the notification carries the same reason" grep -q 'Recording failed:' "$WORK/events.log"
    check "nothing was pasted" [ -z "$(pasted_text)" ]
}

# `toggle` names the effect it had under its own name, not the verb it delegated to.
test_journal_toggle() {
    start_test "toggle logs the effect it had"
    td toggle 2>> "$JOURNAL"
    sleep 0.3

    local id
    id="$(basename "$(single_session_dir)")"
    check "toggle says it started a recording" journal_line_names_session "$id" 'toggle: recording started'
    check "the start path never ran under its own name" journal_lacks "^tiny-dictate: $id: start: "

    td toggle 2>> "$JOURNAL"
    check "session finishes" wait_idle
    check "toggle says it signalled the recorder" journal_line_names_session "$id" 'toggle: recorder signalled'
    check "the recording ran to its end" \
        journal_line_names_session "$id" 'recording ended duration=[0-9.]+s bytes=[0-9]+'
    check "the stop path never ran under its own name" journal_lacks "^tiny-dictate: $id: stop: "
}

# A cancel that lands while the transcription runs drops the text, and the journal says so: the
# transcription ended, and the text never reached the session.
test_journal_late_cancel() {
    start_test "a late cancel is logged as the text being dropped"
    TRANSCRIBE_SLEEP=1 td start 2>> "$JOURNAL"
    sleep 0.3
    td stop 2>> "$JOURNAL"
    sleep 0.2
    td cancel 2>> "$JOURNAL"
    check "session finishes" wait_idle

    local id
    id="$(basename "$(single_session_dir)")"
    check "the cancel is logged" journal_line_names_session "$id" 'cancel: cancelled'
    check "the finished transcription is logged" \
        journal_line_names_session "$id" 'transcription ended duration=[0-9.]+s chars=[0-9]+'
    check "the late cancel is logged as the text being dropped" \
        journal_line_names_session "$id" 'late cancel, text dropped'
    check "the text was never written" journal_lacks 'text written'
    check "nothing was pasted" [ -z "$(pasted_text)" ]
}

# The journal is the tool's own timeline: the encoder's and the recorder's own output is kept out
# of it, in the files the failure path reads -- `arecord.err` and `lame.err`.
test_journal_no_collaborator_chatter() {
    start_test "the encoder's and the recorder's chatter stays out of the journal"
    td start 2>> "$JOURNAL"
    sleep 0.4
    check "the encoder's chatter went to lame.err" \
        grep -q 'LAME' "$RUN/tiny-dictate/session/lame.err"
    check "the recorder's chatter went to arecord.err" \
        grep -q 'arecord: stub chatter' "$RUN/tiny-dictate/session/arecord.err"
    td stop 2>> "$JOURNAL"
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "the dictation wrote its own lines" journal_holds_only_tool_lines
    check "the encoder's banner is not in the journal" journal_lacks 'LAME'
    check "the recorder's banner is not in the journal" journal_lacks 'arecord'
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
run_test test_waveform_channel
run_test test_audio_limit
run_test test_no_audio_limit
run_test test_broken_meter
run_test test_stop_immediately
run_test test_recorder_failure
run_test test_transcription_failure
run_test test_no_transcribe_command
run_test test_no_presenter
run_test test_default_commands
run_test test_cancel
run_test test_chain_after_cancel
run_test test_cancel_during_transcription
run_test test_second_start_refused
run_test test_toggle_during_transcription
run_test test_session_on_disk
run_test test_session_raw_response
run_test test_session_transcriber_writes_nothing
run_test test_session_recording_killed
run_test test_session_audio_survives_kill
run_test test_session_cancelled_keeps_recording
run_test test_journal_timeline
run_test test_journal_nameless_lines
run_test test_journal_failure
run_test test_journal_recording_failure
run_test test_journal_toggle
run_test test_journal_late_cancel
run_test test_journal_no_collaborator_chatter
run_test test_history_lists_sessions_newest_first
run_test test_transcribe_copies_and_rewrites
run_test test_transcribe_print
run_test test_session_delete_removes_one
run_test test_unknown_session_id
run_test test_rescue_without_encoder_or_clipboard
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
