# The automation

The tool is driven by running a command, and nothing else. A keyboard shortcut, a key remapper and a
bar module are three ways of running that command or reading its status; none of them is part of the
tool.

## The status the tool exposes

`tiny-dictate status` prints `idle` or `working`. It is meant to be polled, and it changes nothing.
A bar module, a prompt or a script can read it as often as it likes.

## Keyboard shortcuts

Every desktop environment that binds a key to a command can drive the tool:

```text
bindsym $mod+backslash exec tiny-dictate toggle
bindsym $mod+Shift+backslash exec tiny-dictate cancel
```

`toggle` covers the common case — one key for start and stop. `cancel` needs its own key, because a
dictation ended by accident must remain cancellable without being transcribed.

## keyd

A key remapper is useful when the shortcut has to be a chord the compositor cannot see, or a
double-tap. On the reference setup the double-tap detection lives in
`/etc/keyd/default.conf`, outside this repository: a single tap of Super does nothing, a second tap
within `oneshot_timeout` emits `F19`, and a tap with Shift held emits `F24`. The compositor then
binds those two plain function keys:

```text
bindsym F19 exec systemd-cat -t tiny-dictate tiny-dictate toggle
bindsym F24 exec systemd-cat -t tiny-dictate tiny-dictate cancel
```

`systemd-cat -t tiny-dictate` tags the tool's stderr in the journal, which is where
[the log](../adr/0008-the-log.md) reads as a timeline. An untagged line is what `tiny-dictate` writes
when it is run directly.

Two details of that setup are worth keeping in mind when choosing keys:

- `F24`, not `F20`: the evdev keymap maps the twentieth function key to `XF86AudioMicMute`, so a
  binding on `F20` never matches, and a binding on `XF86AudioMicMute` mutes the microphone instead.
- `F24`, not `Shift+F19`: the compositor only sees the Shift it has been told about, so a Shift
  released a few milliseconds before the tap turns a cancel into a toggle. The key is chosen by the
  keyd layer, which is where the Shift is known.

Both are configuration outside the repository; the repository ships no keyd or sway configuration.

## Waybar

`contrib/waybar/tiny-dictate-module.jsonc` puts the dictation state in the bar: `tiny-dictate status`
prints `idle` or `working`, and the module maps either to an icon, a tooltip and a CSS class. A click
runs `tiny-dictate toggle`.

```bash
cp contrib/waybar/tiny-dictate-module.jsonc ~/.config/waybar/
```

Then include the fragment and add the module to a bar:

```jsonc
{
  "include": ["tiny-dictate-module.jsonc"],
  "modules-right": ["custom/tiny-dictate"]
}
```

The module polls `status` once a second and on every bar event. The icons are mapped in the `exec`
line with `sed`, because waybar's `format-icons` requires a JSON `alt` field that `status` does not
emit: line 1 of the output is the text, line 2 the tooltip, line 3 the CSS class.

The bar is a standing reminder that dictation is on; the [pill](pill.md) is what shows a dictation
while it runs, and what reports a cancellation.
