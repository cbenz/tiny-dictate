// tiny-dictate: panel indicator driven by the state file published by
// ~/.local/bin/tiny-dictate-hotkey.
//
//   recording     red dot, blinking
//   transcribing  spinner, pulsing
//   idle          indicator hidden

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const UUID = 'tiny-dictate@cbenz.dev';
const POLL_MS = 400;
// A style swap repaints reliably, where animating the container's opacity did not.
const BRIGHT = 'color: #e01b24; font-size: 15px; padding: 0 3px 0 1px;';
const DIM = 'color: rgba(224, 27, 36, 0.28); font-size: 15px; padding: 0 3px 0 1px;';

function stateDir() {
    const home = GLib.get_home_dir();
    const base =
        GLib.getenv('XDG_STATE_HOME') ||
        GLib.build_filenamev([home, '.local', 'state']);
    return GLib.build_filenamev([base, 'tiny-dictate']);
}

function readState() {
    const path = GLib.build_filenamev([stateDir(), 'state.json']);
    try {
        const [ok, contents] = GLib.file_get_contents(path);
        if (!ok)
            return null;
        const raw = new TextDecoder().decode(contents);
        return JSON.parse(raw);
    } catch (e) {
        return null;
    }
}

function findCli() {
    const local = GLib.build_filenamev([
        GLib.get_home_dir(),
        '.local',
        'bin',
        'tiny-dictate',
    ]);
    if (GLib.file_test(local, GLib.FileTest.IS_EXECUTABLE))
        return local;
    return GLib.find_program_in_path('tiny-dictate');
}

export default class TinyDictateExtension extends Extension {
    enable() {
        this._state = null;
        this._key = null;
        this._blinkSource = 0;
        this._pollSource = 0;
        this._lit = true;

        this._dot = new St.Label({
            text: '●',
            y_align: Clutter.ActorAlign.CENTER,
            style: BRIGHT,
        });

        this._spinner = new St.Icon({
            icon_name: 'process-working-symbolic',
            icon_size: 16,
            style_class: 'system-status-icon',
            y_align: Clutter.ActorAlign.CENTER,
        });

        this._box = new St.BoxLayout({
            style_class: 'panel-status-indicators-box',
        });
        this._box.add_child(this._dot);
        this._box.add_child(this._spinner);

        this._indicator = new PanelMenu.Button(0.0, 'tiny-dictate', false);
        this._indicator.add_child(this._box);
        this._indicator.hide();
        Main.panel.addToStatusArea(UUID, this._indicator, 0, 'right');

        this._info = new PopupMenu.PopupMenuItem('Idle', {reactive: false});
        this._indicator.menu.addMenuItem(this._info);
        this._indicator.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._cancel = this._indicator.menu.addAction(
            'Cancel the dictation',
            () => this._spawn('cancel'),
            null
        );
        this._cancel.setSensitive(false);

        this._copyLast = this._indicator.menu.addAction(
            'Copy the last dictation',
            () => this._spawn('session', 'copy', 'last')
        );

        this._apply(null);
        this._pollSource = GLib.timeout_add(
            GLib.PRIORITY_DEFAULT,
            POLL_MS,
            () => this._poll()
        );
    }

    disable() {
        if (this._pollSource)
            GLib.source_remove(this._pollSource);
        if (this._blinkSource)
            GLib.source_remove(this._blinkSource);
        this._pollSource = 0;
        this._blinkSource = 0;

        this._cancel = null;
        this._copyLast = null;
        this._info = null;
        this._dot = null;
        this._spinner = null;
        this._box = null;

        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }
    }

    _poll() {
        const next = readState();
        const key = `${next?.state ?? 'idle'}:${next?.outcome ?? ''}`;
        if (key !== this._key) {
            this._key = key;
            this._apply(next);
        }
        return GLib.SOURCE_CONTINUE;
    }

    _apply(state) {
        this._state = state;
        const mode = state && state.state ? state.state : 'idle';

        if (this._blinkSource) {
            GLib.source_remove(this._blinkSource);
            this._blinkSource = 0;
        }
        this._lit = true;
        this._box.opacity = 255;

        this._dot.visible = mode === 'recording';
        this._spinner.visible = mode === 'transcribing';
        this._indicator.visible = mode !== 'idle';

        this._cancel.setSensitive(mode !== 'idle');

        if (mode === 'recording') {
            this._setInfo('Recording…');
            this._dot.style = BRIGHT;
            this._blinkSource = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 480, () => {
                this._lit = !this._lit;
                this._dot.style = this._lit ? BRIGHT : DIM;
                this._dot.opacity = this._lit ? 255 : 140;
                return GLib.SOURCE_CONTINUE;
            });
        } else if (mode === 'transcribing') {
            this._setInfo('Transcribing…');
            this._spinner.opacity = 255;
            this._blinkSource = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 260, () => {
                this._lit = !this._lit;
                this._spinner.opacity = this._lit ? 255 : 90;
                return GLib.SOURCE_CONTINUE;
            });
        } else {
            const outcome = state && state.outcome;
            const failed = state && state.error;
            this._setInfo(
                failed
                    ? `Failed: ${failed}`
                    : outcome === 'transcribed'
                      ? 'Last dictation: transcribed'
                      : 'Idle'
            );
        }
    }

    _setInfo(text) {
        try {
            this._info.label.text = text;
        } catch (e) {
            logError(e, 'tiny-dictate: cannot set the menu label');
        }
    }

    _spawn(...args) {
        const cli = findCli();
        if (!cli)
            return;
        try {
            GLib.spawn_async(null, [cli, ...args], null, GLib.SpawnFlags.SEARCH_PATH, null);
        } catch (e) {
            logError(e, 'tiny-dictate: spawn failed');
        }
    }
}
