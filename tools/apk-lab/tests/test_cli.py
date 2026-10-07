import argparse
import io
import shlex
import zipfile

import pytest
from apk_lab.archives import ArchiveSecurityError
from apk_lab.cli import (
    materialize_split_member,
    non_negative_float,
    validate_dex_entry_name,
)


def test_validate_dex_entry_name_valid():
    assert validate_dex_entry_name("classes.dex") == "classes.dex"
    assert validate_dex_entry_name("classes2.dex") == "classes2.dex"
    assert validate_dex_entry_name("classes10.dex") == "classes10.dex"


def test_validate_dex_entry_name_traversal_rejected():
    with pytest.raises(ArchiveSecurityError, match="Invalid or unsafe DEX"):
        validate_dex_entry_name("../classes.dex")
    with pytest.raises(ArchiveSecurityError, match="Invalid or unsafe DEX"):
        validate_dex_entry_name("foo/classes.dex")
    with pytest.raises(ArchiveSecurityError, match="Invalid or unsafe DEX"):
        validate_dex_entry_name("/classes.dex")
    with pytest.raises(ArchiveSecurityError, match="Invalid or unsafe DEX"):
        validate_dex_entry_name("notdex.txt")


def test_materialize_nested_split_creates_parents(tmp_path):
    zip_bytes = io.BytesIO()
    with zipfile.ZipFile(zip_bytes, "w") as zf:
        zf.writestr("splits/base-master.apk", b"fake_split_content")

    zip_bytes.seek(0)
    extracted_dir = tmp_path / "extracted"
    extracted_dir.mkdir()

    with zipfile.ZipFile(zip_bytes, "r") as zf:
        dest = materialize_split_member(zf, "splits/base-master.apk", extracted_dir)
        assert dest.is_file()
        assert dest.read_bytes() == b"fake_split_content"
        assert dest.parent == extracted_dir / "splits"


def test_materialize_split_traversal_rejected(tmp_path):
    zip_bytes = io.BytesIO()
    with zipfile.ZipFile(zip_bytes, "w") as zf:
        zf.writestr("../escaped.apk", b"evil")

    zip_bytes.seek(0)
    extracted_dir = tmp_path / "extracted"
    extracted_dir.mkdir()

    with (
        zipfile.ZipFile(zip_bytes, "r") as zf,
        pytest.raises(ArchiveSecurityError, match="escapes extraction directory"),
    ):
        materialize_split_member(zf, "../escaped.apk", extracted_dir)


def test_stale_negative_cli_parser_rejected():
    with pytest.raises(argparse.ArgumentTypeError, match="non-negative"):
        non_negative_float("-1")
    with pytest.raises(argparse.ArgumentTypeError, match="non-negative"):
        non_negative_float("-0.5")
    assert non_negative_float("0") == 0.0
    assert non_negative_float("2.5") == 2.5


def test_cleanup_command_formatting_custom_root_and_spaces(tmp_path):
    custom_root = tmp_path / "my custom workspace"
    run_dir = custom_root / "com.example.app" / "1-abc" / "digest123"

    cleanup_cmd = ["uv", "run", "--project", "tools/apk-lab", "apk-lab"]
    if str(custom_root) != ".apk-lab":
        cleanup_cmd.extend(["--workspace-root", str(custom_root)])
    cleanup_cmd.extend(["clean", "--run", str(run_dir)])

    cmd_str = shlex.join(cleanup_cmd)
    tokens = shlex.split(cmd_str)
    assert tokens[0:5] == ["uv", "run", "--project", "tools/apk-lab", "apk-lab"]
    assert "--workspace-root" in tokens
    assert str(custom_root) in tokens
    assert tokens.index("--workspace-root") < tokens.index("clean")
    assert tokens[tokens.index("clean") + 1 :] == ["--run", str(run_dir)]
