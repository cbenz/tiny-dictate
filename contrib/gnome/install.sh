#!/bin/bash
# Install the GNOME integration: the double-tap watcher, its units, the udev rules and the
# panel indicator. Idempotent; re-run it after editing a file.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
uid=$(id -u)
bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
extension_dir="$HOME/.local/share/gnome-shell/extensions/tiny-dictate@cbenz.dev"
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/tiny-dictate"

install -d "$bin_dir" "$unit_dir" "$extension_dir" "$state_dir"

install -m755 "$here/bin/tiny-dictate-hotkey" "$bin_dir/tiny-dictate-hotkey"
install -m644 "$here/systemd/"*.service "$unit_dir/"
install -m644 "$here/gnome-shell-extension/tiny-dictate@cbenz.dev/"* "$extension_dir/"

# The C++ ydotool 0.1.8 announces `key sequence`, the versions that take `<code>:<state>` do not:
# only the former needs the translating shim.
if /usr/bin/ydotool key --help 2>&1 | grep -q 'key sequence'; then
    install -m755 "$here/bin/ydotool-0.1.8" "$bin_dir/ydotool"
    printf '✓ %s/ydotool translates the chord for ydotool 0.1.8\n' "$bin_dir"
fi

# The clipboard lease's flag needs wl-clipboard 2.3.0 or later.
if command -v wl-copy >/dev/null && ! wl-copy --help 2>&1 | grep -q -- '--sensitive'; then
    printf '! %s rejects --sensitive: the paste will fail until wl-clipboard 2.3.0 is installed\n' \
        "$(command -v wl-copy)"
fi

# ydotoold needs to write /dev/uinput, which logind only unlocks for the session that owns the
# seat; the watcher only listens, and uaccess gives it the keyboards it reads.
sudo install -m644 "$here/udev/60-uinput.rules" /etc/udev/rules.d/60-uinput.rules
sudo install -m644 "$here/udev/70-input-uaccess.rules" /etc/udev/rules.d/70-input-uaccess.rules
printf 'uinput\n' | sudo tee /etc/modules-load.d/uinput.conf >/dev/null
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input --action=change
sudo udevadm trigger /dev/uinput

systemctl --user daemon-reload
systemctl --user enable --now ydotoold.service tiny-dictate-hotkey.service

# GNOME Shell reads its extension list once, at startup: enabling the uuid now is what makes the
# indicator appear after the next login, since `gnome-extensions enable` cannot see an extension the
# running shell has never scanned.
current=$(gsettings get org.gnome.shell enabled-extensions)
case "$current" in
    *tiny-dictate@cbenz.dev*) ;;
    *)
        gsettings set org.gnome.shell enabled-extensions \
            "${current%]}, 'tiny-dictate@cbenz.dev']"
        ;;
esac

cat <<EOF
✓ watcher, units, udev rules and indicator installed

Two things are left on your side:
  - the shortcut: nothing to bind, the watcher listens for a double tap of Right Ctrl on its own
    (Shift + tap cancels), and it starts and stops the dictation through tiny-dictate
  - log out and back in once, so GNOME Shell loads the indicator

state file the indicator reads: $state_dir/state.json
EOF
