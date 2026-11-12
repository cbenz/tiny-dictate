# GNOME

Mutter has no `wlr-layer-shell`, so the pill is never drawn here: a dictation works, nothing shows
while it runs. This directory replaces what the pill does on a wlroots compositor — the shortcut and
the state on screen — with what GNOME offers: an evdev watcher for the shortcut, a panel indicator
for the state.

Nothing here is needed to dictate. It is the integration that makes GNOME usable, kept out of the
tool because the tool needs none of it.

## What it installs

| Piece | Where | What it does |
|---|---|---|
| `bin/tiny-dictate-hotkey` | `~/.local/bin` | listens on the keyboards, read-only: double tap of Right Ctrl runs `tiny-dictate start`/`stop`, Shift + tap runs `cancel`. Publishes `state.json` |
| `bin/ydotool-0.1.8` | `~/.local/bin/ydotool` | only on a distribution whose ydotool predates the `<code>:<state>` syntax (see below) |
| `systemd/*.service` | `~/.config/systemd/user` | `ydotoold` (pasting) and the watcher, both as user units |
| `udev/*.rules` | `/etc/udev/rules.d` | `/dev/uinput` for the active seat's user, and read access to the keyboards |
| `gnome-shell-extension/tiny-dictate@cbenz.dev` | `~/.local/share/gnome-shell/extensions` | red dot while recording, pulsing spinner while transcribing, hidden at rest |

```bash
./contrib/gnome/install.sh
```

then **log out and back in once**: GNOME Shell reads its extension list at startup, so a newly
installed indicator cannot appear before that. `gnome-extensions info tiny-dictate@cbenz.dev` answers
`ENABLED` afterwards.

## Why the watcher reads the keyboard instead of binding a key

The reference setup for a double tap is a key remapper (`keyd`) turning it into a key the compositor
binds. Two things stop that here:

- `keyd` is not packaged in Ubuntu 24.04, and
- it has no double-tap primitive at all: its actions are tap/hold (`overload`, `overloadt`,
  `timeout`), and its only timing knob is `overload_tap_timeout`. `42:1`-style double taps cannot be
  expressed.

The watcher does the timing itself, takes no key away from the keyboard (Right Ctrl stays Right
Ctrl), and needs no remapper: it opens the keyboards read-only, and the uaccess rule is what makes
that possible without adding the user to the `input` group.

`tiny-dictate status` cannot tell recording from transcribing — it answers `working` for both — so
the watcher reads the newest session's `metadata.json` next to it: `outcome` is `recording` only
while the microphone is open.

## The ydotool 0.1.8 shim

`tiny-dictate` presses its chord with `ydotool key 42:1` / `110:1 110:0` / `42:0`, the
`<code>:<state>` syntax of ydotool 1.0 and later. The C++ ydotool 0.1.8 that Debian and Ubuntu ship
takes key names (`ydotool key Shift_L+Insert`) and, given `<code>:<state>`, reads the token as a name,
types its first character, and exits 0 — the paste then reports success while inserting `'&&'` on an
AZERTY layout.

`install.sh` detects that version from its own help text and installs the translating shim under the
`ydotool` name, which shadows the system binary for everything the session runs. Remove
`~/.local/bin/ydotool` once the distribution ships a ydotool that takes `<code>:<state>`.

## Requirements

- `python3` and `python3-evdev` — the watcher
- `libnotify` — failures are notified, there is no pill to show them
- `ydotool` + `ydotoold`, `wl-clipboard` 2.3.0 or later, `arecord`, `lame`, `curl` — the tool itself
- a compositor without `wlr-layer-shell` is fine; the pill is simply absent

## Removing it

```bash
systemctl --user disable --now tiny-dictate-hotkey.service ydotoold.service
rm -f ~/.local/bin/tiny-dictate-hotkey ~/.local/bin/ydotool
rm -rf ~/.local/share/gnome-shell/extensions/tiny-dictate@cbenz.dev
sudo rm -f /etc/udev/rules.d/60-uinput.rules /etc/udev/rules.d/70-input-uaccess.rules
```
