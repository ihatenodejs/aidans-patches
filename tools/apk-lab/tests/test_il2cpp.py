from __future__ import annotations

import io
import struct
import zipfile

import pytest
from apk_lab.cli import build_parser, handle_il2cpp
from apk_lab.il2cpp import (
    IL2CPP_METADATA_MAGIC,
    Il2CppMetadata,
    Il2CppSymbolMap,
    analyze_il2cpp,
    extract_il2cpp_metadata_from_artifact,
)
from apk_lab.models import ExitCode


def build_synthetic_metadata(version: int = 29) -> bytes:
    """Builds a minimal valid synthetic global-metadata.dat buffer."""
    # String table
    # Index 0: empty
    # Index 1: "TestNamespace"
    # Index 15: "BlackjackApplication"
    # Index 36: "OpenShop"
    string_data = b"\x00TestNamespace\x00BlackjackApplication\x00OpenShop\x00OtherMethod\x00"
    string_offset = 256
    string_size = len(string_data)

    # Method definitions table (32 bytes per method)
    # Method 0: "OpenShop", paramCount = 1
    # Method 1: "OtherMethod", paramCount = 0
    # struct format: "<iiiiiIHHHH"
    # name_idx, decl_type, return_type, param_start, gen_container, token, flags, iflags, slot, param_count
    methods_offset = string_offset + string_size + 4
    method0 = struct.pack("<iiiiiIHHHH", 36, 0, 0, 0, -1, 0, 0, 0, 0, 1)
    method1 = struct.pack("<iiiiiIHHHH", 45, 0, 0, 0, -1, 0, 0, 0, 0, 0)
    methods_data = method0 + method1
    methods_size = len(methods_data)

    # Type definitions table (88 bytes per type)
    # Type 0: "BlackjackApplication", namespace="TestNamespace", methodStart=0, methodCount=2
    types_offset = methods_offset + methods_size + 4
    type0_buf = bytearray(88)
    struct.pack_into("<i", type0_buf, 0, 15)  # nameIndex: "BlackjackApplication"
    struct.pack_into("<i", type0_buf, 4, 1)  # namespaceIndex: "TestNamespace"
    struct.pack_into("<i", type0_buf, 44, 0)  # methodStart = 0
    struct.pack_into("<H", type0_buf, 72, 2)  # methodCount = 2
    types_data = bytes(type0_buf)
    types_size = len(types_data)

    total_len = types_offset + types_size + 16
    buf = bytearray(total_len)

    # Header
    struct.pack_into("<I", buf, 0, IL2CPP_METADATA_MAGIC)
    struct.pack_into("<I", buf, 4, version)
    struct.pack_into("<I", buf, 24, string_offset)
    struct.pack_into("<I", buf, 28, string_size)
    struct.pack_into("<I", buf, 48, methods_offset)
    struct.pack_into("<I", buf, 52, methods_size)
    struct.pack_into("<I", buf, 160, types_offset)
    struct.pack_into("<I", buf, 164, types_size)

    # Copy section data
    buf[string_offset : string_offset + string_size] = string_data
    buf[methods_offset : methods_offset + methods_size] = methods_data
    buf[types_offset : types_offset + types_size] = types_data

    return bytes(buf)


def test_il2cpp_metadata_parsing():
    raw = build_synthetic_metadata()
    meta = Il2CppMetadata(raw)

    assert meta.sanity == IL2CPP_METADATA_MAGIC
    assert meta.version == 29
    assert meta.get_string_from_index(1) == "TestNamespace"
    assert meta.get_string_from_index(15) == "BlackjackApplication"
    assert meta.get_string_from_index(36) == "OpenShop"

    assert len(meta.method_definitions) == 2
    assert meta.method_definitions[0].name == "OpenShop"
    assert meta.method_definitions[0].parameter_count == 1
    assert meta.method_definitions[1].name == "OtherMethod"

    assert len(meta.type_definitions) == 1
    assert meta.type_definitions[0].name == "BlackjackApplication"
    assert meta.type_definitions[0].namespace == "TestNamespace"
    assert meta.type_definitions[0].method_count == 2


