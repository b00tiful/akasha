#!/usr/bin/env python3
"""Linux PTY acceptance for Akasha's terminal adapter; standard-library test tooling only."""
import argparse
import fcntl
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

BASE = Path(__file__).resolve().parents[1]


def check(binary, root, term, full=False):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 110, 0, 0))
    before = termios.tcgetattr(slave)
    def child_setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    env = dict(os.environ, TERM=term)
    env.pop('NO_COLOR', None)
    args = [str(binary), '--root', str(root), '--project', 'example', 'tui']
    if not full:
        args += ['--ascii', '--no-motion', '--no-color']
    process = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave,
                               env=env, preexec_fn=child_setup)
    output = bytearray()
    def drain(seconds=0.15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], min(0.03, max(0, deadline-time.monotonic())))[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                output.extend(data)
                # A PTY supplies transport, not a terminal emulator. Answer the standard
                # cursor-position query just as VTE/xterm do during Ratatui setup.
                if b'\x1b[6n' in data:
                    os.write(master, b'\x1b[1;1R')
        return bytes(output)
    def wait_for(needle, seconds=8):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            raw = drain(0.05)
            # Differential cell rendering may move the cursor instead of writing spaces.
            visible = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', raw)
            if b''.join(needle.split()) in b''.join(visible.split()):
                return
            if process.poll() is not None:
                break
        raise AssertionError(f'{term}: missing {needle!r}: {bytes(output)[-1500:]!r}')
    def send(data):
        os.write(master, data)
        drain()
    def command(value):
        send(b'\x1b')
        send(value.encode() + b'\r')
    try:
        wait_for(b'Library loaded')
        assert b'\x1b[?1049h' in output
        assert b'\x1b[?2004h' in output
        if full:
            idle_before = len(output)
            drain(0.7)
            assert len(output) > idle_before, 'ambient frame must change while idle'
            command('open Projects/example/entities/core.md')
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
            # Resizing must not lose state or crash. Restore usable dimensions afterward.
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 5, 20, 0, 0))
            drain(0.2)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
            drain(0.2)
            assert process.poll() is None
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
        assert termios.tcgetattr(slave) == before, 'raw terminal attributes must be restored exactly'
        print(f'PASS TERM={term}: startup, input, clean exit, terminal restoration' +
              ('; animation, Unicode paste, dirty guard, checked save, search, resize' if full else '; ASCII, no-color, reduced motion'))
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
    with tempfile.TemporaryDirectory(prefix='akasha-tui-pty-') as folder:
        temp = Path(folder)
        root = temp / 'root'
        shutil.copytree(BASE / 'tests/fixtures/resolution/valid-root', root)
        (temp / 'repository').mkdir()
        for term in ['xterm-256color', 'xterm', 'linux']:
            check(args.binary.resolve(), root, term, full=term == 'xterm-256color')
        subprocess.run([str(args.binary.resolve()), '--root', str(root), '--project', 'example', 'validate'], check=True, stdout=subprocess.DEVNULL)

if __name__ == '__main__':
    main()
