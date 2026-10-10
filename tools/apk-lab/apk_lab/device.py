from __future__ import annotations

import argparse
import collections
import os
import re
import shutil
import subprocess
import sys
import time
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from apk_lab.models import ExitCode


@dataclass(frozen=True)
class AdbDevice:
    serial: str
    state: str


@dataclass(frozen=True)
class ProcStatInfo:
    pid: int
    comm: str
    state: str
    utime: int
    stime: int
    starttime: int


@dataclass(frozen=True)
class SystemStatInfo:
    total_jiffies: int
    cpu_count: int


@dataclass(frozen=True)
class ExitInfoRecord:
    index: int
    timestamp: int
    pid: int
    process: str
    package: str | None
    reason_code: int
    reason_name: str
    subreason_code: int | None
    subreason_name: str | None
    status: int
    description: str | None
    raw_text: str


def find_adb() -> Path:
    """Locates the adb binary from the Android SDK or PATH."""
    sdk_roots = [
        os.environ.get("ANDROID_HOME"),
        os.environ.get("ANDROID_SDK_ROOT"),
    ]
    for root in sdk_roots:
        if root:
            candidate = Path(root) / "platform-tools" / "adb"
            if candidate.is_file() and os.access(candidate, os.X_OK):
                return candidate.resolve()

    which_adb = shutil.which("adb")
    if which_adb:
        return Path(which_adb).resolve()

    raise FileNotFoundError(
        "adb binary not found. Set ANDROID_HOME / ANDROID_SDK_ROOT or ensure adb is on PATH."
    )


class AdbClient:
    """Thin, injectable wrapper around the adb CLI executable."""

    def __init__(self, adb_path: Path | str | None = None) -> None:
        self.adb_path = Path(adb_path).resolve() if adb_path else find_adb()

    def run(
        self,
        args: list[str],
        serial: str | None = None,
        timeout: float | None = None,
    ) -> subprocess.CompletedProcess[str]:
        cmd = [str(self.adb_path)]
        if serial:
            cmd.extend(["-s", serial])
        cmd.extend(args)
        return subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            errors="replace",
            timeout=timeout,
            check=False,
        )

    @contextmanager
    def stream(
        self,
        args: list[str],
        serial: str | None = None,
    ) -> Iterator[subprocess.Popen[str]]:
        cmd = [str(self.adb_path)]
        if serial:
            cmd.extend(["-s", serial])
        cmd.extend(args)
        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            errors="replace",
            bufsize=1,
        )
        try:
            yield proc
        finally:
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=2.0)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()


def parse_adb_devices(output: str) -> list[AdbDevice]:
    """Parses adb devices command output into AdbDevice records."""
    devices: list[AdbDevice] = []
    lines = output.splitlines()
    for line in lines:
        line = line.strip()
        if not line or line.startswith(("*", "List of devices")):
            continue
        parts = line.split()
        if len(parts) >= 2:
            devices.append(AdbDevice(serial=parts[0], state=parts[1]))
    return devices


def format_devices(devices: list[AdbDevice]) -> str:
    """Formats connected devices for display in error messages."""
    if not devices:
        return "none"
    return ", ".join(f"{d.serial} ({d.state})" for d in devices)


def select_device(devices: list[AdbDevice], explicit_serial: str | None) -> str:
    """Selects an online device adhering strictly to selection constraints."""
    if explicit_serial:
        match = next((d for d in devices if d.serial == explicit_serial), None)
        if match is None:
            raise ValueError(
                f"Device '{explicit_serial}' not found. Connected devices: {format_devices(devices)}"
            )
        if match.state != "device":
            raise ValueError(
                f"Device '{explicit_serial}' is in state '{match.state}', must be 'device'. Connected devices: {format_devices(devices)}"
            )
        return explicit_serial

    online = [d for d in devices if d.state == "device"]
    if len(online) == 1:
        return online[0].serial
    if not online:
        raise ValueError(
            f"No online ADB devices found. Connected devices: {format_devices(devices)}"
        )
    raise ValueError(
        f"Multiple online ADB devices found ({[d.serial for d in online]}). Specify target with --device <serial>. Connected devices: {format_devices(devices)}"
    )


def parse_launcher_activity(badging_text: str, package_name: str) -> str:
    """Parses and normalizes the single launchable activity from aapt2 badging."""
    matches = re.findall(r"launchable-activity:\s+name='([^']+)'", badging_text)
    if not matches:
        raise ValueError("No launchable activity found in APK badging output")
    if len(matches) > 1:
        raise ValueError(
            f"Expected exactly one launchable activity, found {len(matches)}: {matches}"
        )

    activity = matches[0]
    if activity.startswith("."):
        activity = f"{package_name}{activity}"
    return f"{package_name}/{activity}"


