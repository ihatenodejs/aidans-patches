from __future__ import annotations

import subprocess
from unittest.mock import MagicMock

import pytest
from apk_lab.device import (
    AdbClient,
    AdbDevice,
    monitor_application,
    parse_adb_devices,
    parse_exit_info_records,
    parse_launcher_activity,
    parse_proc_stat,
    parse_proc_system_stat,
    select_device,
)
from apk_lab.models import ExitCode


def test_parse_adb_devices():
    output = """
* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached
emulator-5554	device
emulator-5556	offline
192.168.1.100:5555	unauthorized
"""
    devices = parse_adb_devices(output)
    assert len(devices) == 3
    assert devices[0] == AdbDevice("emulator-5554", "device")
    assert devices[1] == AdbDevice("emulator-5556", "offline")
    assert devices[2] == AdbDevice("192.168.1.100:5555", "unauthorized")


def test_select_device_single_online():
    devices = [
        AdbDevice("emulator-5554", "device"),
        AdbDevice("emulator-5556", "offline"),
    ]
    assert select_device(devices, None) == "emulator-5554"


def test_select_device_multiple_online_fails_without_serial():
    devices = [
        AdbDevice("emulator-5554", "device"),
        AdbDevice("emulator-5556", "device"),
    ]
    with pytest.raises(ValueError, match="Multiple online ADB devices"):
        select_device(devices, None)


def test_select_device_no_online_fails():
    devices = [AdbDevice("emulator-5556", "offline")]
    with pytest.raises(ValueError, match="No online ADB devices found"):
        select_device(devices, None)


def test_select_device_explicit_serial_valid():
    devices = [
        AdbDevice("emulator-5554", "device"),
        AdbDevice("emulator-5556", "offline"),
    ]
    assert select_device(devices, "emulator-5554") == "emulator-5554"


def test_select_device_explicit_serial_offline_rejected():
    devices = [
        AdbDevice("emulator-5554", "device"),
        AdbDevice("emulator-5556", "offline"),
    ]
    with pytest.raises(ValueError, match="is in state 'offline', must be 'device'"):
        select_device(devices, "emulator-5556")


def test_select_device_explicit_serial_not_found():
    devices = [AdbDevice("emulator-5554", "device")]
    with pytest.raises(ValueError, match="Device 'emulator-9999' not found"):
        select_device(devices, "emulator-9999")


def test_adb_client_run_and_stream(monkeypatch):
    mock_run = MagicMock(return_value=subprocess.CompletedProcess(["adb"], 0, "ok", ""))
    monkeypatch.setattr(subprocess, "run", mock_run)

    client = AdbClient(adb_path="/usr/bin/adb")
    res = client.run(["shell", "getprop"], serial="emulator-5554", timeout=5.0)
    assert res.stdout == "ok"
    mock_run.assert_called_once_with(
        ["/usr/bin/adb", "-s", "emulator-5554", "shell", "getprop"],
        capture_output=True,
        text=True,
        errors="replace",
        timeout=5.0,
        check=False,
    )


def test_parse_proc_stat_with_spaces_and_parentheses():
    # comm with spaces and parentheses: (My (Special) App)
    stat_line = (
        "1234 (My (Special) App) R 100 200 300 0 -1 4194304 10 20 0 0 "
        "150 250 5 10 20 0 4 0 50000 1000000 200 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0"
    )
    info = parse_proc_stat(stat_line)
    assert info.pid == 1234
    assert info.comm == "My (Special) App"
    assert info.state == "R"
    assert info.utime == 150
    assert info.stime == 250
    assert info.starttime == 50000


def test_parse_proc_system_stat():
    stat_text = """
cpu  1000 200 300 5000 100 50 20 0 0 0
cpu0 500 100 150 2500 50 25 10 0 0 0
cpu1 500 100 150 2500 50 25 10 0 0 0
intr 12345
"""
    info = parse_proc_system_stat(stat_text)
    assert info.total_jiffies == 1000 + 200 + 300 + 5000 + 100 + 50 + 20
    assert info.cpu_count == 2


