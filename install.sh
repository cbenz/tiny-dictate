#!/usr/bin/env bash
#
# The reversible dev install: `./install.sh` builds the release binary and links it onto PATH as
# `tiny-dictate`, so the F19/F24 shortcuts and the bar drive this checkout without touching the
# compositor's configuration; `./install.sh restore` puts the main worktree's install back.
#
# Only the two symlinks in ~/.local/bin are managed. `tiny-dictate-transcribe` is the user's own
# wrapper and is never touched, and nothing is ever installed: a missing package is named and the
# script stops.
set -euo pipefail

script="$(readlink -f -- "${BASH_SOURCE[0]}")"
root="$(dirname -- "$script")"

bin_dir="${HOME}/.local/bin"
binary_link="${bin_dir}/tiny-dictate"
presenter_link="${bin_dir}/tiny-dictate-present"

# The main worktree's bash install, whose two executables `restore` links back to.
main_binary_rel="src/tiny-dictate"
main_presenter_rel="src/presenters/layer-shell"

usage() {
    cat <<'EOF'
Usage: install.sh [restore]

  install.sh          build target/release/tiny-dictate and link it onto PATH
  install.sh restore  link the main worktree's tiny-dictate and tiny-dictate-present back

Only those two symlinks are touched: tiny-dictate-transcribe is the user's own wrapper. A missing
build dependency stops the install; a missing runtime command is only listed.
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

    for command in arecord lame ydotool notify-send wl-copy wl-paste curl; do
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

    printf 'install.sh: installed %s -> %s\n' "$binary_link" "$binary"
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
