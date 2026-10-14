#!/usr/bin/env bash
#
# The reversible dev install: `./install.sh` builds the release binary and links it onto PATH as
# `tiny-dictate`, links the reference transcriber as `tiny-dictate-transcribe`, and leaves the
# F19/F24 shortcuts and the bar driving this checkout without touching the compositor's
# configuration; `./install.sh restore` puts the main worktree's install back.
#
# Three entries in ~/.local/bin are managed: `tiny-dictate` and the reference transcriber are
# symlinks into this checkout, and the bash implementation's `tiny-dictate-present` link is removed.
# Whatever held `tiny-dictate-transcribe` is kept as `tiny-dictate-transcribe.bak`, once, so a
# personal wrapper is never lost. Nothing is ever installed from a package: a missing package is
# named and the script stops.
set -euo pipefail

script="$(readlink -f -- "${BASH_SOURCE[0]}")"
root="$(dirname -- "$script")"

bin_dir="${HOME}/.local/bin"
binary_link="${bin_dir}/tiny-dictate"
presenter_link="${bin_dir}/tiny-dictate-present"
transcriber_link="${bin_dir}/tiny-dictate-transcribe"
# The previous transcriber is kept beside its link, once: a second install must not overwrite it.
transcriber_backup="${transcriber_link}.bak"

# The reference transcriber shipped with the tool, installed under its default command name.
reference_transcriber_rel="transcribers/groq"

# The main worktree's bash install, whose two executables `restore` links back to.
main_binary_rel="src/tiny-dictate"
main_presenter_rel="src/presenters/layer-shell"

usage() {
    cat <<'EOF'
Usage: install.sh [restore]

  install.sh          build target/release/tiny-dictate, link it and the reference transcriber
  install.sh restore  link the main worktree's tiny-dictate and tiny-dictate-present back, and put
                      the transcriber backup back

`tiny-dictate` and `tiny-dictate-transcribe` become symlinks into this checkout; whatever held the
transcriber before is kept as tiny-dictate-transcribe.bak. A missing build dependency stops the
install; a missing runtime command is only listed.
EOF
}

die() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

# A symlink is the only thing `restore` can undo, so a path that is not one is never replaced.
refuse_a_copy() {
    local path="$1"

    if [ -e "$path" ] && [ ! -L "$path" ]; then
        printf 'install.sh: refusing to overwrite %s: it is not a symlink\n' "$path" >&2
        printf 'install.sh: restore can only undo a symlink, and a copy left by the README is not promised back; remove it and run again\n' >&2
        exit 1
    fi
}

# Keep whatever holds `tiny-dictate-transcribe` before the reference transcriber takes its place:
# a regular file is copied, a symlink is copied as a symlink, and an existing backup is never
# overwritten, so the first install is the one that remembers the user's own command.
backup_transcriber() {
    { [ -e "$transcriber_link" ] || [ -L "$transcriber_link" ]; } || return 0

    if [ -e "$transcriber_backup" ] || [ -L "$transcriber_backup" ]; then
        printf 'install.sh: keeping the existing backup %s\n' "$transcriber_backup"
        return 0
    fi

    if [ -L "$transcriber_link" ]; then
        ln -s -- "$(readlink -- "$transcriber_link")" "$transcriber_backup"
    else
        cp -p -- "$transcriber_link" "$transcriber_backup"
    fi
    printf 'install.sh: kept %s as %s\n' "$transcriber_link" "$transcriber_backup"
}

# `restore` undoes what `install_dev` did to the transcriber and nothing else: the backup goes back
# when there is one, and the link this script made is removed when there is not.
restore_transcriber() {
    if [ -e "$transcriber_backup" ] || [ -L "$transcriber_backup" ]; then
        mv -f -- "$transcriber_backup" "$transcriber_link"
        printf 'install.sh: restored %s from the backup\n' "$transcriber_link"
    elif [ -L "$transcriber_link" ] &&
        [ "$(readlink -- "$transcriber_link")" = "$root/$reference_transcriber_rel" ]; then
        rm -f -- "$transcriber_link"
        printf 'install.sh: removed %s, which this script had linked\n' "$transcriber_link"
    fi
}