def test_parse_exit_info_records():
    dumpsys = """
ApplicationExitInfo #0:
  timestamp=1710000005000
  pid=4321
  realUid=10100
  package=com.example.app
  process=com.example.app
  reason=6 (ANR)
  subreason=34 (BIND APPLICATION ANR)
  status=0
  description=Process failed to complete startup
ApplicationExitInfo #1:
  timestamp=1710000001000
  pid=4320
  realUid=10100
  package=com.example.app
  process=com.example.app
  reason=5 (CRASH_NATIVE)
  subreason=0
  status=139
  description=SIGSEGV
ApplicationExitInfo #2:
  timestamp=1709000000000
  pid=4310
  realUid=10100
  package=com.example.app
  process=com.example.app
  reason=4 (CRASH)
  status=1
  description=java.lang.NullPointerException
"""
    records = parse_exit_info_records(dumpsys)
    assert len(records) == 3
    assert records[0].reason_code == 6
    assert records[0].reason_name == "ANR"
    assert records[0].subreason_code == 34
    assert records[0].subreason_name == "BIND APPLICATION ANR"
    assert records[0].description == "Process failed to complete startup"

    assert records[1].reason_code == 5
    assert records[1].reason_name == "CRASH_NATIVE"

    assert records[2].reason_code == 4
    assert records[2].reason_name == "CRASH"


def test_parse_launcher_activity():
    badging_normal = """
package: name='com.example.app' versionCode='1' versionName='1.0'
launchable-activity: name='com.example.app.MainActivity'  label='App' icon='res/icon.png'
"""
    assert (
        parse_launcher_activity(badging_normal, "com.example.app")
        == "com.example.app/com.example.app.MainActivity"
    )

    badging_relative = """
launchable-activity: name='.ui.SplashActivity'  label='App' icon='res/icon.png'
"""
    assert (
        parse_launcher_activity(badging_relative, "com.example.app")
        == "com.example.app/com.example.app.ui.SplashActivity"
    )

    badging_none = "package: name='com.example.app'"
    with pytest.raises(ValueError, match="No launchable activity found"):
        parse_launcher_activity(badging_none, "com.example.app")

    badging_multiple = """
launchable-activity: name='com.example.app.Act1' label='A1'
launchable-activity: name='com.example.app.Act2' label='A2'
"""
    with pytest.raises(ValueError, match="Expected exactly one launchable activity"):
        parse_launcher_activity(badging_multiple, "com.example.app")


def test_monitor_healthy_completion():
    client = MagicMock()
    calls = 0

    def mock_run(args, **kwargs):
        nonlocal calls
        calls += 1
        if "exit-info" in args:
            return subprocess.CompletedProcess(
                [], 0, "ApplicationExitInfo #0:\n timestamp=1000\n", ""
            )
        if "date" in args:
            return subprocess.CompletedProcess([], 0, "1710000000\n", "")
        if "pidof" in args:
            return subprocess.CompletedProcess([], 0, "1234\n", "")
        if "/task/" in " ".join(args):
            return subprocess.CompletedProcess(
                [],
                0,
                f"1234 (app) S 1 1 1 0 0 0 0 0 0 0 {10 + calls} {20 + calls} 0 0 20 0 1 0 100 100 10 0 0 0 0 0 0 0 0 0 0 0 0 0",
                "",
            )
        if "/proc/stat" in " ".join(args):
            return subprocess.CompletedProcess(
                [], 0, f"cpu {1000 + calls * 100} 0 0 5000\ncpu0 500\n", ""
            )
        return subprocess.CompletedProcess([], 0, "", "")

    client.run.side_effect = mock_run
    mock_proc = MagicMock()
    mock_proc.stdout = None
    mock_proc.poll.return_value = 0
    cm = MagicMock()
    cm.__enter__.return_value = mock_proc
    cm.__exit__.return_value = False
    client.stream.return_value = cm

    code = monitor_application(
        client=client,
        package="com.example.app",
        serial="emulator-5554",
        timeout=0.01,
        poll_interval=0.002,
    )
    assert code == ExitCode.SUCCESS


