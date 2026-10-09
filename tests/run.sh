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
# The waybar module belongs to the bar, not to the binary; the suite reads it to keep the bar going
# through the command line instead of reading the state file. The checkout the suite runs from is the
# same: `install.sh` is read from here, not from the built binary.
REPO="$(cd "$(dirname "$0")/.." && pwd)"
WAYBAR_MODULE="$REPO/contrib/waybar/tiny-dictate-module.jsonc"
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
# A second stub directory: the ordinary `wl-copy` exits at once, and only the lease's own case wants
# one that holds the offer, the way `--foreground` does.
BIN_LEASE="$WORK/bin-lease"
# A third stub directory: the waybar module's exec is run with a `tiny-dictate` that prints the
# word it is told to print, so the module's own mapping is what the test observes.
BIN_WAYBAR="$WORK/bin-waybar"
RUN="$WORK/run"
HOME_DIR="$WORK/home"
# The durable sessions live under the state directory. Pointing it at the work directory
# keeps a run hermetic: reset_run wipes it like the runtime directory.
STATE_HOME="$WORK/state"
# The configuration lives under the config directory; pointing it at the work directory keeps a
# run hermetic too, and reset_run wipes it like the rest.
CONFIG_HOME="$WORK/config"
mkdir -p "$BIN" "$RUN" "$HOME_DIR" "$STATE_HOME" "$CONFIG_HOME"
export WORK BIN RUN HOME_DIR STATE_HOME CONFIG_HOME
export XDG_STATE_HOME="$STATE_HOME"
export XDG_CONFIG_HOME="$CONFIG_HOME"

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
printf '%s\\n' "arecord stub pid=\$$ args=\$* at \$(date +%s.%N)" >> "\$WORK/arecord-report"
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
    # A real lame reads as soon as it starts and exits on end-of-file. The stub normally
    # stays alive for a quarter of a second whatever happens, which is long enough for the
    # pump to open the fifo -- hiding a fifo the encoder finds writerless. LAME_STRICT_EOF=1
    # lets the first read decide, the way the real encoder does.
    if [ -n "${LAME_STRICT_EOF:-}" ]; then
        bytes="$(dd bs=1600 count=1 status=none 2>/dev/null | tee -a "$out" | wc -c)"
        if [ "$bytes" -eq 0 ]; then
            report "saw EOF"
            break
        fi
    else
        dd bs=1600 count=1 status=none 2>/dev/null >> "$out"
    fi
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

# Stand-in for the reference transcriber's only collaborator: prints a canned provider body on
# stdout and honours `--output <file>` the way the real curl does, which is how the transcriber
# keeps the raw answer next to the transcript.
cat > "$BIN/curl" <<'STUB'
#!/usr/bin/env bash
out=""
previous=""
for arg in "$@"; do
    [ "$previous" = "--output" ] && out="$arg"
    previous="$arg"
done
body='{"text":"canned transcription"}'
if [ -n "$out" ]; then
    printf '%s' "$body" > "$out"
else
    printf '%s' "$body"
fi
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
# Real GTK warns on stderr. The tool must keep the presenter's chatter out of the journal, so the
# stub says something the suite can look for while the dictation runs.
printf 'presenter: stub chatter\n' >&2
printf 'PRESENT start color=%s owner=%s file=%s waveform=%s caller=%s limit=%s command=%s\n' "${6:-}" "$owner" "$state_file" "$waveform_file" "$PPID" "${5:-}" "${4:-}" >> "$WORK/events.log"
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
# The lease wraps the offer in coreutils' `timeout`; the parent's command line is what makes the
# wrapper observable while `timeout` itself stays the real one.
printf '%s\n' "$(tr '\0' ' ' < "/proc/$PPID/cmdline")" >> "$WORK/wl-copy-parent.log"
STUB

cat > "$BIN/wl-paste" <<'STUB'
#!/usr/bin/env bash
cat "$WORK/clipboard" 2>/dev/null
STUB

cat > "$BIN/ydotool" <<'STUB'
#!/usr/bin/env bash
# The chord is logged with what the clipboard held at that instant: a paste that reads the
# previous dictation's text is visible here, which is what the case about the offer's ordering
# asserts on.
printf '%s\n' "$*" >> "$WORK/ydotool.log"
printf '%s\n' "$(cat "$WORK/clipboard" 2>/dev/null)" >> "$WORK/clipboard-at-chord"
exit 0
STUB

cat > "$BIN/pactl" <<'STUB'
#!/usr/bin/env bash
# Stand-in for the audio server's control command, the only command the microphone module runs.
# `list sources` publishes the source's base volume (PACTL_NO_BASE omits it) and its mute state
# (PACTL_MUTED reports it as muted). Every call is logged with the instant it was made, so the
# suite can check which commands ran, with which arguments, and in which order against the
# recorder. `get-source-mute` and `get-source-volume` stay answered because they are the probes a
# tool that checks before writing would run: the muted-source case asserts that only `set-` calls
# were made.
printf 'pactl %s at %s\n' "$*" "$(date +%s.%N)" >> "$WORK/pactl.log"
case "$1" in
    list)
        printf 'Source #0\n'
        [ -n "${PACTL_MUTED:-}" ] && printf '\tMute: yes\n'
        [ -z "${PACTL_NO_BASE:-}" ] && printf '\tBase Volume: 6554 / 10%% / -60.00 dB\n'
        ;;
    get-source-mute)
        [ -n "${PACTL_MUTED:-}" ] && echo "Mute: yes" || echo "Mute: no"
        ;;
    get-source-volume)
        echo "Volume: front-left: 32768 / 50% / -30.00 dB"
        ;;
esac
exit 0
STUB

# The build toolchain `install.sh` drives and checks: a `cargo` that fakes the release binary it
# links, a `cc` the C-compiler check finds, a `pkg-config` that answers for the two GTK modules
# (TD_MISSING_MODULE names one to report missing), and a `git` whose porcelain listing names the
# suite's fake main worktree (TD_MAIN_WORKTREE) as its first entry.
cat > "$BIN/cargo" <<'STUB'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >> "$WORK/cargo.log"
mkdir -p "$PWD/target/release"
printf '#!/usr/bin/env bash\nexit 0\n' > "$PWD/target/release/tiny-dictate"
chmod +x "$PWD/target/release/tiny-dictate"
STUB

cat > "$BIN/cc" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB

cat > "$BIN/pkg-config" <<'STUB'
#!/usr/bin/env bash
# --exists <module>: the two the build links are installed, unless TD_MISSING_MODULE names one.
if [ "${1:-}" = "--exists" ]; then
    case "${2:-}" in
        gtk4 | gtk4-layer-shell-0)
            [ "${TD_MISSING_MODULE:-}" = "$2" ] && exit 1
            exit 0
            ;;
        *) exit 1 ;;
    esac
fi
exit 0
STUB

cat > "$BIN/git" <<'STUB'
#!/usr/bin/env bash
# `git worktree list --porcelain`: the suite's fake main worktree is the first entry.
printf 'worktree %s\nHEAD 0000000000000000000000000000000000000000\nbranch refs/heads/main\n\n' "${TD_MAIN_WORKTREE:?TD_MAIN_WORKTREE is not set}"
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

# A `wl-copy --foreground` stand-in: it holds the offer until it is killed, the way the real one
# holds the selection for the lease, and its death is what takes the offer off the clipboard. Only
# the lease's own cases use it; `cleanup_processes` reaps it. The text is staged rather than
# published at once, so `WL_COPY_PUBLISH_DELAY` can open the window the real `wl-copy` spends on its
# display roundtrip before it owns the selection: until then the previous offer still serves it.
mkdir -p "$BIN_LEASE"
cat > "$BIN_LEASE/wl-copy" <<'STUB'
#!/usr/bin/env bash
staged="$WORK/clipboard.staged.$$"
cat > "$staged"
printf '%s\n' "$*" >> "$WORK/wl-copy.log"
printf '%s\n' "$(tr '\0' ' ' < "/proc/$PPID/cmdline")" >> "$WORK/wl-copy-parent.log"
trap 'rm -f "$WORK/clipboard" "$staged"; exit 0' TERM INT
[ -n "${WL_COPY_PUBLISH_DELAY:-}" ] && sleep "$WL_COPY_PUBLISH_DELAY"
mv "$staged" "$WORK/clipboard"
while :; do sleep 0.05; done
STUB