# Each build dependency, named with the Arch package that provides it. A missing one stops the
# install before the build, the way a failed build would.
check_build_dependencies() {
    local -a missing=()

    command -v cargo >/dev/null 2>&1 || missing+=('cargo (rust)')
    command -v cc >/dev/null 2>&1 || missing+=('a C compiler (base-devel)')
    pkg-config --exists gtk4 2>/dev/null ||
        missing+=('the pkg-config module gtk4 (gtk4)')
    pkg-config --exists gtk4-layer-shell-0 2>/dev/null ||
        missing+=('the pkg-config module gtk4-layer-shell-0 (gtk4-layer-shell)')

    if [ "${#missing[@]}" -gt 0 ]; then
        printf 'install.sh: missing build dependencies:\n' >&2
        printf 'install.sh:   %s\n' "${missing[@]}" >&2
        exit 1
    fi
}

# The runtime commands a dictation uses. They are listed and nothing more: the tool reports the one
# it needs at the moment it needs it.
check_runtime_dependencies() {
    local -a missing=()
    local command

    for command in arecord lame pactl ydotool notify-send wl-copy wl-paste curl; do
        command -v "$command" >/dev/null 2>&1 || missing+=("$command")
    done

    if [ "${#missing[@]}" -gt 0 ]; then
        printf 'install.sh: missing runtime commands, reported by the tool when they are needed:\n' >&2
        printf 'install.sh:   %s\n' "${missing[@]}" >&2
    fi
}

install_dev() {
    refuse_a_copy "$binary_link"
    check_build_dependencies
    check_runtime_dependencies

    printf 'install.sh: building %s\n' "$root"
    ( cd "$root" && cargo build --release )

    local binary="$root/target/release/tiny-dictate"
    [ -x "$binary" ] || die "the build left no executable at $binary"

    mkdir -p "$bin_dir"
    ln -sfn "$binary" "$binary_link"

    # The binary draws the pill itself, so the bash dev install's presenter link is stale.
    if [ -L "$presenter_link" ]; then
        rm -f "$presenter_link"
    fi

    # The reference transcriber, under the tool's default command name: it answers the
    # `--print-audio-limit` probe the core bounds a dictation with, which an older wrapper did not.
    local transcriber="$root/$reference_transcriber_rel"
    [ -f "$transcriber" ] || die "$transcriber is missing from this checkout"
    backup_transcriber
    ln -sfn "$transcriber" "$transcriber_link"

    printf 'install.sh: installed %s -> %s\n' "$binary_link" "$binary"
    printf 'install.sh: installed %s -> %s\n' "$transcriber_link" "$transcriber"
    printf 'install.sh: it declares its audio limit and reads GROQ_API_KEY from the environment\n'
    printf 'install.sh: a later cargo build --release is picked up without re-running this\n'
}

# The main worktree's path: the first entry of the porcelain listing, never a branch name, so the
# script names the same worktree whatever branch it is on. `restore` deliberately points back at
# that worktree's bash install (`src/tiny-dictate`, `src/presenters/layer-shell`); once the port is
# merged into main those files are gone, and the target check below names them and exits non-zero
# instead of half-installing.
main_worktree() {
    git -C "$root" worktree list --porcelain |
        awk '/^worktree / { sub(/^worktree /, ""); print; exit }'
}

restore_main() {
    local main main_binary main_presenter path
    local -a failed=()

    main="$(main_worktree)"
    [ -n "$main" ] || die 'git worktree list --porcelain named no worktree'

    main_binary="$main/$main_binary_rel"
    main_presenter="$main/$main_presenter_rel"

    # Verify both targets before touching either link: a restore that half-installs is worse than
    # one that refuses.
    for path in "$main_binary" "$main_presenter"; do
        { [ -f "$path" ] && [ -x "$path" ]; } || failed+=("$path")
    done
    if [ "${#failed[@]}" -gt 0 ]; then
        printf 'install.sh: cannot restore, these are missing or not executable:\n' >&2
        printf 'install.sh:   %s\n' "${failed[@]}" >&2
        exit 1
    fi

    refuse_a_copy "$binary_link"
    refuse_a_copy "$presenter_link"

    mkdir -p "$bin_dir"
    ln -sfn "$main_binary" "$binary_link"
    ln -sfn "$main_presenter" "$presenter_link"

    restore_transcriber

    printf 'install.sh: restored %s -> %s\n' "$binary_link" "$main_binary"
    printf 'install.sh: restored %s -> %s\n' "$presenter_link" "$main_presenter"
}

case "${1:-}" in
    '') install_dev ;;
    restore) restore_main ;;
    -h | --help | help) usage ;;
    *)
        usage >&2
        exit 2
        ;;
esac