def install_apk(
    client: AdbClient,
    apk_path: Path | str,
    package_name: str,
    serial: str | None = None,
    clean_install: bool = False,
) -> None:
    """Installs an APK using adb install -r or clean install (uninstall + install)."""
    apk_str = str(Path(apk_path).resolve())
    if clean_install:
        client.run(["uninstall", package_name], serial=serial)
        proc = client.run(["install", apk_str], serial=serial)
    else:
        proc = client.run(["install", "-r", apk_str], serial=serial)

    combined = f"{proc.stdout}\n{proc.stderr}".strip()
    if proc.returncode != 0 or "Failure" in combined or "Exception" in combined:
        mode_str = "clean install" if clean_install else "reinstall"
        raise RuntimeError(f"ADB {mode_str} failed: {combined}")


def launch_component_intent(
    client: AdbClient,
    launch_component: str,
    serial: str | None = None,
) -> None:
    """Launches an activity component using am start -n."""
    proc = client.run(["shell", "am", "start", "-n", launch_component], serial=serial)
    combined = f"{proc.stdout}\n{proc.stderr}".strip()
    if proc.returncode != 0 or "Error:" in combined:
        raise RuntimeError(f"ADB launch activity failed: {combined}")


def parse_pidof_output(output: str) -> list[int]:
    """Parses whitespace-separated numeric pidof output."""
    pids: list[int] = []
    for token in output.strip().split():
        try:
            pids.append(int(token))
        except ValueError:
            continue
    return pids


def parse_proc_stat(stat_text: str) -> ProcStatInfo:
    """Parses /proc/<pid>/stat or /proc/<pid>/task/<tid>/stat text handling spaces/parentheses in comm."""
    rindex = stat_text.rfind(")")
    if rindex == -1:
        raise ValueError(
            f"Malformed proc stat line (missing closing parenthesis): {stat_text}"
        )

    left = stat_text[:rindex]
    first_open = left.find("(")
    if first_open == -1:
        raise ValueError(
            f"Malformed proc stat line (missing opening parenthesis): {stat_text}"
        )

    pid_str = left[:first_open].strip()
    pid = int(pid_str)
    comm = left[first_open + 1 :]

    fields = stat_text[rindex + 1 :].strip().split()
    if len(fields) < 20:
        raise ValueError(
            f"Malformed proc stat line (too few fields after comm): {stat_text}"
        )

    state = fields[0]
    utime = int(fields[11])
    stime = int(fields[12])
    starttime = int(fields[19])

    return ProcStatInfo(
        pid=pid,
        comm=comm,
        state=state,
        utime=utime,
        stime=stime,
        starttime=starttime,
    )


def parse_proc_system_stat(stat_text: str) -> SystemStatInfo:
    """Parses /proc/stat lines to calculate total aggregate jiffies and online CPU count."""
    total_jiffies = 0
    cpu_count = 0
    for line in stat_text.splitlines():
        line = line.strip()
        if line.startswith("cpu "):
            parts = line.split()[1:]
            for p in parts:
                try:
                    total_jiffies += int(p)
                except ValueError:
                    continue
        elif re.match(r"^cpu\d+", line):
            cpu_count += 1

    return SystemStatInfo(
        total_jiffies=total_jiffies,
        cpu_count=max(cpu_count, 1),
    )


def parse_exit_info_records(dumpsys_output: str) -> list[ExitInfoRecord]:
    """Parses dumpsys activity exit-info output into structured ExitInfoRecord entries."""
    records: list[ExitInfoRecord] = []
    blocks = re.split(r"(?=ApplicationExitInfo\s+#\d+:)", dumpsys_output)

    for block in blocks:
        block = block.strip()
        if not block.startswith("ApplicationExitInfo"):
            continue

        header_match = re.search(r"ApplicationExitInfo\s+#(\d+):", block)
        index = int(header_match.group(1)) if header_match else len(records)

        timestamp = 0
        ts_match = re.search(r"timestamp=(\d+)", block)
        if ts_match:
            timestamp = int(ts_match.group(1))

        pid = 0
        pid_match = re.search(r"pid=(\d+)", block)
        if pid_match:
            pid = int(pid_match.group(1))

        process = ""
        proc_match = re.search(r"process=([^\s]+)", block)
        if proc_match:
            process = proc_match.group(1)

        pkg_match = re.search(r"package=([^\s]+)", block)
        pkg = pkg_match.group(1) if pkg_match else None

        reason_code = 0
        reason_name = "UNKNOWN"
        reason_match = re.search(r"reason=(\d+)(?:\s*\(([^)]+)\))?", block)
        if reason_match:
            reason_code = int(reason_match.group(1))
            if reason_match.group(2):
                reason_name = reason_match.group(2)

        subreason_code: int | None = None
        subreason_name: str | None = None
        sub_match = re.search(r"subreason=(\d+)(?:\s*\(([^)]+)\))?", block)
        if sub_match:
            subreason_code = int(sub_match.group(1))
            if sub_match.group(2):
                subreason_name = sub_match.group(2)

        status = 0
        st_match = re.search(r"status=(-?\d+)", block)
        if st_match:
            status = int(st_match.group(1))

        desc_match = re.search(r"description=([^\n]+)", block)
        desc = desc_match.group(1).strip() if desc_match else None

        records.append(
            ExitInfoRecord(
                index=index,
                timestamp=timestamp,
                pid=pid,
                process=process,
                package=pkg,
                reason_code=reason_code,
                reason_name=reason_name,
                subreason_code=subreason_code,
                subreason_name=subreason_name,
                status=status,
                description=desc,
                raw_text=block,
            )
        )

    return sorted(records, key=lambda r: r.timestamp, reverse=True)