# A `tiny-dictate` whose one word is chosen by $FAKE_STATUS, and that logs the verb it was given:
# the module must call it, and call it with `status`.
mkdir -p "$BIN_WAYBAR"
cat > "$BIN_WAYBAR/tiny-dictate" <<'STUB'
#!/usr/bin/env bash
printf 'tiny-dictate %s\n' "$*" >> "$WORK/waybar-report"
printf '%s\n' "${FAKE_STATUS:-idle}"
STUB

chmod +x "$BIN"/* "$BIN-broken-meter"/* "$BIN_LEASE"/* "$BIN_WAYBAR"/*

# A broken stub would silently test nothing: check them before running anything.
for stub in "$BIN"/* "$BIN-broken-meter"/* "$BIN_LEASE"/* "$BIN_WAYBAR"/*; do
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

# Same, with a `wl-copy` that holds the offer for the whole lease instead of exiting at once.
td_lease() {
    TINY_DICTATE_TRANSCRIBE="${TINY_DICTATE_TRANSCRIBE:-$BIN/transcribe}" \
        TINY_DICTATE_PRESENT_COMMAND="${TINY_DICTATE_PRESENT_COMMAND:-$BIN/present}" \
        PATH="$BIN_LEASE:$BIN:$PATH" \
        XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
}

# Same as td, with an encoder that reads the fifo and exits on end-of-file the way the real
# lame does, instead of staying alive long enough for the pump to arrive.
td_strict_encoder() {
    LAME_STRICT_EOF=1 PATH="$BIN:$PATH" \
        TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT_COMMAND="$BIN/present" \
        XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
}

# Same as td, with a waveform meter that dies instead of reading the stream.
td_broken_meter() {
    PATH="$BIN-broken-meter:$BIN:$PATH" \
        TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT_COMMAND="$BIN/present" \
        XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" "$@"
}

# The module is JSONC. Pull its `exec` value out, undo the two JSON escapes it uses (`\\` becomes
# `\`, `\"` becomes `"`), and what is left is the shell command waybar runs.
waybar_module_exec() {
    local line

    line="$(grep -m1 '"exec":' "$WAYBAR_MODULE")"
    line="${line#*\"exec\": \"}"
    line="${line%\",}"
    line="${line//\\\\/\\}"
    printf '%s\n' "${line//\\\"/\"}"
}

# Run that command with a `tiny-dictate` that answers the word given, and return what the bar
# would draw: line 1 the text, line 2 the tooltip, line 3 the CSS class.
waybar_module_output() {
    FAKE_STATUS="$1" PATH="$BIN_WAYBAR:$PATH" bash -c "$(waybar_module_exec)"
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
    rm -rf "$CONFIG_HOME/tiny-dictate"
    : > "$WORK/events.log"
    : > "$WORK/ydotool.log"
    : > "$WORK/clipboard-at-chord"
    : > "$WORK/pactl.log"
    : > "$WORK/wl-copy.log"
    : > "$WORK/wl-copy-parent.log"
    rm -f "$WORK/transcribe-size" "$WORK/transcribe-received" "$WORK/clipboard" \
        "$WORK/arecord-report" "$WORK/lame-report" "$WORK/journal.err" "$WORK/waybar-report"
    rm -f "$WORK"/clipboard.staged.*
}

cleanup_processes() {
    pkill -f "$WORK/bin/" 2>/dev/null
    # The lease's blocking stub holds the selection until it is killed; nothing else may leave it
    # running for the next case.
    pkill -f "$WORK/bin-lease/" 2>/dev/null
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

# The pill is handed the colour it draws with as $6: the waveform and the plain band of the bar.
presenter_got_color() {
    grep -q "^PRESENT start color=$1 " "$WORK/events.log"
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

# The pump never reported the stream stopped, which is what a writerless fifo makes it do.
pump_never_reported_a_stop() {
    ! grep -q 'the waveform pump stopped' "$WORK/encoder.err"
}

# The encoder, checked for liveness: a writerless fifo hands it end-of-file, and a real lame is
# gone on it within milliseconds.
encoder_alive() {
    local pid

    pid="$(cat "$RUN/tiny-dictate/session/lame.pid" 2>/dev/null)" || return 1
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

pasted_text() { cat "$WORK/clipboard" 2>/dev/null; }

pasted() { [ -n "$(pasted_text)" ]; }

# The lease's blocking stub is the offer: while it lives, a process is serving the selection, and
# its death is the lease ending.
lease_offer_alive() { pgrep -f "$WORK/bin-lease/" >/dev/null 2>&1; }
no_lease_offer() { ! lease_offer_alive; }

# Neither a file nor a symlink: what removing a link leaves behind, and what an install that only
# manages symlinks must not replace.
path_absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }

# The session spawns the offer and goes on: the clipboard does not necessarily hold the text the
# instant the command that wrote it returned, and a `session transcribe` has no keypress after the
# offer to give it time. A test that reads the clipboard waits for the text rather than racing the
# process.
wait_pasted_text() {
    local i
    for i in $(seq 1 100); do
        [ "$(pasted_text)" = "$1" ] && return 0
        sleep 0.02
    done
    return 1
}

# What the clipboard held at the instant a chord was injected, read off the `ydotool` stub: the
# last chord is the one that carries the paste, and what it read is the assertion.
chord_clipboard_is() {
    local i
    for i in $(seq 1 100); do
        [ "$(tail -1 "$WORK/clipboard-at-chord" 2>/dev/null)" = "$1" ] && return 0
        sleep 0.02
    done
    return 1
}

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

# Nothing is left reading the waveform channel once the recording is over. The pump's argv
# carries this run's own waveform path, which pins the match: a dictation the user happens to be
# running for real, on the real runtime directory, is none of this test's business.
no_waveform_channel_process() {
    local i

    for i in $(seq 1 20); do
        pgrep -f "_pump $RUN/tiny-dictate/session/waveform" >/dev/null 2>&1 || return 0
        sleep 0.05
    done
    return 1
}

session_dir_gone() { [ ! -e "$RUN/tiny-dictate/session" ]; }

# One field of `session show`'s block: the label column, then its padding, then the value.
shown_field() { # shown_field "<block>" <label>
    sed -n "s/^$2  *//p" <<<"$1"
}

# ─────────────────────── the microphone, at the process seam ───────────────────────

# One `pactl` command, with its arguments, was run: the stub logs every call it answered.
pactl_called() { # pactl_called <command line>
    grep -qF "pactl $1 at " "$WORK/pactl.log"
}

# How many times one `pactl` command ran.
pactl_calls() { # pactl_calls <command>
    grep -c "^pactl $1 " "$WORK/pactl.log" 2>/dev/null || true
}

# The source was set before the recorder opened: both stubs stamp their lines with the instant they
# ran, so the two are comparable.
source_written_before_recorder() {
    local written recorded

    written="$(awk '/^pactl set-source-volume/{print $NF}' "$WORK/pactl.log" | tail -1)"
    recorded="$(awk '/^arecord stub/{print $NF}' "$WORK/arecord-report" | tail -1)"
    [ -n "$written" ] && [ -n "$recorded" ] &&
        awk -v written="$written" -v recorded="$recorded" 'BEGIN { exit !(written < recorded) }'
}

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

# The configuration file the tool reads: `$XDG_CONFIG_HOME/tiny-dictate/config.toml`.
config_file() { printf '%s/tiny-dictate/config.toml' "$CONFIG_HOME"; }

# Write a configuration file, one line per argument. An absent file is the default one.
write_config() {
    mkdir -p "$CONFIG_HOME/tiny-dictate"
    printf '%s\n' "$@" > "$(config_file)"
}

# The session ids on disk, oldest first (the id is the local start timestamp).
session_ids() { ls -1 "$(sessions_dir)" 2>/dev/null | sort; }

# How many sessions are kept: the count of directories under the sessions root.
session_count() { session_ids | grep -c . ; }

# The newest and the oldest session id: `session_ids` already sorts them in time, and the
# newest is the last.
newest_session_id() { session_ids | tail -1; }
oldest_session_id() { session_ids | head -1; }

