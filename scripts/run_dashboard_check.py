"""Run a release example in a real PTY and type exit after its terminal outcome.

Usage: python3 scripts/run_dashboard_check.py PROJECT COMMAND [ARG ...]
This is a CI/release check. Users run the dashboard inside screen or tmux.
"""
import errno
import fcntl
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import termios
import time


def main() -> int:
    project = Path(sys.argv[1]).resolve()
    previous = set((project / "output").glob("execution-*"))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 160, 0, 0))
    before = termios.tcgetattr(slave)
    child = subprocess.Popen(sys.argv[2:], stdin=slave, stdout=slave, stderr=slave,
                             env=dict(os.environ, TERM="xterm-256color"))
    terminal_output = bytearray()
    log_path = None
    shown = 0
    sent_exit = False
    try:
        deadline = time.monotonic() + 300
        while child.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.05)[0]:
                try:
                    terminal_output.extend(os.read(master, 65536))
                    del terminal_output[:-65536]
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
            if log_path is None:
                for execution in set((project / "output").glob("execution-*")) - previous:
                    if (execution / "log.txt").is_file():
                        log_path = execution / "log.txt"
                        break
            if log_path is not None:
                log = log_path.read_text()
                print(log[shown:], end="", flush=True)
                shown = len(log)
                if not sent_exit and any(message in log for message in (
                    "workflow: completed", "workflow: failed", "workflow: cancelled"
                )):
                    os.write(master, b"exit\r")
                    sent_exit = True
        if child.poll() is None:
            raise RuntimeError("dashboard example exceeded the 300-second check deadline")
        if child.returncode:
            print(terminal_output.decode(errors="replace"), file=sys.stderr)
        assert termios.tcgetattr(slave) == before, "terminal state was not restored"
        assert sent_exit or child.returncode, "example exited without a recorded terminal outcome"
        return child.returncode
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)


if __name__ == "__main__":
    sys.exit(main())
