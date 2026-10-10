from __future__ import annotations

import subprocess
import zipfile
from pathlib import Path
from unittest.mock import MagicMock

import pytest
from apk_lab.archives import MaterializedApk
from apk_lab.code_inspection import (
    CodeInspectionError,
    DexLocation,
    extract_java_class,
    extract_method_from_class,
    extract_smali_class,
    format_colored_diff,
    locate_class,
    normalize_fqcn,
    parse_java_methods,
    parse_smali_methods,
)
from apk_lab.tools import ToolManager


def test_normalize_fqcn():
    fqcn, desc = normalize_fqcn("com.example.app.MyClass")
    assert fqcn == "com.example.app.MyClass"
    assert desc == "Lcom/example/app/MyClass;"

    fqcn2, desc2 = normalize_fqcn("Lcom/example/app/MyClass;")
    assert fqcn2 == "com.example.app.MyClass"
    assert desc2 == "Lcom/example/app/MyClass;"


def test_locate_class_multidex(tmp_path, monkeypatch):
    apk1 = tmp_path / "base.apk"
    with zipfile.ZipFile(apk1, "w") as zf:
        zf.writestr("classes.dex", b"dex1")
        zf.writestr("classes2.dex", b"dex2")

    mat_apks = [
        MaterializedApk(
            path=apk1, container_member=None, split_name="base", is_base=True
        )
    ]

    # Mock dexdump
    def fake_dexdump(cmd, **kwargs):
        dex_path = cmd[-1]
        if "classes2.dex" in dex_path:
            out = "Class descriptor  : 'Lcom/example/Target;'\n"
        else:
            out = "Class descriptor  : 'Lcom/example/Other;'\n"
        return subprocess.CompletedProcess(cmd, 0, out, "")

    monkeypatch.setattr(subprocess, "run", fake_dexdump)
    monkeypatch.setattr(
        "apk_lab.code_inspection.find_build_tools_bin",
        lambda name: Path("/bin/dexdump"),
    )

    loc = locate_class(mat_apks, "com.example.Target", allow_missing=False)
    assert loc is not None
    assert loc.split == "base"
    assert loc.dex_name == "classes2.dex"
    assert loc.descriptor == "Lcom/example/Target;"


def test_locate_class_missing_fails_standalone(tmp_path, monkeypatch):
    apk1 = tmp_path / "base.apk"
    with zipfile.ZipFile(apk1, "w") as zf:
        zf.writestr("classes.dex", b"dex1")

    mat_apks = [
        MaterializedApk(
            path=apk1, container_member=None, split_name="base", is_base=True
        )
    ]
    monkeypatch.setattr(
        subprocess, "run", lambda *a, **kw: subprocess.CompletedProcess([], 0, "", "")
    )
    monkeypatch.setattr(
        "apk_lab.code_inspection.find_build_tools_bin",
        lambda name: Path("/bin/dexdump"),
    )

    with pytest.raises(CodeInspectionError, match="not found in artifact"):
        locate_class(mat_apks, "com.example.Target", allow_missing=False)

    # When allow_missing=True, returns None
    assert locate_class(mat_apks, "com.example.Target", allow_missing=True) is None


def test_locate_class_duplicate_rejected(tmp_path, monkeypatch):
    apk1 = tmp_path / "base.apk"
    with zipfile.ZipFile(apk1, "w") as zf:
        zf.writestr("classes.dex", b"dex1")
        zf.writestr("classes2.dex", b"dex2")

    mat_apks = [
        MaterializedApk(
            path=apk1, container_member=None, split_name="base", is_base=True
        )
    ]
    # Both DEX return the descriptor
    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *a, **kw: subprocess.CompletedProcess(
            [], 0, "Class descriptor  : 'Lcom/example/Target;'\n", ""
        ),
    )
    monkeypatch.setattr(
        "apk_lab.code_inspection.find_build_tools_bin",
        lambda name: Path("/bin/dexdump"),
    )

    with pytest.raises(CodeInspectionError, match="Duplicate class definitions found"):
        locate_class(mat_apks, "com.example.Target", allow_missing=False)


