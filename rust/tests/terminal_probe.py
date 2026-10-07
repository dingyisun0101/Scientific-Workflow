"""Linux PTY qualification of the public run facade; no third-party packages."""
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time


def check(binary: str, disk_pause: bool) -> None:
    with tempfile.TemporaryDirectory(prefix="workflow-terminal-") as temporary:
        root = Path(temporary)
        configs = root / "wf_configs"
        configs.mkdir()
        study = {
            "workflow_schema": 1,
            "threads": 2,
            "compute": {"mode": "auto"},
            "disk": {"pause_at_percent": 0.00000001 if disk_pause else None},
            "phases": {"run": {"tasks": [
                {"program": "/bin/sleep", "args": ["0.06"]} for _ in range(24)
            ]}},
        }
        (configs / "study.json").write_text(json.dumps(study))
        (configs / "parameters.json").write_text("{}")
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 150, 0, 0))
        before = termios.tcgetattr(slave)
        environment = dict(os.environ, TERM="xterm-256color", WORKFLOW_TERMINAL_PROBE=str(root))
        environment.pop("WORKFLOW_EXPECT_CANCEL", None)
        if disk_pause:
            environment["WORKFLOW_EXPECT_CANCEL"] = "1"
        child = subprocess.Popen(
            [binary, "--exact", "dashboard_child", "--nocapture"],
            stdin=slave, stdout=slave, stderr=slave, env=environment,
        )
        output = bytearray()
        stage = 0
        echo_started = None
        log = ""
        try:
            deadline = time.monotonic() + 20
            while child.poll() is None and time.monotonic() < deadline:
                if select.select([master], [], [], 0.02)[0]:
                    try:
                        output.extend(os.read(master, 65536))
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                logs = list((root / "output").glob("execution-*/log.txt"))
                if logs:
                    log = logs[0].read_text()
                if disk_pause:
                    if stage == 0 and "Work will not resume automatically" in log:
                        os.write(master, b"resume\r")
                        stage = 1
                    elif stage == 1 and "still paused; disk usage" in log:
                        assert "workflow: resumed" not in log
                        assert not list((root / "output").glob("*/replicate-*/task-*"))
                        os.write(master, b"exit\r")
                        stage = 2
                else:
                    if stage == 0 and "workflow: started" in log:
                        os.write(master, b"pause\r")
                        stage = 1
                    elif stage == 1 and "pause requested" in log:
                        echo_started = time.monotonic()
                        os.write(master, b"echo_probe")
                        stage = 2
                    elif stage == 2 and b"echo_probe" in output:
                        latency = time.monotonic() - echo_started
                        assert latency < 0.1, f"input redraw took {latency:.3f}s"
                        print(f"PTY command echo observed after {latency:.3f}s")
                        os.write(master, b"\x7f" * len(b"echo_probe"))
                        os.write(master, b"\x1b[6~\x1b[5~")  # PageDown / PageUp
                        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 25, 100, 0, 0))
                        os.write(master, b"resume\r")
                        stage = 3
                    elif stage == 3 and "workflow: completed" in log:
                        assert "workflow: resumed" in log
                        os.write(master, b"exit\r")
                        stage = 4
            assert child.poll() is not None, "dashboard did not finish: " + log
            assert child.returncode == 0, output.decode(errors="replace")
            assert stage == (2 if disk_pause else 4), (stage, log)
            assert termios.tcgetattr(slave) == before, "terminal attributes were not restored"
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)
            os.close(slave)


def check_startup_failure(binary: str) -> None:
    if os.geteuid() == 0:
        return  # Root bypasses the output-directory permission failure.
    with tempfile.TemporaryDirectory(prefix="workflow-startup-terminal-") as temporary:
        root = Path(temporary)
        configs = root / "wf_configs"
        configs.mkdir()
        (configs / "study.json").write_text(json.dumps({
            "workflow_schema": 1, "threads": 1, "compute": {"mode": "auto"},
            "disk": {"pause_at_percent": None},
            "phases": {"run": {"tasks": [{"program": "/bin/true"}]}},
        }))
        (configs / "parameters.json").write_text("{}")
        output_directory = root / "output"
        output_directory.mkdir()
        output_directory.chmod(0o555)
        master, slave = pty.openpty()
        before = termios.tcgetattr(slave)
        environment = dict(os.environ, TERM="xterm-256color", WORKFLOW_TERMINAL_PROBE=str(root),
                           WORKFLOW_EXPECT_STARTUP_FAILURE="1")
        environment.pop("WORKFLOW_EXPECT_CANCEL", None)
        child = subprocess.Popen(
            [binary, "--exact", "dashboard_child", "--nocapture"],
            stdin=slave, stdout=slave, stderr=slave, env=environment,
        )
        captured = bytearray()
        try:
            deadline = time.monotonic() + 5
            while child.poll() is None and time.monotonic() < deadline:
                if select.select([master], [], [], 0.02)[0]:
                    captured.extend(os.read(master, 65536))
            assert child.poll() is not None, "startup failure waited for an exit command"
            assert child.returncode == 0, captured.decode(errors="replace")
            while select.select([master], [], [], 0)[0]:
                captured.extend(os.read(master, 65536))
            assert b"\x1b[?1049h" in captured, "dashboard never entered alternate screen"
            assert b"\x1b[?1049l" in captured, "dashboard did not leave alternate screen"
            assert termios.tcgetattr(slave) == before, "startup failure did not restore terminal"
            assert not list(output_directory.iterdir()), "startup failure produced task output"
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            output_directory.chmod(0o755)
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    check(sys.argv[1], False)
    check(sys.argv[1], True)
    check_startup_failure(sys.argv[1])
    print("PTY command, disk reminder, paging/resize, startup-abort, and restoration checks passed.")