def test_monitor_high_cpu_spin_detection(monkeypatch):
    client = MagicMock()
    # Mock sequence: baseline exit, date, pidof, then 6 consecutive intervals with R state and high CPU
    responses = [
        subprocess.CompletedProcess([], 0, "", ""),  # baseline exit-info
        subprocess.CompletedProcess([], 0, "1710000000\n", ""),  # date
    ]
    # For 7 samples: pidof + stat + sys_stat
    for i in range(8):
        responses.append(subprocess.CompletedProcess([], 0, "1234\n", ""))  # pidof
        # utime increases by 100 each time, state is R
        responses.append(
            subprocess.CompletedProcess(
                [],
                0,
                f"1234 (app) R 1 1 1 0 0 0 0 0 0 0 {100 + i * 100} 0 0 0 20 0 1 0 100 100 10 0 0 0 0 0 0 0 0 0 0 0 0 0",
                "",
            )
        )
        # total jiffies increases by 100 each time (100% of 1 CPU)
        responses.append(
            subprocess.CompletedProcess(
                [],
                0,
                f"cpu {1000 + i * 100} 0 0 0\ncpu0 100\n",
                "",
            )
        )

    client.run.side_effect = responses

    mock_proc = MagicMock()
    mock_proc.stdout = None
    mock_proc.poll.return_value = 0
    cm = MagicMock()
    cm.__enter__.return_value = mock_proc
    cm.__exit__.return_value = False
    client.stream.return_value = cm

    code = monitor_application(
        client=client,
        package="com.example.app",
        serial="emulator-5554",
        timeout=10.0,
        poll_interval=0.001,
    )
    assert code == ExitCode.RUNTIME_FAILURE


def test_monitor_process_disappears_with_crash_exit_info(monkeypatch):
    client = MagicMock()
    client.run.side_effect = [
        subprocess.CompletedProcess(
            [], 0, "ApplicationExitInfo #0:\n timestamp=1000\n", ""
        ),  # baseline
        subprocess.CompletedProcess([], 0, "1710000000\n", ""),  # date
        subprocess.CompletedProcess([], 0, "1234\n", ""),  # pidof attached
        subprocess.CompletedProcess(
            [],
            0,
            "1234 (app) S 1 1 1 0 0 0 0 0 0 0 10 20 0 0 20 0 1 0 100 100 10 0 0 0 0 0 0 0 0 0 0 0 0 0",
            "",
        ),
        subprocess.CompletedProcess([], 0, "cpu 1000 0 0 5000\ncpu0 500\n", ""),
        subprocess.CompletedProcess([], 0, "", ""),  # pidof empty -> disappeared!
        subprocess.CompletedProcess(  # exit-info query
            [],
            0,
            """
ApplicationExitInfo #0:
  timestamp=2000
  pid=1234
  package=com.example.app
  process=com.example.app
  reason=4 (CRASH)
  status=1
  description=NullPointerException
""",
            "",
        ),
    ]

    mock_proc = MagicMock()
    mock_proc.stdout = None
    mock_proc.poll.return_value = 0
    cm = MagicMock()
    cm.__enter__.return_value = mock_proc
    cm.__exit__.return_value = False
    client.stream.return_value = cm

    code = monitor_application(
        client=client,
        package="com.example.app",
        serial="emulator-5554",
        timeout=5.0,
        poll_interval=0.001,
    )
    assert code == ExitCode.RUNTIME_FAILURE


def test_monitor_startup_timeout():
    client = MagicMock()

    def mock_run(args, **kwargs):
        if "date" in args:
            return subprocess.CompletedProcess([], 0, "1710000000\n", "")
        return subprocess.CompletedProcess([], 0, "", "")

    client.run.side_effect = mock_run
    mock_proc = MagicMock()
    mock_proc.stdout = None
    mock_proc.poll.return_value = 0
    cm = MagicMock()
    cm.__enter__.return_value = mock_proc
    cm.__exit__.return_value = False
    client.stream.return_value = cm

    code = monitor_application(
        client=client,
        package="com.example.app",
        serial="emulator-5554",
        timeout=0.01,
        poll_interval=0.002,
    )
    assert code == ExitCode.RUNTIME_FAILURE


def test_monitor_ctrl_c():
    client = MagicMock()
    calls = 0

    def mock_run(args, **kwargs):
        nonlocal calls
        calls += 1
        if calls >= 3:
            raise KeyboardInterrupt()
        if "date" in args:
            return subprocess.CompletedProcess([], 0, "1710000000\n", "")
        return subprocess.CompletedProcess([], 0, "", "")

    client.run.side_effect = mock_run
    mock_proc = MagicMock()
    mock_proc.stdout = None
    mock_proc.poll.return_value = 0
    cm = MagicMock()
    cm.__enter__.return_value = mock_proc
    cm.__exit__.return_value = False
    client.stream.return_value = cm

    code = monitor_application(
        client=client,
        package="com.example.app",
        serial="emulator-5554",
        timeout=10.0,
        poll_interval=0.001,
    )
    assert code == ExitCode.SUCCESS