def test_extract_smali_class_arguments(tmp_path):
    apk1 = tmp_path / "app.apk"
    with zipfile.ZipFile(apk1, "w") as zf:
        zf.writestr("classes.dex", b"dex_bytes")

    location = DexLocation(
        split="base",
        dex_name="classes.dex",
        descriptor="Lcom/example/Target;",
        apk_path=apk1,
    )

    tm = MagicMock(spec=ToolManager)

    def fake_baksmali(tool_name, cmd, **kwargs):
        # cmd: d <dex> --classes Lcom/example/Target; --code-offsets -o <dir>
        out_dir = Path(cmd[cmd.index("-o") + 1])
        target_file = out_dir / "com" / "example" / "Target.smali"
        target_file.parent.mkdir(parents=True, exist_ok=True)
        target_file.write_text(
            ".class public Lcom/example/Target;\n.super Ljava/lang/Object;\n"
        )
        return subprocess.CompletedProcess(cmd, 0, "", "")

    tm.run_tool_cmd.side_effect = fake_baksmali

    text = extract_smali_class(tm, location)
    assert ".class public Lcom/example/Target;" in text
    assert tm.run_tool_cmd.call_args[0][0] == "baksmali"
    cmd_args = tm.run_tool_cmd.call_args[0][1]
    assert cmd_args[0] == "d"
    assert "--classes" in cmd_args
    assert cmd_args[cmd_args.index("--classes") + 1] == "Lcom/example/Target;"
    assert "--code-offsets" in cmd_args


def test_extract_java_class_arguments(tmp_path):
    apk1 = tmp_path / "app.apk"
    apk1.write_bytes(b"apk")

    location = DexLocation(
        split="base",
        dex_name="classes.dex",
        descriptor="Lcom/example/Target;",
        apk_path=apk1,
    )

    tm = MagicMock(spec=ToolManager)

    def fake_jadx(tool_name, cmd, **kwargs):
        out_file = Path(cmd[cmd.index("--single-class-output") + 1])
        out_file.write_text("package com.example;\npublic class Target {}\n")
        return subprocess.CompletedProcess(cmd, 0, "", "")

    tm.run_tool_cmd.side_effect = fake_jadx

    text = extract_java_class(tm, location)
    assert "public class Target" in text
    cmd_args = tm.run_tool_cmd.call_args[0][1]
    assert "--single-class" in cmd_args
    assert cmd_args[cmd_args.index("--single-class") + 1] == "com.example.Target"
    assert "--show-bad-code" in cmd_args
    assert "--no-res" in cmd_args
    assert "--output-format" in cmd_args


def test_parse_smali_methods_and_overloads():
    smali = """
.class public Lcom/example/Foo;
.super Ljava/lang/Object;

.method public constructor <init>()V
    .registers 1
    invoke-direct {p0}, Ljava/lang/Object;-><init>()V
    return-void
.end method

.method public compute(I)I
    .registers 2
    add-int/lit8 v0, p1, 1
    return v0
.end method

.method public compute(II)I
    .registers 3
    add-int v0, p1, p2
    return v0
.end method
"""
    methods = parse_smali_methods(smali)
    assert len(methods) == 3
    assert methods[0].signature == "<init>()V"
    assert methods[1].signature == "compute(I)I"
    assert methods[2].signature == "compute(II)I"

    # Exact descriptor works
    m1 = extract_method_from_class(smali, "compute(I)I", "smali", "com.example.Foo")
    assert "add-int/lit8" in m1

    # Ambiguous bare name fails
    with pytest.raises(CodeInspectionError, match="Ambiguous method name 'compute'"):
        extract_method_from_class(smali, "compute", "smali", "com.example.Foo")

    # Missing method fails
    with pytest.raises(CodeInspectionError, match="Method 'bar' not found"):
        extract_method_from_class(smali, "bar", "smali", "com.example.Foo")


def test_parse_java_methods_and_descriptor_rejection():
    java = """
package com.example;

public class Foo {
    // A comment with { and }
    private String name = "hello { world }";

    public void process(int x) {
        if (x > 0) {
            System.out.println("positive");
        }
    }

    public void run() {
        System.out.println("run");
    }
}
"""
    methods = parse_java_methods(java)
    assert len(methods) == 2
    assert methods[0].name == "process"
    assert methods[1].name == "run"

    # Extraction by bare name
    proc_body = extract_method_from_class(java, "process", "java", "com.example.Foo")
    assert "public void process(int x)" in proc_body
    assert 'System.out.println("positive");' in proc_body

    # Reject descriptor with --format java
    with pytest.raises(CodeInspectionError, match="cannot be used with --format java"):
        extract_method_from_class(java, "process(I)V", "java", "com.example.Foo")


def test_diff_truncation_marker():
    old_lines = [f"line {i}\n" for i in range(600)]
    new_lines = [f"modified {i}\n" for i in range(600)]

    import difflib

    diff = list(
        difflib.unified_diff(old_lines, new_lines, fromfile="old", tofile="new")
    )
    assert len(diff) > 500
    truncated = diff[:500] + ["\n... diff truncated at 500 lines ...\n"]
    formatted = format_colored_diff(truncated)
    assert "... diff truncated at 500 lines ..." in formatted
