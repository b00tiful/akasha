#!/usr/bin/env python3
"""Linux PTY acceptance for Akasha's terminal adapter; standard-library test tooling only."""
import argparse
import codecs
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import struct
import subprocess
import tempfile
import termios
import time
import unicodedata

BASE = Path(__file__).resolve().parents[1]


class Screen:
    """Reconstruct the cursor-addressed cells emitted by the Crossterm backend.

    This is a test observer for that backend, not a general terminal emulator.
    Styles and terminal modes do not affect the text cells we assert against.
    """

    def __init__(self):
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')
        self.pending = ''
        self.cells = {}
        self.row = self.column = 0

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith('\x1b'):
                match = re.match(r'\x1b\[([0-?]*)([ -/]*)([@-~])', self.pending)
                if not match:
                    if self.pending == '\x1b' or self.pending.startswith('\x1b['):
                        break
                    raise AssertionError(f'Unsupported terminal escape: {self.pending[:40]!r}')
                params, _, operation = match.groups()
                self.pending = self.pending[match.end():]
                if params.startswith('?') or operation in 'mhlqn':
                    continue
                values = [int(value) if value else 0 for value in params.split(';')]
                amount = values[0] or 1
                if operation in 'Hf':
                    self.row = amount - 1
                    self.column = ((values[1] or 1) if len(values) > 1 else 1) - 1
                elif operation == 'A':
                    self.row = max(0, self.row - amount)
                elif operation == 'B':
                    self.row += amount
                elif operation == 'C':
                    self.column += amount
                elif operation == 'D':
                    self.column = max(0, self.column - amount)
                elif operation == 'G':
                    self.column = amount - 1
                elif operation == 'J' and values[0] in (2, 3):
                    self.cells.clear()
                elif operation in 'JK':
                    cursor = (self.row, self.column)
                    for position in list(self.cells):
                        if operation == 'K' and position[0] != self.row:
                            continue
                        if (values[0] == 2 or
                                values[0] == 0 and position >= cursor or
                                values[0] == 1 and position <= cursor):
                            del self.cells[position]
                else:
                    raise AssertionError(f'Unsupported CSI: {params}{operation}')
                continue
            char, self.pending = self.pending[0], self.pending[1:]
            if char == '\r':
                self.column = 0
            elif char == '\n':
                self.row += 1
            elif char == '\b':
                self.column = max(0, self.column - 1)
            elif unicodedata.combining(char):
                position = (self.row, max(0, self.column - 1))
                self.cells[position] = self.cells.get(position, '') + char
            elif char >= ' ':
                width = 2 if unicodedata.east_asian_width(char) in 'WF' else 1
                self.cells[self.row, self.column] = char
                if width == 2:
                    self.cells[self.row, self.column + 1] = ''
                self.column += width

    def text(self):
        rows = max((row for row, _ in self.cells), default=0) + 1
        columns = max((column for _, column in self.cells), default=0) + 1
        return '\n'.join(''.join(self.cells.get((row, column), ' ')
                                 for column in range(columns)).rstrip()
                         for row in range(rows))


def check_screen_observer():
    screen = Screen()
    # Text kept from earlier frames must survive cursor-addressed partial changes.
    stream = '\x1b[2J\x1b[1;1HCREATE task\x1b[1;8Hproblem\x1b[2;1H世界'.encode()
    for byte in stream:
        screen.feed(bytes([byte]))
    assert screen.text() == 'CREATE problem\n世界'
    screen.feed(b'\x1b[1;1HHELP\x1b[K')
    assert screen.text() == 'HELP\n世界'
    assert 'CREATE' not in screen.text(), 'old frames must not satisfy current-state checks'
    screen.feed(b'\x1b[2J')
    assert screen.text() == ''