def capture_thread_dump(
    client: AdbClient, pid: int, serial: str | None = None
) -> str | None:
    """Attempts best-effort thread dump capture via kill -3 and recent logcat."""
    client.run(["shell", "kill", "-3", str(pid)], serial=serial)
    time.sleep(0.5)
    proc = client.run(["logcat", "-d", "--pid", str(pid), "-t", "1000"], serial=serial)
    if proc.returncode == 0 and proc.stdout:
        lines = proc.stdout.splitlines()
        dump_lines = [
            ln for ln in lines if "tid=" in ln or "prio=" in ln or "at " in ln
        ]
        if dump_lines:
            return "\n".join(dump_lines[:100])
    return None


def monitor_application(
    client: AdbClient,
    package: str,
    serial: str | None,
    timeout: float | None = None,
    dump_threads: bool = False,
    poll_interval: float = 0.5,
) -> int:
    """Monitors an application process health, ANRs, crashes, and high CPU spin."""
    start_monotonic = time.monotonic()
    deadline = start_monotonic + timeout if timeout is not None else None

    # Capture initial exit-info baseline
    init_exit_res = client.run(
        ["shell", "dumpsys", "activity", "exit-info", package], serial=serial
    )
    baseline_records = parse_exit_info_records(init_exit_res.stdout)
    baseline_ts = max((r.timestamp for r in baseline_records), default=0)

    # Capture device epoch for logcat filtering
    epoch_res = client.run(["shell", "date", "+%s"], serial=serial)
    try:
        start_epoch_sec = int(epoch_res.stdout.strip())
    except ValueError:
        start_epoch_sec = int(time.time())

    logcat_args = [
        "logcat",
        "-v",
        "epoch",
        "-T",
        str(start_epoch_sec),
        "ActivityManager:E",
        "AndroidRuntime:E",
        "libc:F",
        "DEBUG:F",
        "crash_dump:F",
        "tombstoned:F",
        "*:S",
    ]

    log_queue: collections.deque[str] = collections.deque(maxlen=256)

    with client.stream(logcat_args, serial=serial) as logcat_proc:
        active_pid: int | None = None
        last_stat: ProcStatInfo | None = None
        last_sys_stat: SystemStatInfo | None = None
        spin_intervals = 0
        healthy_intervals = 0

        print(f"Monitoring process for package '{package}'...")

        try:
            while True:
                # 1. Drain logcat buffer non-blocking
                if logcat_proc.stdout:
                    while True:
                        import select

                        rlist, _, _ = select.select([logcat_proc.stdout], [], [], 0)
                        if not rlist:
                            break
                        line = logcat_proc.stdout.readline()
                        if not line:
                            break
                        log_queue.append(line.strip())

                # Check log lines for immediate live ANR or crash events
                for line in list(log_queue):
                    if package in line or (active_pid and str(active_pid) in line):
                        if "ANR in" in line or "BIND APPLICATION ANR" in line:
                            print(f"\nLive ANR detected: {line}", file=sys.stderr)
                            if dump_threads and active_pid:
                                dump = capture_thread_dump(client, active_pid, serial)
                                if dump:
                                    print(f"\nThread dump:\n{dump}", file=sys.stderr)
                            return ExitCode.RUNTIME_FAILURE
                        if (
                            "FATAL EXCEPTION" in line
                            or "SIGSEGV" in line
                            or "fatal signal" in line.lower()
                        ):
                            print(f"\nLive Crash detected: {line}", file=sys.stderr)
                            return ExitCode.RUNTIME_FAILURE

                # 2. Check for running PID
                pid_out = client.run(["shell", "pidof", package], serial=serial).stdout
                pids = parse_pidof_output(pid_out)

                if not pids:
                    if active_pid is not None:
                        # Process was running and just disappeared!
                        print(f"\nProcess {active_pid} terminated.", file=sys.stderr)
                        # Query dumpsys activity exit-info
                        exit_res = client.run(
                            ["shell", "dumpsys", "activity", "exit-info", package],
                            serial=serial,
                        )
                        records = parse_exit_info_records(exit_res.stdout)
                        new_records = [
                            r
                            for r in records
                            if r.timestamp > baseline_ts
                            and (r.package == package or r.pid == active_pid)
                        ]
                        if new_records:
                            rec = new_records[0]
                            desc = f": {rec.description}" if rec.description else ""
                            sub = (
                                f" (subreason {rec.subreason_code} {rec.subreason_name})"
                                if rec.subreason_name
                                else ""
                            )
                            if rec.reason_code in (4, 5, 6):
                                print(
                                    f"ApplicationExitInfo: reason={rec.reason_code} ({rec.reason_name}){sub}{desc}",
                                    file=sys.stderr,
                                )
                                return ExitCode.RUNTIME_FAILURE
                            print(
                                f"ApplicationExitInfo: reason={rec.reason_code} ({rec.reason_name}){sub}{desc}"
                            )
                            return ExitCode.SUCCESS

                        print(
                            "Process exited without ApplicationExitInfo record.",
                            file=sys.stderr,
                        )
                        return ExitCode.RUNTIME_FAILURE

                    # Process has not started yet
                    if deadline is not None and time.monotonic() >= deadline:
                        print(
                            f"Startup timeout: package '{package}' did not start within timeout.",
                            file=sys.stderr,
                        )
                        return ExitCode.RUNTIME_FAILURE

                    time.sleep(poll_interval)
                    continue

                # Discovered PID
                current_pid = pids[0]
                if active_pid is None or active_pid != current_pid:
                    active_pid = current_pid
                    last_stat = None
                    last_sys_stat = None
                    spin_intervals = 0
                    healthy_intervals = 0
                    print(f"Attached to process PID {active_pid}")

                # 3. Sample proc stats
                stat_out = client.run(
                    ["shell", "cat", f"/proc/{active_pid}/task/{active_pid}/stat"],
                    serial=serial,
                ).stdout
                sys_stat_out = client.run(
                    ["shell", "cat", "/proc/stat"], serial=serial
                ).stdout

                try:
                    proc_stat = parse_proc_stat(stat_out)
                    sys_stat = parse_proc_system_stat(sys_stat_out)
                except (ValueError, IndexError):
                    time.sleep(poll_interval)
                    continue

                if (
                    last_stat
                    and last_sys_stat
                    and last_stat.starttime == proc_stat.starttime
                ):
                    thread_delta = (proc_stat.utime + proc_stat.stime) - (
                        last_stat.utime + last_stat.stime
                    )
                    sys_delta = sys_stat.total_jiffies - last_sys_stat.total_jiffies

                    cpu_pct = 0.0
                    if sys_delta > 0:
                        cpu_pct = (
                            (thread_delta / sys_delta) * sys_stat.cpu_count * 100.0
                        )

                    # Check spin condition: state R and CPU >= 90%
                    if proc_stat.state == "R" and cpu_pct >= 90.0:
                        spin_intervals += 1
                        if spin_intervals >= 6:  # 6 * 0.5s = 3.0s
                            print(
                                f"\nHigh CPU Spin Warning: main thread TID {active_pid} sustained >=90% CPU ({cpu_pct:.1f}%) in state 'R' for 3.0s",
                                file=sys.stderr,
                            )
                            if dump_threads:
                                dump = capture_thread_dump(client, active_pid, serial)
                                if dump:
                                    print(f"\nThread dump:\n{dump}", file=sys.stderr)
                            return ExitCode.RUNTIME_FAILURE
                    else:
                        spin_intervals = 0
                        healthy_intervals += 1
                else:
                    healthy_intervals += 1

                last_stat = proc_stat
                last_sys_stat = sys_stat

                # Check deadline
                if deadline is not None and time.monotonic() >= deadline:
                    if healthy_intervals >= 1:
                        print(
                            f"Monitoring completed: process healthy after {timeout}s."
                        )
                        return ExitCode.SUCCESS
                    print(
                        f"Monitoring timeout: no complete healthy sample within {timeout}s.",
                        file=sys.stderr,
                    )
                    return ExitCode.RUNTIME_FAILURE

                time.sleep(poll_interval)

        except KeyboardInterrupt:
            print("\nInterrupted.")
            return ExitCode.SUCCESS


def run_monitor(args: argparse.Namespace) -> int:
    """CLI handler for monitor subcommand."""
    client = AdbClient()
    devices = parse_adb_devices(client.run(["devices"]).stdout)
    serial = select_device(devices, args.device)
    return monitor_application(
        client=client,
        package=args.package,
        serial=serial,
        timeout=args.timeout,
        dump_threads=args.dump_threads,
    )
