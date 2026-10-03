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


def check(binary, root, agent_home, term, full=False, expect_restore=False,
          keyboard=False, size=(32, 110), env_no_color=False, split_paste_start=False,
          recovery_start=False):
    def snapshot():
        return {p: p.read_bytes() for p in root.rglob('*') if p.is_file()}

    def stage_recovery():
        # Derive authentic note/state images through the checked product writer.
        identity = 'Projects/example/entities/core.md'
        note = root / identity
        state = root / 'Projects/example/.akasha-state.toml'
        before, state_before = note.read_bytes(), state.read_bytes()
        after = before + '\nInterrupted recovery: Привет 世界  \n'.encode()
        expected = root.parent / 'recovery-expected.md'
        replacement = root.parent / 'recovery-replacement.md'
        expected.write_bytes(before)
        replacement.write_bytes(after)
        subprocess.run([str(binary), '--root', str(root), '--project', 'example',
                        'update-entity', identity, '--expected', str(expected),
                        '--replacement', str(replacement), '--index',
                        str(root / 'Projects/example/index.md')],
                       check=True, stdout=subprocess.DEVNULL)
        state_after = state.read_bytes()
        state.write_bytes(state_before)
        note.write_bytes('External editor: 世界  \r\n'.encode())
        journal = root / 'Projects/example/.akasha-edit-journal.json'
        journal.write_text(json.dumps(dict(schema_version=1, project='example', id=identity,
                                          note_before=before.decode(), note_after=after.decode(),
                                          state_before=state_before.decode(), state_after=state_after.decode())))
        journal.chmod(0o600)
        return note, journal, before, after

    if recovery_start:
        recovery_note, journal, recovery_before, recovery_after = stage_recovery()
        interrupted = snapshot()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', *size, 0, 0))
    before = termios.tcgetattr(slave)
    def child_setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    env = dict(os.environ, TERM=term)
    env.pop('NO_COLOR', None)
    if env_no_color:
        env['NO_COLOR'] = ''  # Presence, including an empty value, disables color.
    env['XDG_STATE_HOME'] = str(root.parent / 'state')
    args = [str(binary), '--root', str(root), '--project', 'example', 'tui']
    if not full:
        args += ['--ascii', '--no-motion']
        if not env_no_color:
            args += ['--no-color']
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
    def wait_for_ready(seconds=8):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            drain(0.05)
            lines = screen.text().splitlines()
            if len(lines) > 1 and 'AKASHA' in lines[1] and lines[1].rstrip().endswith('ready'):
                return
            if process.poll() is not None:
                break
        raise AssertionError(f'{term}: TUI did not finish startup; current screen:\n{screen.text()}')
    def wait_for_refusal(seconds=8):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            drain(0.05)
            lines = screen.text().splitlines()
            if (len(lines) > 1 and 'AKASHA' in lines[1]
                    and not lines[1].rstrip().endswith('working')
                    and ('Operation failed' in screen.text() or 'external change' in lines[1])):
                return
            if process.poll() is not None:
                break
        raise AssertionError(f'{term}: stale apply did not finish refusing; current screen:\n{screen.text()}')
    def send(data):
        os.write(master, data)
        drain()
    def command(value, from_editor=False):
        # A retained READING title can render during save/load/reopen. Wait for the
        # serialized worker before focusing the prompt, or reopen can steal focus.
        deadline = time.monotonic() + 8
        while True:
            lines = screen.text().splitlines()
            if len(lines) > 1 and 'AKASHA' in lines[1] and not lines[1].rstrip().endswith('working'):
                break
            assert process.poll() is None, 'TUI exited before command injection'
            assert time.monotonic() < deadline, 'TUI worker did not finish before command injection'
            drain(0.05)
        if keyboard:
            # Reader -> list -> prompt; Backspace focuses prompt outside editing.
            # No mouse/focus reports or function keys are needed on this path.
            send(b'\x1b[Z\x1b[Z' if from_editor else b'\x7f')
        else:
            rows, _, _, _ = struct.unpack('HHHH', fcntl.ioctl(slave, termios.TIOCGWINSZ, b'\0' * 8))
            send(f'\x1b[<0;6;{rows - 3}M\x1b[<0;6;{rows - 3}m'.encode())
        send(value.encode() + b'\r')
    def create_inputs(note_type, values):
        command(f'create {note_type}')
        wait_for(f'CREATE {note_type}'.encode())
        send(values[0].encode() + b'\r')
        wait_for(b'(1/')  # Named template-field editor, with its configured field count.
        for value in values[1:]:
            send(b'\x1b[200~' + value.encode() + b'\x1b[201~')
            send(b'\x0e')
        wait_for(b'EDIT INDEX' if note_type == 'entity' else b'EDIT ROADMAP')

    def create_review(note_type):
        send(b'\x0e')
        wait_for(b'REVIEW EXACT NOTE')
        send(b'\x0e')
        wait_for(b'REVIEW EXACT INDEX' if note_type == 'entity' else b'REVIEW EXACT ROADMAP')

    def lifecycle_review(label):
        send(b'\x0e')
        wait_for(b'REVIEW EXACT NOTE')
        send(b'\x0e')
        wait_for(b'REVIEW EXACT ' + label)

    def displayed_plan_id():
        # Read the actual review one wrapped screen line at a time, including
        # the single-row reader at 40x12; do not regenerate the core plan hash.
        rows, _, _, _ = struct.unpack('HHHH', fcntl.ioctl(slave, termios.TIOCGWINSZ, b'\0' * 8))
        first_body = 5 if rows >= 13 else 4
        collected = ''
        send(b'\x1b[H')
        for _ in range(100):
            visible = re.search(r'PlanID:(sha256:[0-9a-f]{64})',
                                re.sub(r'\s+', '', screen.text()))
            if visible:
                return visible.group(1)
            collected += screen.text().splitlines()[first_body].strip()
            match = re.search(r'Plan ID: ?(sha256:[0-9a-f]{64})', collected)
            if match:
                return match.group(1)
            send(b'\x1b[B')
        raise AssertionError(f'{term}: plan ID not reachable through reader scrolling: {collected}')

    def project_initialization_review():
        slug = f'init-{term}-{"keyboard" if keyboard else "full"}'
        repository = root.parent / f'{slug} repository 世界'
        repository.mkdir()
        pointer = repository / '.akasha.toml'
        project = root / 'Projects' / slug
        prepare = f'init {slug} {repository}'
        before_init = snapshot()
        command(prepare)
        wait_for(b'PROJECT INITIALIZATION REVIEW')
        plan_id = displayed_plan_id()
        for value in ['confirm incorrect', 'save', 'quit', 'refresh']:
            command(value)
        assert process.poll() is None and snapshot() == before_init
        command('discard')
        assert snapshot() == before_init and not pointer.exists() and not project.exists()
        command(prepare)
        wait_for(b'PROJECT INITIALIZATION REVIEW')
        assert displayed_plan_id() == plan_id
        # A valid but changed registry invalidates the complete reviewed replacement.
        registry = root / 'Meta/projects.yaml'
        registry.write_bytes(registry.read_bytes() + b'\n# External registry edit\n')
        external = snapshot()
        command(f'confirm {plan_id}')
        wait_for(b'PROJECT INITIALIZATION RESULT')
        assert not pointer.exists() and not project.exists()
        lock = root / 'Meta/.projects.yaml.akasha-init.lock'
        assert {p: b for p, b in snapshot().items() if p != lock} == {p: b for p, b in external.items() if p != lock}
        command(prepare)
        wait_for(b'PROJECT INITIALIZATION REVIEW')
        fresh_id = displayed_plan_id()
        assert fresh_id != plan_id
        command(f'confirm {fresh_id}')
        wait_for(b'PROJECT INITIALIZATION RESULT')
        wait_for_ready()
        assert pointer.read_bytes() == f'schema_version = 1\nproject = "{slug}"\n'.encode()
        assert (project / 'index.md').read_bytes() == b''
        assert (project / 'roadmap.md').read_bytes() == b''
        assert not (root / 'Meta/.projects.yaml.akasha-init-journal.json').exists()
        resolved = json.loads(subprocess.check_output(
            [str(binary), '--root', str(root), '--json', 'resolve'], cwd=repository))
        assert resolved['project'] == slug and resolved['pointer'] == str(pointer)
        subprocess.run([str(binary), '--root', str(root), '--project', slug, 'validate'],
                       check=True, stdout=subprocess.DEVNULL)
        command('refresh')
        wait_for_ready()
        command(f'project {slug}')
        wait_for(slug.encode())
        command('project example')

    def repository_link_review():
        repository = root.parent / 'repository'
        pointer = repository / '.akasha.toml'
        expected = b'schema_version = 1\nproject = "example"\n'
        if pointer.exists():
            assert pointer.read_bytes() == expected
            pointer.unlink()  # Reset only this disposable fixture between profiles.
        before_link = snapshot()
        prepare = f'link example {repository}'
        command(prepare)
        wait_for(b'REPOSITORY LINK REVIEW')
        command('discard')
        assert not pointer.exists() and snapshot() == before_link
        command(prepare)
        wait_for(b'REPOSITORY LINK REVIEW')


        plan_id = displayed_plan_id()
        command('confirm incorrect')
        command('save')
        command('quit')
        assert process.poll() is None and not pointer.exists() and snapshot() == before_link
        # A concurrent human pointer must survive confirmation unchanged.
        pointer.write_bytes(b'Human pointer\r\n')
        command(f'confirm {plan_id}')
        wait_for(b'REPOSITORY LINK RESULT')
        assert pointer.read_bytes() == b'Human pointer\r\n' and snapshot() == before_link
        pointer.unlink()
        command(prepare)
        wait_for(b'REPOSITORY LINK REVIEW')
        assert displayed_plan_id() == plan_id
        command(f'confirm {plan_id}')
        wait_for(b'REPOSITORY LINK RESULT')
        assert pointer.read_bytes() == expected and snapshot() == before_link
        resolved = json.loads(subprocess.check_output(
            [str(binary), '--root', str(root), '--json', 'resolve'], cwd=repository))
        assert resolved['project'] == 'example' and resolved['pointer'] == str(pointer)

    try:
        if recovery_start:
            wait_for(b'RECOVERY INSPECTION')
            wait_for(b'recovery')
            assert snapshot() == interrupted, 'initial refusal must preserve all files'
            for value in ['recovery', 'refresh']:
                command(value)
                wait_for(b'RECOVERY INSPECTION')
                assert snapshot() == interrupted, 'inspection and retry must preserve conflict bytes'
            # Only the fixture operator reconciles bytes. The TUI then runs core rollback.
            recovery_note.write_bytes(recovery_after)
            command('refresh')
            wait_for(b'Core recovery completed: RolledBack')
            wait_for_ready()
            assert recovery_note.read_bytes() == recovery_before and not journal.exists()
        else:
            wait_for(b'Library loaded')
        if expect_restore:
            # The note path is visible in its selected list row before the
            # asynchronous source load completes. Wait for the reader frame.
            wait_for(b'READING')
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
            create_inputs('task', [
                'pty-created.md', 'open', '2026-09-17', '2026-09-17',
                'PTY created task', 'Tracks [[Projects/example/entities/core|the core]].',
            ])
            send(b'\x1b[1;5F')
            projection = b'\n- [[Projects/example/records/tasks/pty-created|PTY created task]]\n'
            send(b'\x1b[200~' + projection + b'\x1b[201~')
            create_review('task')
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
            # Long-lived creation must check the projection under the core lock,
            # independently of the periodic external-change observer.
            for note_type, existing_id, projection_name, flag, values in [
                ('task', 'Projects/example/records/tasks/active.md', 'roadmap.md', '--roadmap',
                 ['pty-reviewed.md', 'open', '2026-10-03', '2026-10-03',
                  'Reviewed task', 'Привет 世界']),
                ('entity', 'Projects/example/entities/core.md', 'index.md', '--index',
                 ['pty-reviewed.md', 'pty-reviewed', 'subsystem', 'active', '2026-10-03',
                  'Reviewed entity', 'Привет 世界']),
            ]:
                create_inputs(note_type, values)
                create_review(note_type)
                maintained = root / 'Projects/example' / projection_name
                external = maintained.read_bytes() + f'\nConcurrent {note_type} decision: 世界  \n'.encode()
                accepted = root.parent / f'accepted-{projection_name}'
                accepted.write_bytes(external)
                existing = root / existing_id
                subprocess.run([
                    str(binary), '--root', str(root), '--project', 'example',
                    'update-record' if note_type == 'task' else 'update-entity', existing_id,
                    '--expected', str(existing), '--replacement', str(existing), flag, str(accepted),
                ], check=True, stdout=subprocess.DEVNULL)
                before_refusal = {str(path.relative_to(root)): path.read_bytes()
                                  for path in root.rglob('*') if path.is_file()}
                command('save')
                wait_for(b'Operation failed')
                wait_for(b'REVIEW')
                assert maintained.read_bytes() == external, 'refusal must retain concurrent projection'
                assert {str(path.relative_to(root)): path.read_bytes()
                        for path in root.rglob('*') if path.is_file()} == before_refusal
                command('discard')
                assert {str(path.relative_to(root)): path.read_bytes()
                        for path in root.rglob('*') if path.is_file()} == before_refusal
                # Fresh review uses the concurrent projection, then creates and reopens.
                create_inputs(note_type, values)
                create_review(note_type)
                command('save')
                wait_for(b'READING')
                folder = 'records/tasks' if note_type == 'task' else 'entities'
                reviewed = root / 'Projects/example' / folder / 'pty-reviewed.md'
                assert 'Привет 世界'.encode() in reviewed.read_bytes()
                assert maintained.read_bytes() == external, 'fresh creation must keep reviewed projection'
            command('open Projects/example/records/tasks/pty-created.md')
            wait_for(b'READING')
            task_before_discard = created.read_bytes()
            roadmap_before_discard = roadmap.read_bytes()
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
            lifecycle_review(b'ROADMAP')
            send(b'\x13')  # Ctrl-S applies the reviewed pair through the core.
            wait_for(b'READING')
            assert created.read_bytes() == task_before_discard + task_addition
            assert roadmap.read_bytes() == roadmap_before_discard + roadmap_addition
            # Entities pair with the configured index; problems pair with the roadmap.
            # Inspect both rendered document labels and exact persisted bytes.
            for identity, title, projection_path, label in [
                ('Projects/example/entities/core.md', b'ENTITY LIFECYCLE',
                 root / 'Projects/example/index.md', b'INDEX'),
                ('Projects/example/records/problems/open.md', b'PROBLEM LIFECYCLE',
                 roadmap, b'ROADMAP'),
            ]:
                command(f'open {identity}')
                wait_for(b'READING')
                source_path = root / identity
                note_before = source_path.read_bytes()
                projection_before = projection_path.read_bytes()
                command('lifecycle')
                wait_for(title)
                wait_for(b'NOTE SOURCE')
                send(b'\x1b[1;5F')
                send(b'\x1b[200~\nDiscard paired note draft.\n\x1b[201~')
                send(b'\x0e')
                wait_for(title + ' · '.encode() + label)
                send(b'\x1b[1;5F')
                send(b'\x1b[200~\nDiscard paired projection draft.\n\x1b[201~')
                command('discard')
                wait_for(b'READING')
                assert source_path.read_bytes() == note_before
                assert projection_path.read_bytes() == projection_before
                command('lifecycle')
                wait_for(title)
                send(b'\x1b[1;5F')
                note_addition = '\nPTY paired note: Привет 世界  \n'.encode()
                send(b'\x1b[200~' + note_addition + b'\x1b[201~')
                send(b'\x0e')
                wait_for(title + ' · '.encode() + label)
                send(b'\x1b[1;5F')
                projection_addition = b'\nPTY paired projection update.\n'
                send(b'\x1b[200~' + projection_addition + b'\x1b[201~')
                send(b'\x10')  # Ctrl-P returns to the retained note draft.
                wait_for(b'NOTE SOURCE')
                send(b'\x0e')  # Return to the projection editor before review.
                lifecycle_review(label)
                send(b'\x13')
                wait_for(b'READING')
                assert source_path.read_bytes() == note_before + note_addition
                assert projection_path.read_bytes() == projection_before + projection_addition
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
            repository_link_review()
            # Resizing must not lose state or crash. Restore usable dimensions afterward.
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 5, 20, 0, 0))
            drain(0.2)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
            drain(0.2)
            assert process.poll() is None
            command('open Projects/example/records/tasks/pty-created.md')
            wait_for(b'Projects/example/records/tasks/pty-created.md')
        elif keyboard:
            # Include the five-second automatic check: unchanged data must stay quiet
            # even when the terminal never supplies focus-reporting events.
            # The prior profile can leave an open-note navigation state. Its
            # source loads after "Library loaded", so wait for the worker to
            # finish before measuring background output.
            wait_for_ready()
            length = len(output)
            start_screen = screen.text()
            drain(5.3)
            assert len(output) == length, (
                f'unchanged background checks must not repaint: '
                f'{len(output) - length} bytes after startup; '
                f'initial screen={start_screen!r}; current screen={screen.text()!r}; '
                f'new bytes={bytes(output[length:])[:240]!r}'
            )
            command('open Projects/example/entities/core.md')
            wait_for(b'READING')
            command('edit')
            wait_for(b'SOURCE')
            note = root / 'Projects/example/entities/core.md'
            original = note.read_bytes()
            send(b'\x1b[1;5F')
            paste = f'\nKeyboard {term}: Привет 世界 e\u0301\n'.encode()
            # Keep the opening marker together for the supported baseline; the
            # opt-in probe exercises the parser's bounded Escape ambiguity window.
            if split_paste_start:
                send(b'\x1b')
                send(b'[200~')
            else:
                send(b'\x1b[200~')
            # Deliberately split UTF-8 and the closing marker across writes.
            for byte in paste + b'\x1b[201~':
                os.write(master, bytes([byte]))
                drain(0.002)
            drain()
            for rows, columns in [(5, 20), (12, 40), size]:
                fcntl.ioctl(slave, termios.TIOCSWINSZ,
                            struct.pack('HHHH', rows, columns, 0, 0))
                drain(0.2)
            send(b'\x11')  # Ctrl-Q must retain the draft after every resize.
            wait_for(b'Unsaved changes')
            assert process.poll() is None
            assert note.read_bytes() == original
            send(b'\x13')
            wait_for(b'Saved through the core')
            assert note.read_bytes() == original + paste, 'fragmented paste/resize must preserve exact bytes'
            send(b'\x1b[200~Discard this draft\x1b[201~')
            command('discard', from_editor=True)
            wait_for(b'READING')
            assert note.read_bytes() == original + paste, 'keyboard discard must not write'
            # Both entity/index buffers remain reachable even at 40x12 with no mouse.
            index = root / 'Projects/example/index.md'
            index_before = index.read_bytes()
            command('lifecycle')
            wait_for(b'ENTITY LIFECYCLE')
            send(b'\x1b[1;5F')
            send(b'\x1b[200~\nCompact entity draft.\n\x1b[201~')
            send(b'\x0e')
            wait_for('ENTITY LIFECYCLE · INDEX'.encode())
            send(b'\x1b[1;5F')
            send(b'\x1b[200~\nCompact index draft.\n\x1b[201~')
            send(b'\x10')
            wait_for(b'NOTE SOURCE')
            command('discard', from_editor=True)
            wait_for(b'READING')
            assert note.read_bytes() == original + paste
            assert index.read_bytes() == index_before
            command(f'search Keyboard {term}')
            wait_for(b'matches')
            command('open Projects/example/entities/core.md')
            wait_for(b'READING')
            # A lone Escape still leaves a clean reader within a bounded delay.
            escape_started = time.monotonic()
            send(b'\x1b')
            wait_for(f'SEARCH · Keyboard {term}'.encode(), seconds=0.8)
            assert time.monotonic() - escape_started < 1.0, 'Escape must not wait indefinitely'
            # Ctrl-C clears a pending command without exiting or executing it.
            send(b'\x7funexecuted command\x03')
            assert process.poll() is None
            assert 'unexecuted command' not in screen.text()
            assert b'\x1b[38;' not in output and b'\x1b[48;' not in output
        else:
            length = len(output)
            drain(0.4)
            assert len(output) == length, 'reduced motion must not repaint an idle screen'
            assert b'\x1b[38;' not in output and b'\x1b[48;' not in output
        if full or keyboard:
            # Both lifecycle reviews, revision and no-write discard remain reachable at 40x12.
            for identity, label in [
                ('Projects/example/records/tasks/active.md', b'ROADMAP'),
                ('Projects/example/entities/core.md', b'INDEX'),
            ]:
                command(f'open {identity}')
                wait_for(b'READING')
                command('lifecycle')
                wait_for(b'NOTE SOURCE')
                note_path = root / identity
                projection_path = root / 'Projects/example' / ('index.md' if label == b'INDEX' else 'roadmap.md')
                note_before = note_path.read_bytes()
                projection_before = projection_path.read_bytes()
                before_review = {p: p.read_bytes() for p in root.rglob('*') if p.is_file()}
                note_addition = f'\nReviewed lifecycle {term}: Привет 世界  \n'.encode()
                projection_addition = b'\nReviewed lifecycle projection.\n'
                send(b'\x1b[1;5F')
                send(b'\x1b[200~' + note_addition + b'\x1b[201~')
                send(b'\x13')
                assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                send(b'\x0e')
                wait_for(b' LIFECYCLE')
                send(b'\x1b[1;5F')
                send(b'\x1b[200~' + projection_addition + b'\x1b[201~')
                send(b'\x13')
                assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                lifecycle_review(label)
                send(b'\x11')
                assert process.poll() is None, 'review must guard exit'
                send(b'\x10')
                wait_for(b'REVIEW EXACT NOTE')
                send(b'\x10')  # Return to editing; invalidate both reviews.
                wait_for(label)
                projection_revision = b'Revised projection.\n'
                send(b'\x1b[1;5F')
                send(b'\x1b[200~' + projection_revision + b'\x1b[201~')
                send(b'\x13')
                assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                send(b'\x10')
                wait_for(b'NOTE SOURCE')
                note_revision = b'Revised note.\n'
                send(b'\x1b[1;5F')
                send(b'\x1b[200~' + note_revision + b'\x1b[201~')
                send(b'\x0e')
                lifecycle_review(label)
                assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                if label == b'ROADMAP':
                    command('discard')
                    wait_for(b'READING')
                    assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                else:
                    # A genuine checked index-only write after review must refuse the stale pair.
                    expected = root.parent / 'lifecycle-expected.md'
                    accepted = root.parent / 'lifecycle-index.md'
                    expected.write_bytes(note_before)
                    external_index = projection_before + b'\nConcurrent index after lifecycle review.\n'
                    accepted.write_bytes(external_index)
                    subprocess.run([str(binary), '--root', str(root), '--project', 'example',
                                    'update-entity', identity, '--expected', str(expected),
                                    '--replacement', str(expected), '--index', str(accepted)],
                                   check=True, stdout=subprocess.DEVNULL)
                    external = {p: p.read_bytes() for p in root.rglob('*') if p.is_file()}
                    send(b'\x13')
                    # A queued background check can replace the one-row error toast.
                    # Require completed foreground work, retained exact review and bytes below.
                    wait_for_refusal()
                    wait_for(b'REVIEW EXACT INDEX')
                    assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == external
                    command('discard')
                    wait_for(b'READING')
                    assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == external
                    command('lifecycle')
                    wait_for(b'NOTE SOURCE')
                    send(b'\x1b[1;5F')
                    send(b'\x1b[200~' + note_addition + note_revision + b'\x1b[201~')
                    send(b'\x0e')
                    send(b'\x1b[1;5F')
                    send(b'\x1b[200~' + projection_addition + projection_revision + b'\x1b[201~')
                    lifecycle_review(label)
                    send(b'\x13')
                    wait_for(b'READING')
                    assert note_path.read_bytes() == note_before + note_addition + note_revision
                    assert projection_path.read_bytes() == external_index + projection_addition + projection_revision
            # Guided fields and exact review remain reachable in the compact keyboard profile.
            for note_type in ['session', 'handoff']:
                command('handoff' if note_type == 'handoff' else 'event session')
                wait_for(f'EVENT {note_type}'.encode())
                name = f'pty-guided-{term}-{"keyboard" if keyboard else "full"}-{note_type}.md'
                destination = root / f'Projects/example/events/{"sessions" if note_type == "session" else "handoffs"}/{name}'
                before_review = {p: p.read_bytes() for p in root.rglob('*') if p.is_file()}
                send(name.encode() + b'\r')
                wait_for(b'date')
                body = f'Guided {term}: Привет 世界  \n\nSee [[Projects/example/entities/core|core]].\n{{{{title}}}}'
                for value in ['2026-10-03', f'Guided {term}', body]:
                    send(b'\x1b[200~' + value.encode() + b'\x1b[201~')
                    send(b'\x0e')
                wait_for(b'REVIEW')
                assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                send(b'\x11')
                assert process.poll() is None, 'unfinished event form must guard exit'
                send(b'\x10')
                wait_for(b'body')
                send(b'\x1b[1;5F')
                send(b'\x1b[200~\nRevised before publication.\x1b[201~')
                send(b'\x0e')
                wait_for(b'REVIEW')
                if note_type == 'session':
                    command('discard')
                    assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                    assert not destination.exists(), 'discarded event must remain absent'
                else:
                    send(b'\x13')
                    wait_for(b'READ ONLY')
                    expected = ('---\nschema_version: 1\nproject: example\ntype: handoff\n'
                                f'date: 2026-10-03\n---\n\n# Guided {term}\n\n{body}\nRevised before publication.\n')
                    assert destination.read_bytes() == expected.encode(), 'exact reviewed immutable source must be published'
                    assert destination.read_bytes().count(b'{{title}}') == 1, 'substitution must remain nonrecursive'
            # Mutable multiline fields, revision, paired exact review and no-write discard
            # use the same actual keyboard path in full and compact 40x12 scenarios.
            for note_type in ['task', 'entity']:
                name = f'pty-multiline-{term}-{"keyboard" if keyboard else "full"}-{note_type}.md'
                folder = 'entities' if note_type == 'entity' else 'records/tasks'
                destination = root / f'Projects/example/{folder}/{name}'
                projection_path = root / f'Projects/example/{"index.md" if note_type == "entity" else "roadmap.md"}'
                body = f'  Mutable {term}: Привет 世界  \n\nLiteral {{{{title}}}}'
                values = ([name, name[:-3], 'subsystem', 'active', '2026-10-03', f'Mutable {term}', body]
                          if note_type == 'entity' else
                          [name, 'open', '2026-10-03', '2026-10-03', f'Mutable {term}', body])
                create_inputs(note_type, values)
                before_review = {p: p.read_bytes() for p in root.rglob('*') if p.is_file()}
                send(b'\x13')
                assert not destination.exists(), 'projection editor must not publish'
                create_review(note_type)
                send(b'\x11')
                assert process.poll() is None, 'paired review must guard exit'
                send(b'\x10')  # exact note
                wait_for(b'REVIEW EXACT NOTE')
                send(b'\x10')  # editable projection; invalidates review
                wait_for(b'EDIT INDEX' if note_type == 'entity' else b'EDIT ROADMAP')
                send(b'\x10')  # body field
                wait_for(b'body')
                send(b'\x1b[1;5F')
                send(b'\x1b[200~\nRevised mutable field.\x1b[201~')
                send(b'\x0e')  # projection editor
                create_review(note_type)
                assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                if note_type == 'task':
                    command('discard')
                    assert {p: p.read_bytes() for p in root.rglob('*') if p.is_file()} == before_review
                    assert not destination.exists()
                else:
                    send(b'\x13')
                    wait_for(b'READING')
                    expected = ('---\nschema_version: 1\n'
                                f'entity: {name[:-3]}\nkind: subsystem\nstatus: active\nreviewed: 2026-10-03\n'
                                f'---\n\n# Mutable {term}\n\n{body}\nRevised mutable field.\n')
                    assert destination.read_bytes() == expected.encode(), 'published note must match exact multiline review'
                    assert projection_path.read_bytes() == before_review[projection_path]
            # Recovery inspection stays read-only even while a source draft is active.
            command('open Projects/example/entities/core.md')
            wait_for(b'READING')
            command('edit')
            wait_for(b'SOURCE')
            send(b'\x1b[1;5F')
            retained_draft = f'\nRetained recovery draft {term}: Привет 世界  \n'.encode()
            send(b'\x1b[200~' + retained_draft + b'\x1b[201~')
            before_inspection = snapshot()
            command('recovery', from_editor=True)
            wait_for(b'RECOVERY INSPECTION')
            send(b'\x13')
            assert snapshot() == before_inspection
            command('refresh')
            assert snapshot() == before_inspection and process.poll() is None
            command('back')
            wait_for(b'SOURCE')
            command('discard', from_editor=True)
            assert snapshot() == before_inspection, 'discard must preserve canonical files'

            # A genuine refused save retains the editor and closes stale library rows.
            command('edit')
            wait_for(b'SOURCE')
            send(b'\x1b[1;5F')
            send(b'\x1b[200~' + retained_draft + b'\x1b[201~')
            note, journal, recovery_source_before, recovery_source_after = stage_recovery()
            interrupted = snapshot()
            command('save', from_editor=True)
            wait_for(b'RECOVERY INSPECTION')
            assert snapshot() == interrupted
            command('refresh')
            command('quit')
            assert snapshot() == interrupted and process.poll() is None
            command('back')
            wait_for(b'SOURCE')
            # The original draft and undo stack remain usable after leaving inspection.
            # Save was submitted from the prompt; returning restores that exact focus.
            send(b'\x1b[Z')  # Prompt -> retained source editor.
            send(b'\x1b[1;5F')
            if size[1] <= 40:
                # The one-row editor viewport shows the trailing blank line at Ctrl-End.
                send(b'\x1b[A\x1b[H')  # Previous logical line, then its beginning.
            wait_for(b'Retained recovery draft')
            send(b'\x1b[200~\nAfter inspection.\x1b[201~')
            send(b'\x1a')
            command('discard', from_editor=True)
            wait_for(b'RECOVERY INSPECTION')
            assert snapshot() == interrupted
            note.write_bytes(recovery_source_after)
            command('refresh')
            wait_for(b'Core recovery completed: RolledBack')
            wait_for_ready()
            assert note.read_bytes() == recovery_source_before and not journal.exists()
            subprocess.run([str(binary), '--root', str(root), '--project', 'example',
                            'validate'], check=True, stdout=subprocess.DEVNULL)

            repository_link_review()
            project_initialization_review()
            # Preserve the established cross-launch task navigation checkpoint.
            command('open Projects/example/records/tasks/pty-created.md')
            wait_for(b'READING')
        command('quit')
        process.wait(timeout=8)
        drain()
        assert process.returncode == 0
        assert b'\x1b[?1049l' in output and b'\x1b[?2004l' in output
        assert b'\x1b[?1004l' in output
        assert b'\x1b[?1006l' in output
        assert termios.tcgetattr(slave) == before, 'raw terminal attributes must be restored exactly'
        print(f'PASS TERM={term} {size[1]}x{size[0]}: startup, input, clean exit, terminal restoration' +
              ('; animation, Unicode paste, dirty guard, checked save, search, create/lifecycle forms, recovery inspection/refusal/retry, integration inspection/cancel/confirm/apply/remove, resize' if full else
               '; keyboard-only, fragmented Unicode paste, dirty resize, save/discard, search, quiet refresh, Ctrl-C' if keyboard else
               '; ASCII, no-color, reduced motion' + ('; initial recovery refusal/retry' if recovery_start else '')) +
              ('; repository-link and project-init review/cancel/conflict/confirm/resolve' if full or keyboard else ''))
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=BASE / 'target/debug/akasha')
    parser.add_argument('--profile', choices=['all', 'existing', 'keyboard'], default='all')
    parser.add_argument('--split-paste-start', action='store_true',
                        help='regress a split paste opening Escape (requires --profile keyboard)')
    args = parser.parse_args()
    if args.split_paste_start and args.profile != 'keyboard':
        parser.error('--split-paste-start requires --profile keyboard')
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
        for term in ([] if args.profile == 'keyboard' else ['xterm-256color', 'xterm', 'linux']):
            full = term == 'xterm-256color'
            check(args.binary.resolve(), root, agent_home, term, full=full,
                  expect_restore=expect_restore)
            expect_restore = True
        if args.profile != 'existing':
            for term, size, env_no_color in [
                ('xterm-256color', (24, 80), True),
                ('screen-256color', (12, 40), False),
                ('tmux-256color', (24, 80), False),
            ]:
                check(args.binary.resolve(), root, agent_home, term, keyboard=True,
                      size=size, env_no_color=env_no_color,
                      split_paste_start=args.split_paste_start)
        if args.profile == 'all' and not args.split_paste_start:
            for term, size in [('xterm-256color', (32, 110)), ('screen-256color', (12, 40)),
                               ('tmux-256color', (24, 80))]:
                check(args.binary.resolve(), root, agent_home, term, size=size,
                      recovery_start=True)
        state = temp / 'state/akasha/tui-navigation-v1.json'
        assert state.is_file(), 'clean exit must publish navigation state'
        assert state.stat().st_mode & 0o777 == 0o600, 'navigation state must be private'
        subprocess.run([str(args.binary.resolve()), '--root', str(root), '--project', 'example', 'validate'], check=True, stdout=subprocess.DEVNULL)

if __name__ == '__main__':
    main()