def test_il2cpp_symbol_map_query():
    raw = build_synthetic_metadata()
    meta = Il2CppMetadata(raw)
    smap = Il2CppSymbolMap(meta)

    # All symbols
    all_matches = smap.search()
    assert len(all_matches) == 2

    # Query matching method substring
    query_shop = smap.search("OpenShop")
    assert len(query_shop) == 1
    assert query_shop[0].method_name == "OpenShop"
    assert query_shop[0].type_name == "BlackjackApplication"

    # Query matching type name
    query_type = smap.search("Blackjack")
    assert len(query_type) == 2

    # Query with no matches
    query_none = smap.search("NonExistentMethod")
    assert len(query_none) == 0

    # Regex query
    query_regex = smap.search(r"^Open.*")
    assert len(query_regex) == 1
    assert query_regex[0].method_name == "OpenShop"


def test_il2cpp_invalid_magic_rejected():
    raw = bytearray(build_synthetic_metadata())
    struct.pack_into("<I", raw, 0, 0x12345678)
    with pytest.raises(ValueError, match="Invalid metadata file magic"):
        Il2CppMetadata(bytes(raw))

    # Too small buffer
    with pytest.raises(ValueError, match="too small"):
        Il2CppMetadata(b"short")


def test_extract_il2cpp_metadata_from_split_artifact(tmp_path):
    raw_meta = build_synthetic_metadata()

    # Create base APK with metadata
    base_apk = io.BytesIO()
    with zipfile.ZipFile(base_apk, "w") as zf:
        zf.writestr("assets/bin/Data/Managed/Metadata/global-metadata.dat", raw_meta)
        zf.writestr("AndroidManifest.xml", b"<manifest/>")

    # Create split APK with native library
    split_apk = io.BytesIO()
    with zipfile.ZipFile(split_apk, "w") as zf:
        zf.writestr("lib/arm64-v8a/libil2cpp.so", b"il2cpp_arm64_bytes")

    # Create outer APKS container
    container_file = tmp_path / "game.apks"
    with zipfile.ZipFile(container_file, "w") as zf:
        zf.writestr("splits/base.apk", base_apk.getvalue())
        zf.writestr("splits/split.arm64_v8a.apk", split_apk.getvalue())

    meta_bytes, native_libs = extract_il2cpp_metadata_from_artifact(container_file)
    assert meta_bytes == raw_meta
    assert "lib/arm64-v8a/libil2cpp.so" in native_libs
    assert native_libs["lib/arm64-v8a/libil2cpp.so"] == b"il2cpp_arm64_bytes"

    # analyze_il2cpp integration
    records = analyze_il2cpp(container_file, query="OpenShop")
    assert len(records) == 1
    assert records[0]["type"] == "BlackjackApplication"
    assert records[0]["method"] == "OpenShop"

    # Artifact without metadata raises FileNotFoundError
    empty_apk = tmp_path / "empty.apk"
    with zipfile.ZipFile(empty_apk, "w") as zf:
        zf.writestr("AndroidManifest.xml", b"<manifest/>")

    with pytest.raises(FileNotFoundError, match="global-metadata.dat not found"):
        extract_il2cpp_metadata_from_artifact(empty_apk)


def test_cli_il2cpp_integration(tmp_path, capsys):
    raw_meta = build_synthetic_metadata()
    apk_file = tmp_path / "game.apk"
    with zipfile.ZipFile(apk_file, "w") as zf:
        zf.writestr("assets/bin/Data/Managed/Metadata/global-metadata.dat", raw_meta)

    parser = build_parser()

    # CLI query with formatted table
    args = parser.parse_args(["il2cpp", str(apk_file), "--query", "OpenShop"])
    code = handle_il2cpp(args)
    assert code == ExitCode.SUCCESS
    out = capsys.readouterr().out
    assert "OpenShop" in out
    assert "BlackjackApplication" in out

    # CLI query with JSON stdout
    args = parser.parse_args(["il2cpp", str(apk_file), "--query", "OpenShop", "--json"])
    code = handle_il2cpp(args)
    assert code == ExitCode.SUCCESS
    out = capsys.readouterr().out
    assert '"method": "OpenShop"' in out

    # CLI on artifact with missing metadata returns USAGE_OR_TOOL_ERROR
    empty_apk = tmp_path / "empty.apk"
    with zipfile.ZipFile(empty_apk, "w") as zf:
        zf.writestr("AndroidManifest.xml", b"<manifest/>")

    args = parser.parse_args(["il2cpp", str(empty_apk)])
    code = handle_il2cpp(args)
    assert code == ExitCode.USAGE_OR_TOOL_ERROR