# The one session directory of the run, when there is exactly one.
single_session_dir() {
    local ids count

    ids="$(ls -1 "$(sessions_dir)" 2>/dev/null)"
    count="$(printf '%s\n' "$ids" | grep -c .)"
    [ "$count" -eq 1 ] || return 1
    printf '%s/%s' "$(sessions_dir)" "$ids"
}

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
    pkill -9 -f "_pump $RUN/tiny-dictate/session/waveform" 2>/dev/null
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

# The encoder reads the fifo the instant it starts, and a fifo with no writer hands it
# end-of-file: the encoder is gone before the pump -- which loads GTK first -- can open the
# write end. `start` must therefore open both ends itself and hand the encoder a reader that
# already has a writer, rather than let the pump be the one that opens the fifo.
test_encoder_finds_a_writer() {
    start_test "the encoder never reads a writerless fifo"
    td_strict_encoder start 2> "$WORK/encoder.err"
    sleep 0.4
    check "the encoder is still there" encoder_alive
    td stop
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "audio given to the transcriber is not empty (was $(audio_size) bytes)" \
        [ "$(audio_size)" -gt 0 ] 2>/dev/null
    check "the pump never reported a stopped stream" pump_never_reported_a_stop
    check "no waveform channel process left behind" no_waveform_channel_process
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
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

# The default source is set before the recorder opens: the tool reads the base volume the audio
# server publishes, writes it back raw, unmutes the source and says so in one line naming the
# session ([ADR-0014](../adr/0014-the-microphone-at-its-base-volume.md)).
test_microphone_base_volume() {
    start_test "the default source is set to its base volume before the recorder opens"
    td start 2>> "$JOURNAL"
    sleep 0.4
    check "the base volume is read from the server" pactl_called "list sources"
    check "the base volume is written back raw" \
        pactl_called "set-source-volume @DEFAULT_SOURCE@ 6554"
    check "the level is written before the recorder opens" source_written_before_recorder
    check "the dictation is recording" [ "$(td status)" = "working" ]
    td stop 2>> "$JOURNAL"
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]

    local id
    id="$(basename "$(single_session_dir)")"
    check "one line names the level it set and the session" \
        journal_line_names_session "$id" 'microphone set to base volume 6554'
    check "the level is set once for the dictation" [ "$(pactl_calls set-source-volume)" -eq 1 ]
    check "no presentation left behind" presenter_left
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

# A source the server publishes as muted is no longer a refusal: the tool unmutes it and dictates,
# and it never reads the mute state to decide anything ([ADR-0014](../adr/0014-the-microphone-at-its-base-volume.md)).
test_microphone_muted_source_records() {
    start_test "a muted source is unmuted instead of refused"
    PACTL_MUTED=1 td start 2>> "$JOURNAL"
    sleep 0.4
    check "the dictation is recording, muted or not" [ "$(td status)" = "working" ]
    check "the source is unmuted" pactl_called "set-source-mute @DEFAULT_SOURCE@ 0"
    check "the mute state is never read" \
        bash -c '! grep -q "^pactl get-source-mute" "$1"' _ "$WORK/pactl.log"
    check "the base volume is still set" \
        pactl_called "set-source-volume @DEFAULT_SOURCE@ 6554"
    td stop 2>> "$JOURNAL"
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "nothing was notified" no_notification_at_all
    check "no muted refusal was logged" journal_lacks 'the microphone is muted'
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

# A server that answers but publishes no base volume is not silence: the level is left alone and
# the journal says why, and the source is unmuted all the same.
test_microphone_without_a_base_volume() {
    start_test "a server that publishes no base volume leaves the level alone"
    PACTL_NO_BASE=1 td start 2>> "$JOURNAL"
    sleep 0.4
    check "the server was asked" pactl_called "list sources"
    check "no volume is written" \
        bash -c '! grep -q "^pactl set-source-volume" "$1"' _ "$WORK/pactl.log"
    check "the source is still unmuted" pactl_called "set-source-mute @DEFAULT_SOURCE@ 0"
    td stop 2>> "$JOURNAL"
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]

    local id
    id="$(basename "$(single_session_dir)")"
    check "one line names the session and why nothing was set" \
        journal_line_names_session "$id" 'microphone left at its level: no base volume published'
    check "no leftover process" no_leftover_process
    check "session directory removed" session_dir_gone
}

test_second_start_refused() {
    start_test "a second start is refused while recording"
    td start
    sleep 0.2
    td start 2> "$WORK/second-start.err"
    check "second start exits non-zero" [ "$?" -ne 0 ]
    check "the refusal names the session it collided with" \
        grep -q "tiny-dictate: $(cat "$RUN/tiny-dictate/session/session.id"): failure: already busy" \
        "$WORK/second-start.err"
    check "recording still running" [ "$(td status)" = "working" ]
    check "the refused start wrote nothing to the source" \
        [ "$(pactl_calls set-source-volume)" -eq 1 ]
    td cancel
    check "session finishes" wait_idle
    check "no presentation left behind" presenter_left
}

# The command lock: `start`, `stop`, `cancel` and `toggle` take it for the duration of the command,
# so two shortcuts pressed at once cannot interleave. A live owner refuses the next command; a lock
# left behind by a dead one is reclaimed. `status` takes no lock.
test_command_lock() {
    start_test "the command lock refuses a second command and reclaims a stale one"
    local lock="$RUN/tiny-dictate/command.lock"

    # A live owner: the command is refused before the claim, and nothing of the dictation is touched.
    mkdir -p "$RUN/tiny-dictate" "$lock"
    printf '%s' "$$" > "$lock/pid"
    td start 2> "$WORK/locked.err"
    check "the refused command exits non-zero" [ "$?" -ne 0 ]
    check "the refusal is notified" \
        grep -q 'already handling another command' "$WORK/events.log"
    check "nothing was claimed" [ ! -e "$RUN/tiny-dictate/session" ]
    check "status is still idle" [ "$(td status)" = "idle" ]

    # A stale lock: its owner is gone, so the next command removes it and takes it.
    printf '%s' '999999999' > "$lock/pid"
    td start
    check "the stale lock is reclaimed and the dictation starts" [ "$(td status)" = "working" ]
    check "the lock is released on the way out" [ ! -e "$lock" ]
    td cancel
    check "session finishes" wait_idle
    check "no leftover process" no_leftover_process
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

# ────────────── the rescue surface under `session`: list, transcribe, delete ──────────────

# `session list` prints the kept sessions newest first, and each line carries the id, the human
# duration, the outcome and the first words of the transcript or the error.
test_session_list_shows_the_kept_sessions() {
    start_test "session list shows the kept sessions newest first with their fields"
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
    lines="$(td session list)"
    check "the listing prints one line per session" [ "$(printf '%s\n' "$lines" | grep -c .)" -eq 2 ]
    first="$(printf '%s\n' "$lines" | head -1)"
    last="$(printf '%s\n' "$lines" | tail -1)"

    check "the newest session is first" bash -c '[[ "$1" == "$2 "* ]]' _ "$first" "$newest"
    check "the oldest session is last" bash -c '[[ "$1" == "$2 "* ]]' _ "$last" "$oldest"
    check "the newest line carries its outcome" bash -c '[[ "$1" == *" transcribed "* ]]' _ "$first"
    check "the newest line carries the first words of the transcript" bash -c '[[ "$1" == *" second text" ]]' _ "$first"
    check "the newest line carries a human duration" bash -c '[[ "$1" =~ [0-9]+:[0-9][0-9] ]]' _ "$first"
    check "the oldest line carries its outcome" bash -c '[[ "$1" == *" failed "* ]]' _ "$last"
    check "the oldest line carries the error" bash -c '[[ "$1" == *"groq: Error code: 401 - invalid api key" ]]' _ "$last"

    # The failed session is what the rescue surface exists for: transcribe it again with the
    # transcriber working, and the text lands in the session that had none.
    TRANSCRIBE_TEXT="rescued text" td session transcribe "$oldest" > /dev/null 2>&1
    check "transcribing the failed session by id succeeds" [ "$?" -eq 0 ]
    check "it now holds the text" [ "$(cat "$(sessions_dir)/$oldest/transcript.txt")" = "rescued text" ]
    check "its outcome became transcribed" [ "$(metadata_value "$(sessions_dir)/$oldest" outcome)" = '"transcribed"' ]

    # A closed stdout is how a Unix tool is expected to leave: `session list | head` must not
    # panic.
    check "the listing piped into head leaves no panic" \
        bash -c '"$1" session list 2>"$2" | head -1 >/dev/null; ! grep -q panicked "$2"' _ "$SCRIPT" "$WORK/list-pipe.err"
}

# A session whose metadata cannot be read is listed rather than skipped: it is exactly the session
# `session delete` can remove, so hiding it would hide a recording the user can get rid of.
test_session_list_shows_an_unreadable_session() {
    start_test "session list shows a session whose metadata cannot be read"

    mkdir -p "$(sessions_dir)/2026-10-07T19-05-22"
    printf '%s' 'not json' > "$(sessions_dir)/2026-10-07T19-05-22/metadata.json"

    local lines
    lines="$(td session list)"
    check "the unreadable session is a row" [ "$(printf '%s\n' "$lines" | grep -c .)" -eq 1 ]
    check "the row is its id and question marks" [ "$lines" = "2026-10-07T19-05-22  ?  ?  ?" ]

    td session delete 2026-10-07T19-05-22 2> "$WORK/unreadable-delete.err"
    check "the unreadable session can be deleted" [ "$?" -eq 0 ]
    check "its directory is gone" [ ! -e "$(sessions_dir)/2026-10-07T19-05-22" ]
}

# ────────────────────────── `session show`: one session read ──────────────────────────

# `session show` reads one session in full: the fields the shared view renders, in one order,
# then the transcript. `last` and a named id are the same read.
test_session_show_prints_the_fields_and_the_transcript() {
    start_test "session show prints the fields and then the transcript of one session"
    TRANSCRIBE_TEXT="hello there, this is the transcript" td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir id out
    dir="$(single_session_dir)"
    id="$(basename "$dir")"

    out="$(td session show last 2> "$WORK/show.err")"
    check "session show succeeds" [ "$?" -eq 0 ]
    check "the id is a field" [ "$(shown_field "$out" id)" = "$id" ]
    check "the path is a field" [ "$(shown_field "$out" path)" = "$dir" ]
    check "the outcome is a field" [ "$(shown_field "$out" outcome)" = "transcribed" ]
    check "the started is a field" bash -c '[[ "$1" =~ ^[0-9]+(\.[0-9]+)?$ ]]' _ "$(shown_field "$out" started)"
    check "the ended is a field" bash -c '[[ "$1" =~ ^[0-9]+(\.[0-9]+)?$ ]]' _ "$(shown_field "$out" ended)"
    check "the duration is the human one" bash -c '[[ "$1" =~ ^[0-9]+:[0-9][0-9]$ ]]' _ "$(shown_field "$out" duration)"
    check "the audio bytes are a field" [ "$(shown_field "$out" 'audio bytes')" = "$(metadata_value "$dir" audio_bytes)" ]
    check "the pasted flag is a field" [ "$(shown_field "$out" pasted)" = "yes" ]
    check "the transcriber is a field" [ "$(shown_field "$out" transcriber)" = "$BIN/transcribe" ]
    check "an absent error is a dash" [ "$(shown_field "$out" error)" = "-" ]
    check "the transcript follows the fields" bash -c '[[ "$1" == *"hello there, this is the transcript" ]]' _ "$out"

    local by_id
    by_id="$(td session show "$id" 2> /dev/null)"
    check "session show <id> prints the same block" [ "$by_id" = "$out" ]

    check "nothing was read from the runtime directory" session_dir_gone
    check "no leftover process" no_leftover_process
}

# A session with no transcript says so rather than printing an empty block: a cancelled dictation
# keeps its recording and was never transcribed.
test_session_show_without_a_transcript() {
    start_test "session show says a session has no transcript"
    td start
    sleep 0.4
    td cancel
    check "session finishes" wait_idle

    local dir out
    dir="$(single_session_dir)"

    out="$(td session show last 2> "$WORK/show-none.err")"
    check "session show succeeds" [ "$?" -eq 0 ]
    check "the fields are still printed" [ "$(shown_field "$out" id)" = "$(basename "$dir")" ]
    check "the outcome is the cancelled one" [ "$(shown_field "$out" outcome)" = "cancelled" ]
    check "the session is said to have no transcript" \
        bash -c '[[ "$1" == *"no transcript" ]]' _ "$out"
    check "no transcript file was written" [ ! -e "$dir/transcript.txt" ]
    check "nothing was copied to the clipboard" [ ! -e "$WORK/clipboard" ]
}

# A directory whose metadata cannot be read says so rather than succeeding with an empty body: it
# shows what it can -- the id and the path -- and the read it was asked for fails.
test_session_show_with_unreadable_metadata() {
    start_test "session show says a session's metadata cannot be read"

    local dir id
    id="2026-10-07T19-05-22"
    dir="$(sessions_dir)/$id"
    mkdir -p "$dir"
    printf '%s' 'not json' > "$dir/metadata.json"

    local out
    out="$(td session show "$id" 2> "$WORK/show-unreadable.err")"
    check "session show exits non-zero" [ "$?" -ne 0 ]
    check "the id is printed" [ "$(shown_field "$out" id)" = "$id" ]
    check "the path is printed" [ "$(shown_field "$out" path)" = "$dir" ]
    check "the block says the metadata cannot be read" \
        bash -c '[[ "$1" == *"the metadata cannot be read"* ]]' _ "$out"
    check "the failure is logged with the session" \
        grep -q "^tiny-dictate: $id: cannot read the session's metadata$" "$WORK/show-unreadable.err"
    check "the metadata is untouched" [ "$(cat "$dir/metadata.json")" = "not json" ]
    check "the session is still there" test -d "$dir"
}

# `session transcribe last` re-runs the transcriber on the newest session, rewrites the transcript
# and the raw response, copies the text to the clipboard and says so.
test_transcribe_copies_and_rewrites() {
    start_test "session transcribe re-runs the transcriber, rewrites the session and copies the text"
    TRANSCRIBE_TEXT="first text" TRANSCRIBE_RAW_TEXT='{"raw":"first"}' td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir id
    dir="$(single_session_dir)"
    id="$(basename "$dir")"
    rm -f "$WORK/clipboard"

    TRANSCRIBE_TEXT="second text" TRANSCRIBE_RAW_TEXT='{"raw":"second"}' \
        td session transcribe last 2> "$WORK/transcribe.err"
    check "transcribe succeeds" [ "$?" -eq 0 ]
    check "the transcript is overwritten" [ "$(cat "$dir/transcript.txt")" = "second text" ]
    check "the raw response is overwritten" [ "$(cat "$dir/transcript.raw.json")" = '{"raw":"second"}' ]
    check "the text is on the clipboard" wait_pasted_text "second text"
    check "the command says the text is on the clipboard" \
        grep -q "^tiny-dictate: $id: transcript copied to the clipboard$" "$WORK/transcribe.err"
    check "the metadata says transcribed" [ "$(metadata_value "$dir" outcome)" = '"transcribed"' ]
    check "no error is recorded" [ "$(metadata_value "$dir" error)" = "null" ]
    check "the transcriber is recorded" [ "$(metadata_value "$dir" transcriber)" = "\"$BIN/transcribe\"" ]
    check "no leftover process" no_leftover_process
}

# `session transcribe --print` writes the text to standard output instead of the clipboard.
test_transcribe_print() {
    start_test "session transcribe --print writes the text to stdout instead of the clipboard"
    TRANSCRIBE_TEXT="first text" td start
    sleep 0.4
    td stop
    check "session finishes" wait_idle

    local dir out
    dir="$(single_session_dir)"
    rm -f "$WORK/clipboard"

    out="$(TRANSCRIBE_TEXT="printed text" td session transcribe last --print 2> "$WORK/print.err")"
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

    td session transcribe does-not-exist 2> "$WORK/unknown-transcribe.err"
    check "session transcribe exits non-zero" [ "$?" -ne 0 ]
    check "session transcribe reports the unknown id" \
        grep -q 'unknown session: does-not-exist' "$WORK/unknown-transcribe.err"

    td session delete does-not-exist 2> "$WORK/unknown-delete.err"
    check "session delete exits non-zero" [ "$?" -ne 0 ]
    check "session delete reports the unknown id" \
        grep -q 'unknown session: does-not-exist' "$WORK/unknown-delete.err"

    td session show does-not-exist > "$WORK/unknown-show.out" 2> "$WORK/unknown-show.err"
    check "session show exits non-zero" [ "$?" -ne 0 ]
    check "session show reports the unknown id" \
        grep -q 'unknown session: does-not-exist' "$WORK/unknown-show.err"
    check "session show reads nothing for an unknown id" [ ! -s "$WORK/unknown-show.out" ]

    check "the session is still there" test -d "$dir"
    check "the transcript is untouched" [ "$(cat "$dir/transcript.txt")" = "kept text" ]
}

# `session list` and `session transcribe` need neither the encoder nor the clipboard tools: the
# dependency checks live in the commands that use them, so the rescue surface runs on a bare
# machine.
test_rescue_without_encoder_or_clipboard() {
    start_test "the session verbs run without the encoder or the clipboard tools"
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

    out="$(env "${bare_env[@]}" "$SCRIPT" session list)"
    check "session list runs without the tools" [ "$(printf '%s\n' "$out" | grep -c .)" -eq 1 ]
    check "the listing still shows the transcript" bash -c '[[ "$1" == *"rescued text" ]]' _ "$out"

    out="$(env "${bare_env[@]}" "$SCRIPT" session transcribe last --print)"
    check "session transcribe --print runs without the tools" [ "$out" = "rescued text" ]

    out="$(env "${bare_env[@]}" "$SCRIPT" session show last)"
    check "session show runs without the tools" \
        bash -c '[[ "$1" == *"rescued text"* ]]' _ "$out"

    out="$(env "${bare_env[@]}" "$SCRIPT" status)"
    check "status runs without the tools" [ "$out" = "idle" ]

    env "${bare_env[@]}" "$SCRIPT" help > "$WORK/bare-help.out"
    check "help runs without the tools" grep -q 'Usage: tiny-dictate' "$WORK/bare-help.out"
}

# The command line's own contract, which the spec keeps in `tests/run.sh` rather than a second
# Rust suite: an unknown verb prints the usage on stderr and exits non-zero, the verbs that only
# query exit zero, and a pidfile whose process is gone reads as idle.
test_command_line() {
    start_test "an unknown verb fails with the usage; the query verbs exit zero"

    td dictate > "$WORK/unknown.out" 2> "$WORK/unknown.err"
    check "an unknown verb exits non-zero" [ "$?" -ne 0 ]
    check "an unknown verb writes nothing to stdout" [ ! -s "$WORK/unknown.out" ]
    check "the usage is printed on stderr" \
        grep -q 'Usage: tiny-dictate <command>' "$WORK/unknown.err"

    # The two verbs that moved under `session` are unknown arguments at the root: the removal is
    # announced by the usage, never silent, and nothing runs.
    local removed
    for removed in history transcribe; do
        td "$removed" last > "$WORK/removed.out" 2> "$WORK/removed.err"
        check "the removed root verb $removed exits non-zero" [ "$?" -ne 0 ]
        check "the removed root verb $removed writes nothing to stdout" [ ! -s "$WORK/removed.out" ]
        check "the removed root verb $removed prints its name and the usage" \
            bash -c 'grep -q "^Unknown argument: $1$" "$2" && grep -q "Usage: tiny-dictate" "$2"' _ "$removed" "$WORK/removed.err"
    done

    # Bare `session` takes no argument of its own: anything after it is a sub-verb or an unknown
    # word, and an unknown one is the usage, never a silent listing.
    td session now > "$WORK/session-now.out" 2> "$WORK/session-now.err"
    check "an unknown sub-verb exits non-zero" [ "$?" -ne 0 ]
    check "an unknown sub-verb writes nothing to stdout" [ ! -s "$WORK/session-now.out" ]
    check "an unknown sub-verb prints the usage" \
        grep -q 'Usage: tiny-dictate <command>' "$WORK/session-now.err"

    # An empty history is an empty listing and not an error.
    td session > "$WORK/session.out" 2> "$WORK/session.err"
    check "bare session exits zero" [ "$?" -eq 0 ]
    check "bare session lists nothing when nothing is kept" [ ! -s "$WORK/session.out" ]

    td status > "$WORK/status.out"
    check "status exits zero" [ "$?" -eq 0 ]
    check "status is idle with no dictation" [ "$(cat "$WORK/status.out")" = "idle" ]

    # A pidfile whose process is gone reads as idle, not as working: liveness is the process, not
    # the file.
    mkdir -p "$RUN/tiny-dictate/session"
    printf '%s' '999999999' > "$RUN/tiny-dictate/session/arecord.pid"
    check "a stale pidfile reads as idle" [ "$(td status)" = "idle" ]

    td help > "$WORK/help.out"
    check "help exits zero" [ "$?" -eq 0 ]
    check "help prints the usage on stdout" \
        grep -q 'Usage: tiny-dictate <command>' "$WORK/help.out"
}

# The bar asks the tool for its status instead of reading the state file: reading the file made an
# internal file an interface for anything outside the binary, which is what #9's criterion asked
# for and #16 reverses (ADR-0013). The module runs `tiny-dictate status` and maps its two words.
test_waybar_module_asks_the_tool() {
    start_test "the waybar module asks the tool instead of reading the state file"

    local exec_line output

    exec_line="$(waybar_module_exec)"
    check "the module has an exec line" test -n "$exec_line"
    check "the module runs tiny-dictate status" \
        grep -q '^tiny-dictate status' <<<"$exec_line"
    check "the module does not read the state file" \
        test -z "$(grep -F 'session/state' "$WAYBAR_MODULE")"

    output="$(waybar_module_output idle)"
    check "the bar called tiny-dictate status" \
        grep -q '^tiny-dictate status$' "$WORK/waybar-report"
    check "idle is three lines" [ "$(printf '%s\n' "$output" | wc -l)" -eq 3 ]
    check "idle draws the microphone" [ "$(sed -n 1p <<<"$output")" = "🎤" ]
    check "idle carries its tooltip" [ "$(sed -n 2p <<<"$output")" = "Dictation idle" ]
    check "idle carries its CSS class" [ "$(sed -n 3p <<<"$output")" = "idle" ]

    : > "$WORK/waybar-report"
    output="$(waybar_module_output working)"
    check "working draws the hourglass" [ "$(sed -n 1p <<<"$output")" = "⏳" ]
    check "working carries its tooltip" [ "$(sed -n 2p <<<"$output")" = "Dictation working…" ]
    check "working carries its CSS class" [ "$(sed -n 3p <<<"$output")" = "working" ]

    check "the click still runs toggle" \
        grep -q '"on-click": "tiny-dictate toggle"' "$WAYBAR_MODULE"
}

# ─────────────── the configuration file and the prune ───────────────

# The number of sessions kept comes from the configuration: after a dictation whose text was
# pasted, the oldest sessions beyond the limit are removed, and the one just recorded is kept.
test_prune_keeps_the_limit() {
    start_test "the prune keeps the configured number of sessions"
    write_config 'keep = 2'

    td start 2>> "$WORK/prune.err"
    sleep 0.4
    td stop
    check "the first dictation finishes" wait_idle

    local first
    first="$(session_ids | head -1)"
    check "the first session is on disk" test -n "$first"

    td start 2>> "$WORK/prune.err"
    sleep 0.4
    td stop
    check "the second dictation finishes" wait_idle
    check "two sessions are within the limit" [ "$(session_count)" -eq 2 ]

    td start 2>> "$WORK/prune.err"
    sleep 0.4
    td stop
    check "the third dictation finishes" wait_idle
    check "the limit is respected" [ "$(session_count)" -eq 2 ]
    check "the oldest session was removed" [ ! -e "$(sessions_dir)/$first" ]
    check "the session just recorded is kept" test -d "$(sessions_dir)/$(session_ids | tail -1)"
    check "the prune is logged" grep -q 'pruned' "$WORK/prune.err"
    check "no leftover process" no_leftover_process
}

# A dictation that failed prunes nothing: the failure is the moment when nothing is deleted,
# and the session it left stays available for `session transcribe`.
test_prune_not_after_failure() {
    start_test "a failed dictation prunes nothing"
    write_config 'keep = 1'

    td start
    sleep 0.4
    td stop
    check "the pasted dictation finishes" wait_idle
    check "one session is kept" [ "$(session_count)" -eq 1 ]

    # Forget the text the first dictation pasted: the failure must not put anything back.
    rm -f "$WORK/clipboard"
    TRANSCRIBE_FAIL=1 td start
    sleep 0.4
    td stop
    check "the failed dictation finishes" wait_idle
    check "the failure pruned nothing" [ "$(session_count)" -eq 2 ]
    check "nothing was pasted" [ -z "$(pasted_text)" ]
    check "no leftover process" no_leftover_process
}

# A cancelled dictation prunes nothing either: it was never pasted, so it is not the moment the
# directory is bounded.
test_prune_not_after_cancel() {
    start_test "a cancelled dictation prunes nothing"
    write_config 'keep = 1'

    td start
    sleep 0.4
    td stop
    check "the pasted dictation finishes" wait_idle
    check "one session is kept" [ "$(session_count)" -eq 1 ]

    td start
    sleep 0.3
    td cancel
    check "the cancelled dictation finishes" wait_idle
    check "the cancel pruned nothing" [ "$(session_count)" -eq 2 ]
    check "no leftover process" no_leftover_process
}

# A crashed run prunes nothing either: a `SIGKILL` never reaches the pasted path, so the
# sessions it left behind stay where they are.
test_prune_not_after_crash() {
    start_test "a crashed run prunes nothing"
    write_config 'keep = 1'

    td start
    sleep 0.4
    td stop
    check "the pasted dictation finishes" wait_idle
    check "one session is kept" [ "$(session_count)" -eq 1 ]

    td start
    sleep 0.3
    kill_run
    check "the crash pruned nothing" [ "$(session_count)" -eq 2 ]
    check "no leftover process" no_leftover_process
}

# The configuration file is optional and never a way to refuse a dictation: an unknown key, a
# malformed file and an unreadable one all fall back to the defaults.
test_config_never_blocks_a_dictation() {
    start_test "an unknown-key, malformed or unreadable config still dictates"

    write_config 'keep = 1' 'unknown_key = "x"'
    td start
    sleep 0.3
    td stop
    check "a dictation with an unknown key finishes" wait_idle
    check "and its text is pasted" [ "$(pasted_text)" = "text transcribed" ]

    write_config 'this is not toml = = ='
    td start
    sleep 0.3
    td stop
    check "a dictation with a malformed config finishes" wait_idle
    check "and its text is pasted" [ "$(pasted_text)" = "text transcribed" ]

    chmod 000 "$(config_file)"
    td start
    sleep 0.3
    td stop
    check "a dictation with an unreadable config finishes" wait_idle
    check "and its text is pasted" [ "$(pasted_text)" = "text transcribed" ]
    chmod 600 "$(config_file)"
    check "no leftover process" no_leftover_process
}

# The pill's colour is read from the configuration by the session and passed to the presenter:
# the default when no file says otherwise, the configured one when it does.
test_pill_color() {
    start_test "the pill takes its colour from the configuration"

    td start
    sleep 0.3
    td stop
    check "the dictation with no config finishes" wait_idle
    check "the default colour reaches the presenter" presenter_got_color "#e4e4e7"

    : > "$WORK/events.log"
    write_config 'color = "#ff0000" # the pill'
    td start
    sleep 0.3
    td stop
    check "the dictation with a colour finishes" wait_idle
    check "the configured colour reaches the presenter" presenter_got_color "#ff0000"
    check "no leftover process" no_leftover_process
}

# ─────────────────────── the clipboard lease ───────────────────────

# The transcript is offered on both selections with the KDE password hint, wrapped in
# `timeout <clipboard_seconds>`: the offer serves the lease, and the hint is what keeps the text
# out of a clipboard history. `--paste-once` is never used — a stolen focus, a paste into the
# wrong window or a clipboard manager reading the data would cost the user the text (ADR-0007).
#
# The stub logs the command line of its own parent, so `timeout` stays the real coreutils and the
# wrapper is still observable.
test_clipboard_lease_invocation() {
    start_test "the offer carries the password hint on both selections, under the default lease"
    TRANSCRIBE_TEXT="leased text" td start
    sleep 0.4
    td stop
    check "the dictation finishes" wait_idle

    check "the CLIPBOARD offer carries the password hint" \
        grep -qx -- '--foreground --sensitive' "$WORK/wl-copy.log"
    check "the PRIMARY offer carries the password hint" \
        grep -qx -- '--primary --foreground --sensitive' "$WORK/wl-copy.log"
    check "the offer is never paste-once" \
        bash -c '! grep -q -- --paste-once "$1"' _ "$WORK/wl-copy.log"
    check "the CLIPBOARD offer is wrapped in the default lease" \
        grep -q -- 'timeout 60 .*wl-copy --foreground --sensitive' "$WORK/wl-copy-parent.log"
    check "the PRIMARY offer is wrapped in the default lease" \
        grep -q -- 'timeout 60 .*wl-copy --primary --foreground --sensitive' "$WORK/wl-copy-parent.log"
}

# The lease's duration is the configuration's `clipboard_seconds`, and `0` means no expiry: no
# `timeout` wrapper at all, the offer then serving until another client takes the selection over.
test_clipboard_lease_duration_from_config() {
    start_test "the lease duration comes from the configuration, and 0 means no expiry"
    write_config 'clipboard_seconds = 7'
    TRANSCRIBE_TEXT="leased text" td start
    sleep 0.4
    td stop
    check "the dictation finishes" wait_idle

    check "the CLIPBOARD offer is wrapped at the configured duration" \
        grep -q -- 'timeout 7 .*wl-copy --foreground --sensitive' "$WORK/wl-copy-parent.log"
    check "the PRIMARY offer is wrapped at the configured duration" \
        grep -q -- 'timeout 7 .*wl-copy --primary --foreground --sensitive' "$WORK/wl-copy-parent.log"

    : > "$WORK/wl-copy.log"
    : > "$WORK/wl-copy-parent.log"
    write_config 'clipboard_seconds = 0'
    TRANSCRIBE_TEXT="leased text" td start
    sleep 0.4
    td stop
    check "the second dictation finishes" wait_idle

    check "the text is still offered" wait_pasted_text "leased text"
    check "a zero duration leaves no timeout wrapper" \
        bash -c '! grep -q "^timeout " "$1"' _ "$WORK/wl-copy-parent.log"
    check "the hint is still carried without a lease" \
        grep -qx -- '--foreground --sensitive' "$WORK/wl-copy.log"
    check "no leftover process" no_leftover_process
}

# `session transcribe` offers the rescued text through the same single path as a dictation, so it
# gets the same lease: the same `--foreground --sensitive` offer, wrapped in the same `timeout`.
test_transcribe_uses_the_lease() {
    start_test "session transcribe offers the rescued text through the same lease"
    write_config 'clipboard_seconds = 4'
    TRANSCRIBE_TEXT="first text" td start
    sleep 0.4
    td stop
    check "the dictation finishes" wait_idle

    : > "$WORK/wl-copy.log"
    : > "$WORK/wl-copy-parent.log"
    rm -f "$WORK/clipboard"
    TRANSCRIBE_TEXT="rescued text" td session transcribe last 2> "$WORK/lease-transcribe.err"
    check "session transcribe succeeds" [ "$?" -eq 0 ]
    check "the rescued text is offered" wait_pasted_text "rescued text"
    check "the session verb offers the CLIPBOARD selection with the hint" \
        grep -qx -- '--foreground --sensitive' "$WORK/wl-copy.log"
    check "the session verb offers the PRIMARY selection with the hint" \
        grep -qx -- '--primary --foreground --sensitive' "$WORK/wl-copy.log"
    check "the session verb's offer is wrapped in the configured lease" \
        grep -q -- 'timeout 4 .*wl-copy --foreground --sensitive' "$WORK/wl-copy-parent.log"
    check "no leftover process" no_leftover_process
}

# The offer serves the lease and then dies, taking the transcript with it: the same process is
# still there, still pasteable, later in the lease, and nothing of it is left once the lease ends.
# What the compositor hands over when the user copies something else is not observable through a
# stub; `wl-copy --foreground` exiting by itself is what leaves that newer copy alone.
test_clipboard_lease_expires() {
    start_test "the offer serves the whole lease, then takes the transcript with it"
    write_config 'clipboard_seconds = 2'
    TRANSCRIBE_TEXT="leased text" td_lease start
    sleep 0.4
    td_lease stop
    check "the dictation finishes" wait_idle

    check "the text is offered" wait_pasted_text "leased text"
    check "a process is serving the offer" lease_offer_alive

    sleep 0.3
    check "the offer is still pasteable later in the lease" [ "$(pasted_text)" = "leased text" ]
    check "the same process is still serving it" lease_offer_alive

    local i
    for i in $(seq 1 60); do
        no_lease_offer && break
        sleep 0.1
    done
    check "the offer is gone once the lease ends" no_lease_offer
    check "the transcript is gone from the clipboard" [ -z "$(pasted_text)" ]
}

# The reported bug: the chord was injected before the offer had taken the selection, so the paste
# read what the previous offer still held. The lease stub stages the new text and publishes it a
# delay later -- the window the real `wl-copy` spends on its display roundtrip -- while `wl-paste`
# keeps serving the previous text, and the `ydotool` stub records what the clipboard held at the
# instant of the chord. Two dictations in a row: the first one is what leaves a live offer behind.
test_paste_waits_for_the_offer() {
    start_test "the paste waits for the offer to serve the text it injects"
    write_config 'clipboard_seconds = 30'
    TRANSCRIBE_TEXT="first text" WL_COPY_PUBLISH_DELAY=0.2 td_lease start
    sleep 0.4
    td_lease stop
    check "the first dictation finishes" wait_idle
    check "the first text is on the clipboard" wait_pasted_text "first text"
    check "the first offer is still serving, the way the lease keeps it" lease_offer_alive

    : > "$WORK/ydotool.log"
    TRANSCRIBE_TEXT="second text" WL_COPY_PUBLISH_DELAY=0.2 td_lease start
    sleep 0.4
    td_lease stop
    check "the second dictation finishes" wait_idle
    check "the second text is on the clipboard" wait_pasted_text "second text"

    check "the chord reads the text of the dictation it belongs to" chord_clipboard_is "second text"
}

# A machine without `wl-paste`: the read-back that waits for the offer cannot be asked, and the
# paste goes on as it did before the wait existed rather than being lost. The PATH holds the stubs
# and the commands the stubs themselves run, minus the one command.
test_paste_without_the_read_back_tool() {
    start_test "the paste is injected when wl-paste is missing"
    local bin="$WORK/bin-no-wl-paste" tool path
    rm -rf "$bin"
    mkdir -p "$bin"
    for tool in bash env cp wc sleep cat dd tee date stat awk grep sed ls tr mv printf kill seq timeout; do
        path="$(command -v "$tool")" && ln -sf "$path" "$bin/$tool"
    done
    for path in "$BIN"/*; do
        case "$(basename "$path")" in wl-paste) continue ;; esac
        ln -sf "$path" "$bin/$(basename "$path")"
    done
    check "the PATH has no wl-paste" bash -c '! PATH="$1" command -v wl-paste >/dev/null 2>&1' _ "$bin"

    PATH="$bin" TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT_COMMAND="$BIN/present" \
        TRANSCRIBE_TEXT="text transcribed" XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" start
    sleep 0.4
    PATH="$bin" TINY_DICTATE_TRANSCRIBE="$BIN/transcribe" TINY_DICTATE_PRESENT_COMMAND="$BIN/present" \
        XDG_RUNTIME_DIR="$RUN" HOME="$HOME_DIR" "$SCRIPT" stop
    check "the dictation finishes" wait_idle
    check "the text is on the clipboard" wait_pasted_text "text transcribed"
    check "the chord was injected without the read-back" chord_clipboard_is "text transcribed"
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

# The journal is the tool's own timeline: the encoder's, the recorder's and the presenter's own
# output is kept out of it, in the files the failure path reads -- `arecord.err`, `lame.err` and
# `presenter.err`.
test_journal_no_collaborator_chatter() {
    start_test "the collaborators' chatter stays out of the journal"
    td start 2>> "$JOURNAL"
    sleep 0.4
    check "the encoder's chatter went to lame.err" \
        grep -q 'LAME' "$RUN/tiny-dictate/session/lame.err"
    check "the recorder's chatter went to arecord.err" \
        grep -q 'arecord: stub chatter' "$RUN/tiny-dictate/session/arecord.err"
    check "the presenter's chatter went to presenter.err" \
        grep -q 'presenter: stub chatter' "$RUN/tiny-dictate/session/presenter.err"
    td stop 2>> "$JOURNAL"
    check "session finishes" wait_idle
    check "transcription pasted" [ "$(pasted_text)" = "text transcribed" ]
    check "the dictation wrote its own lines" journal_holds_only_tool_lines
    check "the encoder's banner is not in the journal" journal_lacks 'LAME'
    check "the recorder's banner is not in the journal" journal_lacks 'arecord'
    check "the presenter's chatter is not in the journal" journal_lacks 'presenter: stub chatter'
}

# The reference transcriber's contract: `$1` the audio file, `$2` the session directory it may
# leave the provider's own answer in as `transcript.raw.json`, stdout the transcript. The provider
# is stubbed at `curl`, so the body is canned and the test is hermetic.
test_reference_transcriber() {
    start_test "the reference transcriber honours \$1, \$2 and stdout"
    local session="$WORK/transcriber-session"
    local audio="$WORK/transcriber-audio.mp3"
    local expected='{"text":"canned transcription"}'
    local out rc
    rm -rf "$session"
    mkdir -p "$session"
    printf 'raw audio bytes' > "$audio"

    rc=0
    out="$(PATH="$BIN:$PATH" GROQ_API_KEY=stub "$REPO/transcribers/groq" "$audio" "$session")" || rc=$?
    check "the transcriber exits zero" [ "$rc" -eq 0 ]
    check "the provider's body is the transcript on stdout" [ "$out" = "$expected" ]
    check "the raw answer is kept verbatim in \$2" \
        [ "$(cat "$session/transcript.raw.json")" = "$expected" ]

    # `$2` is optional: without it the transcriber still transcribes, and leaves nothing behind.
    rm -f "$session/transcript.raw.json"
    rc=0
    out="$(PATH="$BIN:$PATH" GROQ_API_KEY=stub "$REPO/transcribers/groq" "$audio")" || rc=$?
    check "the transcriber works without \$2" [ "$rc" -eq 0 ]
    check "the body is still the transcript" [ "$out" = "$expected" ]
    check "nothing is written without \$2" [ ! -e "$session/transcript.raw.json" ]
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

# ─────────────────────── the dev install ───────────────────────

# `install.sh` is the reversible dev install: one command links this checkout onto PATH, and
# `restore` puts the main worktree's install back. The suite drives a fake checkout, a fake HOME
# and a stub `git`, so the real `~/.local/bin` is never touched.
test_dev_install_and_restore() {
    start_test "the dev install links this checkout, restore puts the main worktree back"

    local checkout="$WORK/install-checkout" main="$WORK/install-main"
    local home="$WORK/install-home" copy_home="$WORK/install-home-copy"
    local dep_home="$WORK/install-home-dep" bare_home="$WORK/install-home-bare"
    local rc=0
    rm -rf "$checkout" "$main" "$home" "$copy_home" "$dep_home" "$bare_home"
    mkdir -p "$checkout" "$home/.local/bin" "$copy_home/.local/bin" "$dep_home/.local/bin" \
        "$bare_home/.local/bin" "$main/src/presenters"
    cp "$REPO/install.sh" "$checkout/install.sh"
    mkdir -p "$checkout/transcribers"
    cp "$REPO/transcribers/groq" "$checkout/transcribers/groq"
    chmod +x "$checkout/install.sh"

    # The main worktree's bash install: restore links back to these two executables.
    printf '#!/usr/bin/env bash\nexit 0\n' > "$main/src/tiny-dictate"
    printf '#!/usr/bin/env bash\nexit 0\n' > "$main/src/presenters/layer-shell"
    chmod +x "$main/src/tiny-dictate" "$main/src/presenters/layer-shell"

    # The bash dev install's presenter link, the one the Rust install must take away, and the
    # user's own wrapper, which neither command may touch.
    ln -s "$main/src/presenters/layer-shell" "$home/.local/bin/tiny-dictate-present"
    printf 'my wrapper\n' > "$home/.local/bin/tiny-dictate-transcribe"

    : > "$WORK/cargo.log"
    PATH="$BIN:$PATH" HOME="$home" TD_MAIN_WORKTREE="$main" "$checkout/install.sh" \
        > "$WORK/install.out" 2> "$WORK/install.err" || rc=$?
    check "install exits zero" [ "$rc" -eq 0 ]
    check "the release build ran" grep -q -- '--release' "$WORK/cargo.log"
    check "the binary link points into this checkout" \
        [ "$(readlink "$home/.local/bin/tiny-dictate")" = "$checkout/target/release/tiny-dictate" ]
    check "the target the link points at is executable" [ -x "$checkout/target/release/tiny-dictate" ]
    check "the stale presenter link is gone" path_absent "$home/.local/bin/tiny-dictate-present"
    check "the reference transcriber is linked from this checkout" \
        [ "$(readlink "$home/.local/bin/tiny-dictate-transcribe")" = "$checkout/transcribers/groq" ]
    check "the custom wrapper is kept as a backup" \
        [ "$(cat "$home/.local/bin/tiny-dictate-transcribe.bak")" = "my wrapper" ]
    check "the installed transcriber answers the audio-limit probe" \
        [ "$("$home/.local/bin/tiny-dictate-transcribe" --print-audio-limit)" = 25000000 ]

    # `restore` rebuilds both links from the main worktree, whose path the porcelain listing names.
    rc=0
    PATH="$BIN:$PATH" HOME="$home" TD_MAIN_WORKTREE="$main" "$checkout/install.sh" restore \
        > "$WORK/restore.out" 2> "$WORK/restore.err" || rc=$?
    check "restore exits zero" [ "$rc" -eq 0 ]
    check "the binary link points at the main worktree" \
        [ "$(readlink "$home/.local/bin/tiny-dictate")" = "$main/src/tiny-dictate" ]
    check "the presenter link points at the main worktree" \
        [ "$(readlink "$home/.local/bin/tiny-dictate-present")" = "$main/src/presenters/layer-shell" ]
    check "restore puts the custom wrapper back" \
        [ "$(cat "$home/.local/bin/tiny-dictate-transcribe")" = "my wrapper" ]
    check "restore leaves no backup behind" path_absent "$home/.local/bin/tiny-dictate-transcribe.bak"

    # A personal wrapper is often a symlink: the backup keeps it a symlink, and restore puts that
    # link back rather than a copy of what it points at.
    rm -f "$home/.local/bin/tiny-dictate-transcribe"
    ln -s "$main/src/tiny-dictate" "$home/.local/bin/tiny-dictate-transcribe"
    rc=0
    PATH="$BIN:$PATH" HOME="$home" TD_MAIN_WORKTREE="$main" "$checkout/install.sh" \
        > "$WORK/install-again.out" 2> "$WORK/install-again.err" || rc=$?
    check "a second install replaces a symlinked wrapper" [ "$rc" -eq 0 ]
    check "the symlinked wrapper is kept as a symlink" \
        [ "$(readlink "$home/.local/bin/tiny-dictate-transcribe.bak")" = "$main/src/tiny-dictate" ]
    rc=0
    PATH="$BIN:$PATH" HOME="$home" TD_MAIN_WORKTREE="$main" "$checkout/install.sh" restore \
        > "$WORK/restore-again.out" 2> "$WORK/restore-again.err" || rc=$?
    check "restore exits zero again" [ "$rc" -eq 0 ]
    check "the wrapper is a symlink again" \
        [ "$(readlink "$home/.local/bin/tiny-dictate-transcribe")" = "$main/src/tiny-dictate" ]

    # A target that vanished is named, and restore exits non-zero rather than half-installing.
    rm "$main/src/presenters/layer-shell"
    rc=0
    PATH="$BIN:$PATH" HOME="$home" TD_MAIN_WORKTREE="$main" "$checkout/install.sh" restore \
        > "$WORK/restore-missing.out" 2> "$WORK/restore-missing.err" || rc=$?
    check "restore fails when a target is gone" [ "$rc" -ne 0 ]
    check "restore names the missing target" \
        grep -q "$main/src/presenters/layer-shell" "$WORK/restore-missing.err"

    # The binary link already exists as a copy: install refuses, names it, and never builds.
    printf '#!/usr/bin/env bash\nexit 0\n' > "$copy_home/.local/bin/tiny-dictate"
    chmod +x "$copy_home/.local/bin/tiny-dictate"
    : > "$WORK/cargo.log"
    rc=0
    PATH="$BIN:$PATH" HOME="$copy_home" TD_MAIN_WORKTREE="$main" "$checkout/install.sh" \
        > "$WORK/refuse.out" 2> "$WORK/refuse.err" || rc=$?
    check "install refuses a copy" [ "$rc" -ne 0 ]
    check "the refusal names the file" grep -q "$copy_home/.local/bin/tiny-dictate" "$WORK/refuse.err"
    check "the copy is left alone" [ -f "$copy_home/.local/bin/tiny-dictate" ]
    check "the copy is still not a symlink" [ ! -L "$copy_home/.local/bin/tiny-dictate" ]
    check "the refusal stopped before the build" [ ! -s "$WORK/cargo.log" ]

    # A missing build dependency stops the install before the build and names the Arch package.
    : > "$WORK/cargo.log"
    rc=0
    TD_MISSING_MODULE=gtk4-layer-shell-0 PATH="$BIN:$PATH" HOME="$dep_home" \
        "$checkout/install.sh" > "$WORK/missing.out" 2> "$WORK/missing.err" || rc=$?
    check "a missing build dependency stops the install" [ "$rc" -ne 0 ]
    check "the Arch package is named" grep -q 'gtk4-layer-shell' "$WORK/missing.err"
    check "the dependency check ran before the build" [ ! -s "$WORK/cargo.log" ]

    # A PATH with the build toolchain and none of the runtime commands: they are listed and
    # nothing more, and the install still runs to the end.
    local bare="$WORK/install-bare-bin" tool path
    rm -rf "$bare"
    mkdir -p "$bare"
    for tool in bash env readlink dirname mkdir rm ln awk chmod; do
        path="$(command -v "$tool")" && ln -sf "$path" "$bare/$tool"
    done
    for tool in cargo cc pkg-config git; do
        ln -sf "$BIN/$tool" "$bare/$tool"
    done
    check "the bare PATH has no arecord" bash -c '! PATH="$1" command -v arecord >/dev/null 2>&1' _ "$bare"

    rc=0
    HOME="$bare_home" PATH="$bare" TD_MAIN_WORKTREE="$main" "$checkout/install.sh" \
        > "$WORK/bare.out" 2> "$WORK/bare.err" || rc=$?
    check "the install runs without the runtime commands" [ "$rc" -eq 0 ]
    check "a missing runtime command is listed" grep -q 'arecord' "$WORK/bare.err"
    check "the listing did not stop the install" [ -L "$bare_home/.local/bin/tiny-dictate" ]
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
run_test test_encoder_finds_a_writer
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
run_test test_microphone_base_volume
run_test test_microphone_muted_source_records
run_test test_microphone_without_a_base_volume
run_test test_second_start_refused
run_test test_command_lock
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
run_test test_reference_transcriber
run_test test_session_list_shows_the_kept_sessions
run_test test_session_list_shows_an_unreadable_session
run_test test_session_show_prints_the_fields_and_the_transcript
run_test test_session_show_without_a_transcript
run_test test_session_show_with_unreadable_metadata
run_test test_transcribe_copies_and_rewrites
run_test test_transcribe_print
run_test test_session_delete_removes_one
run_test test_unknown_session_id
run_test test_rescue_without_encoder_or_clipboard
run_test test_command_line
run_test test_waybar_module_asks_the_tool
run_test test_prune_keeps_the_limit
run_test test_prune_not_after_failure
run_test test_prune_not_after_cancel
run_test test_prune_not_after_crash
run_test test_config_never_blocks_a_dictation
run_test test_pill_color
run_test test_clipboard_lease_invocation
run_test test_clipboard_lease_duration_from_config
run_test test_transcribe_uses_the_lease
run_test test_clipboard_lease_expires
run_test test_paste_waits_for_the_offer
run_test test_paste_without_the_read_back_tool
run_test test_dev_install_and_restore
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