def check(binary, root, agent_home, term, full=False, expect_restore=False):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 110, 0, 0))
    before = termios.tcgetattr(slave)
    def child_setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    env = dict(os.environ, TERM=term)
    env.pop('NO_COLOR', None)
    env['XDG_STATE_HOME'] = str(root.parent / 'state')
    args = [str(binary), '--root', str(root), '--project', 'example', 'tui']
    if not full:
        args += ['--ascii', '--no-motion', '--no-color']
    process = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave,
                               env=env, preexec_fn=child_setup)
    output = bytearray()
    screen = Screen()
    def drain(seconds=0.15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], min(0.03, max(0, deadline-time.monotonic())))[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                output.extend(data)
                screen.feed(data)
                # A PTY supplies transport, not a terminal emulator. Answer the standard
                # cursor-position query just as VTE/xterm do during Ratatui setup.
                if b'\x1b[6n' in data:
                    os.write(master, b'\x1b[1;1R')
        return bytes(output)
    def wait_for(needle, seconds=8):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            drain(0.05)
            visible = screen.text().encode()
            if b''.join(needle.split()) in b''.join(visible.split()):
                return
            if process.poll() is not None:
                break
        raise AssertionError(f'{term}: missing {needle!r}; current screen:\n{screen.text()}')
    def send(data):
        os.write(master, data)
        drain()
    def command(value):
        rows, _, _, _ = struct.unpack('HHHH', fcntl.ioctl(slave, termios.TIOCGWINSZ, b'\0' * 8))
        send(f'\x1b[<0;6;{rows - 3}M\x1b[<0;6;{rows - 3}m'.encode())
        send(value.encode() + b'\r')
    try:
        wait_for(b'Library loaded')
        if expect_restore:
            wait_for(b'Projects/example/records/tasks/pty-created.md')
        assert b'\x1b[?1049h' in output
        assert b'\x1b[?2004h' in output
        assert b'\x1b[?1004h' in output
        assert b'\x1b[?1006h' in output
        if full:
            idle_before = len(output)
            drain(0.7)
            assert len(output) > idle_before, 'ambient frame must change while idle'
            # Focus reporting must stop idle repaint and resume without reopening the vault.
            send(b'\x1b[O')
            paused = len(output)
            drain(0.4)
            assert len(output) == paused, 'unfocused terminal must freeze animation'
            send(b'\x1b[I')
            resumed = len(output)
            drain(0.5)
            assert len(output) > resumed, 'focused terminal must resume animation'
            send(b'/he\t')
            assert process.poll() is None
            send(b'\r')
            # The form shortcuts lengthen HELP enough that COMMANDS is below this
            # 32-row viewport. Observe the post-Enter panel title instead.
            wait_for(b'HELP')
            command('home')
            # Click the first category, then its note, using SGR mouse reports.
            send(b'\x1b[<0;7;6M\x1b[<0;7;6m')
            send(b'\x1b[<0;7;6M\x1b[<0;7;6m')

            wait_for(b'Synthetic entity')
            command('edit')
            wait_for(b'SOURCE')
            # Ctrl-End in the textarea moves to the end of the buffer.
            send(b'\x1b[1;5F')
            note = root / 'Projects/example/entities/core.md'
            original = note.read_bytes()
            paste = '\n\nPTY acceptance: Привет 世界\n'.encode()
            send(b'\x1b[200~' + paste + b'\x1b[201~')
            command('quit')
            wait_for(b'Unsaved changes')
            assert process.poll() is None
            command('save')
            wait_for(b'Saved through the core')
            assert paste in note.read_bytes(), 'pasted Unicode must survive the checked save'
            assert note.read_bytes().startswith(original), 'paste must append without damaging source'
            command('refresh')
            drain(0.3)
            command('search PTY acceptance')
            wait_for(b'matches')
            command('create task')
            wait_for(b'CREATE task')
            for value in [
                'pty-created.md',
                'open',
                '2026-09-17',
                '2026-09-17',
                'PTY created task',
                'Tracks [[Projects/example/entities/core|the core]].',
            ]:
                command(value)
            wait_for(b'REVIEW ROADMAP')
            send(b'\x1b[1;5F')
            projection = b'\n- [[Projects/example/records/tasks/pty-created|PTY created task]]\n'
            send(b'\x1b[200~' + projection + b'\x1b[201~')
            command('save')
            created = root / 'Projects/example/records/tasks/pty-created.md'
            deadline = time.monotonic() + 8
            while not created.is_file() and time.monotonic() < deadline:
                drain(0.05)
            assert created.is_file(), 'creation form must publish the configured task'
            # Successful creation reloads the library and opens the new editable task.
            # The transient status line may be overwritten before a PTY observes it.
            wait_for(b'READING')
            task_before_discard = created.read_bytes()
            roadmap = root / 'Projects/example/roadmap.md'
            roadmap_before_discard = roadmap.read_bytes()
            assert projection in roadmap_before_discard
            command('lifecycle')
            wait_for(b'TASK LIFECYCLE')
            send(b'\x1b[1;5F')
            send(b'\x1b[200~\nDiscarded task draft.\n\x1b[201~')
            send(b'\x0e')  # Ctrl-N: maintained roadmap buffer.
            send(b'\x1b[1;5F')
            send(b'\x1b[200~\nDiscarded roadmap draft.\n\x1b[201~')
            command('discard')
            wait_for(b'READING')
            assert created.read_bytes() == task_before_discard, 'discard must not change task bytes'
            assert roadmap.read_bytes() == roadmap_before_discard, 'discard must not change roadmap bytes'
            command('lifecycle')
            wait_for(b'TASK LIFECYCLE')
            send(b'\x1b[1;5F')
            task_addition = b'\nPTY lifecycle task update.\n'
            send(b'\x1b[200~' + task_addition + b'\x1b[201~')
            send(b'\x0e')
            send(b'\x1b[1;5F')
            roadmap_addition = b'\nPTY lifecycle roadmap update.\n'
            send(b'\x1b[200~' + roadmap_addition + b'\x1b[201~')
            send(b'\x13')  # Ctrl-S applies both buffers through the core.
            wait_for(b'READING')
            assert created.read_bytes() == task_before_discard + task_addition
            assert roadmap.read_bytes() == roadmap_before_discard + roadmap_addition
            assert not list(agent_home.iterdir())
            command(f'integrations codex {agent_home}')
            wait_for(b'INTEGRATIONS')
            wait_for(b'No files were changed')
            assert not list(agent_home.iterdir()), 'read-only integration inspection must not write client-home files'
            command(f'integration apply hook codex {agent_home}')
            wait_for(b'INTEGRATION REVIEW')
            command('discard')
            assert not list(agent_home.iterdir()), 'cancelled review must not write client-home files'
            for operation in ['apply', 'remove']:
                plan_args = [str(binary), '--root', str(root), '--json',
                             'prepare-session-hook', 'codex', '--home', str(agent_home)]
                if operation == 'remove':
                    plan_args.append('--remove')
                plan = json.loads(subprocess.check_output(plan_args))
                command(f'integration {operation} hook codex {agent_home}')
                wait_for(b'INTEGRATION REVIEW')
                command('confirm incorrect')
                wait_for(b'Confirmation must match')
                assert (agent_home / 'hooks.json').exists() == (operation == 'remove')
                command(f'confirm {plan["plan_id"]}')
                wait_for(b'INTEGRATION RESULT')
                wait_for(b'Changed: true')
                assert (agent_home / 'hooks.json').exists() == (operation == 'apply')
            # Resizing must not lose state or crash. Restore usable dimensions afterward.
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 5, 20, 0, 0))
            drain(0.2)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
            drain(0.2)
            assert process.poll() is None
            command('open Projects/example/records/tasks/pty-created.md')
            wait_for(b'Projects/example/records/tasks/pty-created.md')
        else:
            length = len(output)
            drain(0.4)
            assert len(output) == length, 'reduced motion must not repaint an idle screen'
            assert b'\x1b[38;' not in output and b'\x1b[48;' not in output
        command('quit')
        process.wait(timeout=8)
        drain()
        assert process.returncode == 0
        assert b'\x1b[?1049l' in output and b'\x1b[?2004l' in output
        assert b'\x1b[?1004l' in output
        assert b'\x1b[?1006l' in output
        assert termios.tcgetattr(slave) == before, 'raw terminal attributes must be restored exactly'
        print(f'PASS TERM={term}: startup, input, clean exit, terminal restoration' +
              ('; animation, Unicode paste, dirty guard, checked save, search, create/lifecycle forms, integration inspection/cancel/confirm/apply/remove, resize' if full else '; ASCII, no-color, reduced motion'))
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BASE / 'target/debug/akasha')
    args = parser.parse_args()
    check_screen_observer()
    with tempfile.TemporaryDirectory(prefix='akasha-tui-pty-') as folder:
        temp = Path(folder)
        root = temp / 'root'
        shutil.copytree(BASE / 'tests/fixtures/resolution/valid-root', root)
        for template in (BASE / 'tests/fixtures/tui').glob('*.md'):
            shutil.copyfile(template, root / 'Projects/example/templates' / template.name)
        (temp / 'repository').mkdir()
        agent_home = temp / 'codex-home'
        agent_home.mkdir()
        expect_restore = False
        for term in ['xterm-256color', 'xterm', 'linux']:
            full = term == 'xterm-256color'
            check(args.binary.resolve(), root, agent_home, term, full=full,
                  expect_restore=expect_restore)
            expect_restore = True
        state = temp / 'state/akasha/tui-navigation-v1.json'
        assert state.is_file(), 'clean exit must publish navigation state'
        assert state.stat().st_mode & 0o777 == 0o600, 'navigation state must be private'
        subprocess.run([str(args.binary.resolve()), '--root', str(root), '--project', 'example', 'validate'], check=True, stdout=subprocess.DEVNULL)

if __name__ == '__main__':
    main()
