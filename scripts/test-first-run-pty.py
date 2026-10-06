#!/usr/bin/env python3
"""Disposable keyboard acceptance for first run and reviewed init recovery."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import runpy
import select
import struct
import subprocess
import tempfile
import termios
import time

BASE = Path(__file__).resolve().parents[1]
Screen = runpy.run_path(str(BASE / 'scripts/test-tui-pty.py'))['Screen']


def check(binary, rows, columns, term):
    with tempfile.TemporaryDirectory(prefix='akasha-first-run-') as folder:
        temp = Path(folder)
        root = temp / 'memory 世界'
        repo = temp / 'repository'
        repo.mkdir()
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, 0, 0))
        attributes = termios.tcgetattr(slave)
        env = dict(os.environ, TERM=term, XDG_STATE_HOME=str(temp / 'state'),
                   XDG_CONFIG_HOME=str(temp / 'config'))
        env.pop('AKASHA_ROOT', None)
        env.pop('NO_COLOR', None)
        def child_setup():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        process = subprocess.Popen([str(binary), '--root', str(root), 'tui', '--no-motion', '--ascii'],
                                   cwd=repo, env=env, stdin=slave, stdout=slave, stderr=slave,
                                   preexec_fn=child_setup)
        screen = Screen()
        output = bytearray()

        def drain(seconds=0.08):
            until = time.monotonic() + seconds
            while time.monotonic() < until:
                if select.select([master], [], [], min(0.03, max(0, until-time.monotonic())))[0]:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        break
                    output.extend(data)
                    screen.feed(data)
                    if b'\x1b[6n' in data:
                        os.write(master, b'\x1b[1;1R')

        def wait_for(needle=None):
            until = time.monotonic() + 10
            while time.monotonic() < until:
                drain()
                text = screen.text()
                lines = text.splitlines()
                if needle is None:
                    if len(lines) > 1 and 'AKASHA' in lines[1] and not lines[1].rstrip().endswith('working'):
                        return
                elif ''.join(needle.split()) in ''.join(text.split()):
                    return
                if process.poll() is not None:
                    break
            raise AssertionError(f'{term} {columns}x{rows}: missing {needle!r}\n{screen.text()}')

        def send(data):
            os.write(master, data)
            drain()

        def command(value):
            wait_for()
            send(b'\x7f')
            send(value.encode() + b'\r')

        def displayed_id():
            send(b'\x1b[H')
            collected = ''
            first_body = 5 if rows >= 13 else 4
            for _ in range(130):
                match = re.search(r'PlanID:(sha256:[0-9a-f]{64})', re.sub(r'\s+', '', screen.text()))
                if match:
                    return match.group(1)
                collected += screen.text().splitlines()[first_body].strip()
                match = re.search(r'Plan ID: ?(sha256:[0-9a-f]{64})', collected)
                if match:
                    return match.group(1)
                send(b'\x1b[B')
            raise AssertionError(f'Plan ID not reachable: {collected}')

        def scroll_until(needle):
            send(b'\x1b[H')
            for _ in range(160):
                if ''.join(needle.split()) in ''.join(screen.text().split()):
                    return
                send(b'\x1b[B')
            raise AssertionError(f'Review text not reachable: {needle!r}\n{screen.text()}')

        def files_snapshot():
            return {p.relative_to(temp): p.read_bytes() for p in temp.rglob('*')
                    if p.is_file() and not p.is_relative_to(temp / 'state')}

        def stage_init(project, committed):
            # Derive synthetic recovery images from an actual successful named init.
            # Core acceptance independently uses process exits at real publication stages.
            repository = temp / f'{project} repository 世界'
            repository.mkdir()
            registry = root / 'Meta/projects.yaml'
            registry_before = registry.read_bytes()
            output = subprocess.run([str(binary), '--root', str(root), '--json', 'init', project],
                                    cwd=repository, env=env, check=True, capture_output=True)
            result = json.loads(output.stdout)
            project_dir = Path(result['project_dir'])
            fingerprint = lambda source: 'sha256:' + hashlib.sha256(source).hexdigest()
            directories = sorted((p.relative_to(project_dir) for p in project_dir.rglob('*') if p.is_dir()),
                                 key=lambda p: (len(p.parts), str(p)))
            files = sorted(p for p in project_dir.rglob('*') if p.is_file())
            pointer = Path(result['pointer'])
            journal = registry.with_name('.projects.yaml.akasha-init-journal.json')
            source = json.dumps({
                'schema_version': 1, 'project': project, 'project_dir': str(project_dir),
                'repository_dir': str(repository), 'directories': [str(p) for p in directories],
                'files': [{'path': str(p.relative_to(project_dir)), 'after': fingerprint(p.read_bytes())} for p in files],
                'pointer_after': fingerprint(pointer.read_bytes()),
                'registry_before': fingerprint(registry_before), 'registry_after': fingerprint(registry.read_bytes()),
                'template_files': result['template_files'],
            }).encode()
            descriptor = os.open(journal, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, 'wb') as target:
                target.write(source)
            if not committed:
                registry.write_bytes(registry_before)
            return repository, project_dir, pointer, journal, registry_before

        try:
            wait_for()
            prepare = f'setup {root}'
            command(prepare)
            wait_for('ROOT SETUP REVIEW')
            plan_id = displayed_id()
            assert not root.exists()
            command('confirm wrong')
            wait_for()
            assert not root.exists()
            command('discard')
            wait_for()
            assert not root.exists()
            assert not list(temp.glob('.*.akasha-setup.lock')), 'cancel must not create a lock'
            command(prepare)
            wait_for('ROOT SETUP REVIEW')
            assert displayed_id() == plan_id
            # A competing destination invalidates the displayed review without touching its bytes.
            root.mkdir()
            (root / 'human.md').write_text('Preserve these exact bytes.\n')
            command(f'confirm {plan_id}')
            wait_for('ROOT SETUP RESULT')
            wait_for('Operation failed:')
            assert (root / 'human.md').read_text() == 'Preserve these exact bytes.\n'
            assert list(root.iterdir()) == [root / 'human.md']
            # The harness alone removes its deliberate conflict; product never deletes it.
            (root / 'human.md').unlink()
            root.rmdir()
            command(prepare)
            wait_for('ROOT SETUP REVIEW')
            plan_id = displayed_id()
            command(f'confirm {plan_id}')
            wait_for('ROOT SETUP RESULT')
            wait_for('Root configured.')
            assert (root / 'akasha.toml').is_file()
            command('init first')
            wait_for('PROJECT INITIALIZATION REVIEW')
            init_id = displayed_id()
            assert not (repo / '.akasha.toml').exists()
            command(f'confirm {init_id}')
            wait_for('PROJECT INITIALIZATION RESULT')
            wait_for('Project initialized.')
            before = {p: p.read_bytes() for p in root.rglob('*') if p.is_file()}
            command('onboard')
            wait_for('AGENT ONBOARDING HANDOFF')
            wait_for()
            assert before == {p: p.read_bytes() for p in root.rglob('*') if p.is_file()}
            assert not (temp / 'config').exists(), 'setup must not install user/client wiring'
            subprocess.run([str(binary), '--root', str(root), 'validate'], cwd=repo,
                           check=True, stdout=subprocess.DEVNULL)
            for committed in [False, True]:
                project = 'rolled-back' if not committed else 'finalized'
                recovery_repo, project_dir, pointer, journal, registry_before = stage_init(project, committed)
                before = files_snapshot()
                command('recover-init')
                wait_for('INITIALIZATION RECOVERY REVIEW')
                recovery_id = displayed_id()
                scroll_until(f'Outcome: {"finalized" if committed else "rolled-back"}')
                assert files_snapshot() == before, 'recovery review must write nothing'
                for guarded in ['confirm wrong', 'save', 'quit', 'refresh']:
                    command(guarded)
                    wait_for()
                    assert files_snapshot() == before
                    assert process.poll() is None
                command('discard')
                wait_for()
                assert files_snapshot() == before, 'recovery cancellation must write nothing'
                command('recover-init')
                wait_for('INITIALIZATION RECOVERY REVIEW')
                assert displayed_id() == recovery_id
                if not committed:
                    index = project_dir / 'index.md'
                    original = index.read_bytes()
                    index.write_bytes('external 世界\r\n  '.encode())
                    conflict = files_snapshot()
                    command(f'confirm {recovery_id}')
                    wait_for('INITIALIZATION RECOVERY RESULT')
                    scroll_until('Operation failed:')
                    assert files_snapshot() == conflict, 'stale recovery must preserve external bytes'
                    command('recover-init')
                    wait_for('INITIALIZATION RECOVERY REVIEW')
                    scroll_until('Recovery review refused:')
                    assert files_snapshot() == conflict
                    # Only this disposable fixture restores its exact known preimage.
                    index.write_bytes(original)
                    command('recover-init')
                    wait_for('INITIALIZATION RECOVERY REVIEW')
                    recovery_id = displayed_id()
                command(f'confirm {recovery_id}')
                wait_for('INITIALIZATION RECOVERY RESULT')
                scroll_until('Initialization recovery completed.')
                assert not journal.exists()
                if committed:
                    expected = before.copy()
                    del expected[journal.relative_to(temp)]
                    assert files_snapshot() == expected, 'finalization only removes the journal'
                else:
                    assert not project_dir.exists() and not pointer.exists()
                    assert (root / 'Meta/projects.yaml').read_bytes() == registry_before
                    # A new project needs a separate fresh initialization review.
                    command(f'init {project} {recovery_repo}')
                    wait_for('PROJECT INITIALIZATION REVIEW')
                    new_id = displayed_id()
                    command(f'confirm {new_id}')
                    wait_for('PROJECT INITIALIZATION RESULT')
                    wait_for('Project initialized.')
                subprocess.run([str(binary), '--root', str(root), 'validate'], cwd=recovery_repo,
                               env=env, check=True, stdout=subprocess.DEVNULL)
            command('quit')
            process.wait(timeout=5)
            drain()
            assert process.returncode == 0
            assert termios.tcgetattr(slave) == attributes, 'terminal attributes must restore exactly'
            assert b'\x1b[?1049l' in output
            print(f'{term} {columns}x{rows}: PASS; first run, init recovery displayed-ID cancel/guards/conflict/rollback/finalize, fresh init, validation, terminal restoration')
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
    for rows, columns, term in [(32, 110, 'xterm-256color'), (12, 40, 'screen-256color')]:
        check(args.binary.resolve(), rows, columns, term)


if __name__ == '__main__':
    main()
