#!/usr/bin/env python3
"""Native GTK3/VTE acceptance; run with /usr/bin/python3 on a local display.

Uses installed PyGObject/VTE only as development tooling. Commands use VTE's
child-input API; paste uses its real bracketed-paste implementation. This does
not claim physical keyboard, clipboard, other emulator, or SSH acceptance.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import termios
import time

BASE = Path(__file__).resolve().parents[1]


def child(binary, root, report, flags):
    before = termios.tcgetattr(0)
    result = subprocess.run([binary, '--root', root, '--project', 'example', 'tui', *flags])
    restored = termios.tcgetattr(0) == before
    with open(report, 'x', encoding='utf-8') as output:
        json.dump({'exit': result.returncode, 'terminal_restored': restored}, output)
    print('AKASHA_NATIVE_RETURN', flush=True)
    return 0 if result.returncode == 0 and restored else 1


def check(binary, profile, columns, rows, flags, Gtk, Vte, GLib):
    with tempfile.TemporaryDirectory(prefix='akasha-tui-vte-') as folder:
        temp = Path(folder)
        root = temp / 'root'
        shutil.copytree(BASE / 'tests/fixtures/resolution/valid-root', root)
        (temp / 'repository').mkdir()
        report = temp / 'result.json'
        note = root / 'Projects/example/entities/core.md'
        if profile == 'no-color':
            expected_file = temp / 'expected.md'
            replacement_file = temp / 'replacement.md'
            expected_file.write_bytes(note.read_bytes())
            replacement_file.write_bytes(note.read_bytes().replace(b'\n', b'\r\n'))
            subprocess.run([str(binary), '--root', str(root), '--project', 'example',
                            'update-entity', 'Projects/example/entities/core.md',
                            '--expected', str(expected_file), '--replacement', str(replacement_file),
                            '--index', str(root / 'Projects/example/index.md')],
                           check=True, stdout=subprocess.DEVNULL)
        original = note.read_bytes()
        window = Gtk.Window(title=f'Akasha isolated acceptance: {profile}')
        terminal = Vte.Terminal()
        terminal.set_size(columns, rows)
        window.add(terminal)
        window.show_all()
        pid = None
        exited = []
        spawn_errors = []
        terminal.connect('child-exited', lambda _terminal, status: exited.append(status))

        def spawned(_terminal, child_pid, error, _data):
            nonlocal pid
            if error:
                spawn_errors.append(str(error))
            else:
                pid = child_pid

        env = dict(os.environ, XDG_STATE_HOME=str(temp / 'state'))
        env.pop('NO_COLOR', None)
        if profile == 'no-color':
            env['NO_COLOR'] = ''
        # Keep the actual emulator TERM, rather than advertising a multiplexer.
        env['TERM'] = 'xterm-256color'
        argv = [sys.executable, str(Path(__file__).resolve()), '--child', str(binary),
                str(root), str(report), *flags]
        terminal.spawn_async(Vte.PtyFlags.DEFAULT, str(temp), argv,
                             [f'{key}={value}' for key, value in env.items()],
                             GLib.SpawnFlags.DEFAULT, None, None, 10000, None, spawned, None)

        def pump(seconds=0.15):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                while Gtk.events_pending():
                    Gtk.main_iteration_do(False)
                if spawn_errors:
                    raise AssertionError(spawn_errors[0])
                time.sleep(0.005)

        def visible():
            return terminal.get_text(None, None)[0]

        def wait_for(needle, seconds=8):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                pump(0.03)
                if needle in visible():
                    return
                if exited:
                    break
            raise AssertionError(f'{profile}: missing {needle!r}; VTE screen:\n{visible()}')

        def send(data):
            terminal.feed_child(data)
            pump()

        def command(value, editor=False):
            send(b'\x1b[Z\x1b[Z' if editor else b'\x7f')
            send(value.encode() + b'\r')

        def resize(cols, lines):
            padding = terminal.get_style_context().get_padding(Gtk.StateFlags.NORMAL)
            window.resize(cols * terminal.get_char_width() + padding.left + padding.right,
                          lines * terminal.get_char_height() + padding.top + padding.bottom)
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                pump(0.05)
                if terminal.get_column_count() == cols and terminal.get_row_count() == lines:
                    return
            raise AssertionError(f'VTE resize expected {cols}x{lines}, got '
                                 f'{terminal.get_column_count()}x{terminal.get_row_count()}')

        try:
            wait_for('Library loaded')
            resize(columns, rows)
            command('open Projects/example/entities/core.md')
            wait_for('READING')
            command('edit')
            wait_for('SOURCE')
            send(b'\x1b[1;5F')
            # VTE supplies the opening/closing markers based on the application's mode.
            paste = f'\n\nNative VTE {profile}: Привет 世界 e\u0301\nsecond line\n'
            terminal.paste_text(paste)
            pump(0.5)
            wait_for('SOURCE *')
            for size in [(20, 5), (40, 12), (columns, rows)]:
                resize(*size)
            send(b'\x11')
            wait_for('Unsaved changes')
            assert not exited and note.read_bytes() == original
            send(b'\x13')
            wait_for('Saved through the core')
            expected = original + paste.replace('\n', '\r\n' if profile == 'no-color' else '\n').encode()
            assert note.read_bytes() == expected, 'VTE paste must preserve UTF-8 and document newlines'
            terminal.paste_text('discarded native draft')
            pump()
            command('discard', editor=True)
            wait_for('READING')
            assert note.read_bytes() == expected, 'discard must not write'
            command(f'search Native VTE {profile}')
            wait_for('matches')
            command('open Projects/example/entities/core.md')
            wait_for('READING')
            send(b'\x1b')
            wait_for('SEARCH', seconds=1)
            command('quit')
            deadline = time.monotonic() + 8
            while not exited and time.monotonic() < deadline:
                pump(0.05)
            assert exited == [0], f'child exit status: {exited}'
            assert json.loads(report.read_text()) == {'exit': 0, 'terminal_restored': True}
            wait_for('AKASHA_NATIVE_RETURN')
            state = temp / 'state/akasha/tui-navigation-v1.json'
            assert state.stat().st_mode & 0o777 == 0o600
            subprocess.run([str(binary), '--root', str(root), '--project', 'example', 'validate'],
                           check=True, stdout=subprocess.DEVNULL)
            print(f'PASS VTE {profile} {columns}x{rows}: native screen, multiline Unicode paste, '
                  'dirty resize, checked save, discard, search, Escape, exit, exact termios, private state',
                  flush=True)
        finally:
            if pid and not exited:
                try:
                    os.killpg(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                deadline = time.monotonic() + 3
                while not exited and time.monotonic() < deadline:
                    pump(0.05)
                if not exited:
                    try:
                        os.killpg(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            window.destroy()
            pump(0.05)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == '--child':
        os.umask(0o077)
        return child(*sys.argv[2:5], sys.argv[5:])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BASE / 'target/debug/akasha')
    args = parser.parse_args()
    import gi
    gi.require_version('Gtk', '3.0')
    gi.require_version('Vte', '2.91')
    from gi.repository import Gtk, Vte, GLib
    if not Gtk.init_check()[0]:
        parser.error('a working local GTK display is required; no native check ran')
    binary = args.binary.resolve(strict=True)
    print(f'Native VTE {Vte.get_major_version()}.{Vte.get_minor_version()}.{Vte.get_micro_version()}',
          flush=True)
    for profile, columns, rows, flags in [
        ('default', 110, 32, []),
        ('compact-ascii', 40, 12, ['--ascii', '--no-motion', '--no-color']),
        ('no-color', 80, 24, ['--no-motion']),
    ]:
        check(binary, profile, columns, rows, flags, Gtk, Vte, GLib)
    return 0


if __name__ == '__main__':
    sys.exit(main())
