#!/usr/bin/env python3
"""Generate the deterministic synthetic Unity/IL2CPP corpus for plan 001.

This is fixture-generation code, not a production parser. It writes only
synthetic ZIP containers and uses the standard library. The recipes mirror the
synthetic builders in ``tests/test_unity.py`` and ``tests/test_il2cpp.py`` so
the real Python oracle can be exercised without proprietary Unity inputs.

Run from the repository root:

    python3 tools/apk-lab/migration/fixtures/binary/generate.py
    python3 tools/apk-lab/migration/fixtures/binary/generate.py \
        --output .apk-lab-migration/corpus

The default output directory is ignored by git. The generated ``generated.json``
records the pinned SHA-256/size identities and the expected query substrings;
the public inventory must never contain the generated APK bytes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
import zipfile
from pathlib import Path
from typing import Any

ZIP_DATE_TIME = (1980, 1, 1, 0, 0, 0)
ZIP_CREATE_SYSTEM = 0
ZIP_EXTERNAL_ATTR = 0
FILLER_BLOCK_BYTES = 65536
IL2CPP_METADATA_MAGIC = 0xFAB11BAF


def deterministic_block(seed: int) -> bytes:
    """Return a stable pseudo-random block without relying on library RNGs."""
    mask = 0xFFFFFFFFFFFFFFFF
    state = seed & mask or 0x9E3779B97F4A7C15
    out = bytearray(FILLER_BLOCK_BYTES)
    for index in range(FILLER_BLOCK_BYTES):
        state ^= (state << 13) & mask
        state ^= state >> 7
        state ^= (state << 17) & mask
        out[index] = state & 0xFF
    return bytes(out)


def deterministic_bytes(length: int, seed: int) -> bytes:
    if length <= 0:
        return b""
    block = deterministic_block(seed)
    repeats = (length + len(block) - 1) // len(block)
    return (block * repeats)[:length]


def gameobject_record(name: str, tag: int, active: int) -> bytes:
    name_bytes = name.encode("utf-8")
    padding = (4 - (len(name_bytes) % 4)) % 4
    return b"".join(
        (
            struct.pack("<I", len(name_bytes)),
            name_bytes,
            b"\x00" * padding,
            struct.pack("<H", tag),
            bytes([active]),
            b"\x00" * 16,
        )
    )


def unity_assets(
    fixture_id: str, params: dict[str, Any]
) -> tuple[list[tuple[str, bytes]], dict[str, Any]]:
    members = int(params["members"])
    objects_per_member = int(params["objectsPerMember"])
    target_member = int(params["targetMember"])
    target_index = int(params["targetIndex"])
    target_name = str(params["targetName"])
    tag = int(params["tag"])
    active = int(params["active"])
    prefix_padding = int(params["prefixPadding"])
    filler_before = int(params["fillerBeforeBytes"])
    filler_after = int(params["fillerAfterBytes"])
    record_prefix = str(params["recordNamePrefix"])

    target_bytes = target_name.encode("utf-8")
    seen_target = 0
    target_expected: dict[str, Any] | None = None
    entries: list[tuple[str, bytes]] = []

    for member in range(members):
        member_name = f"sharedassets{member:02d}.assets"
        path = f"assets/bin/Data/{member_name}"
        buffer = bytearray(b"\x00" * prefix_padding)
        buffer.extend(deterministic_bytes(filler_before, 0xC0FFEE + member))

        for index in range(objects_per_member):
            if member == target_member and index == target_index:
                name = target_name
            else:
                name = f"{record_prefix}_{member:02d}_{index:04d}_{fixture_id}"

            string_offset = len(buffer) + 4
            buffer.extend(gameobject_record(name, tag, active))

            if name == target_name:
                seen_target += 1
                if target_expected is None:
                    name_len = len(name.encode("utf-8"))
                    pad = (4 - (name_len % 4)) % 4
                    target_expected = {
                        "member": path,
                        "name": name,
                        "file_offset": string_offset - 4,
                        "is_active_offset": string_offset + name_len + pad + 2,
                        "is_active_value": active,
                    }

        buffer.extend(deterministic_bytes(filler_after, 0xBEEF + member))
        expected_in_member = 1 if member == target_member else 0
        if buffer.count(target_bytes) != expected_in_member:
            raise SystemExit(
                f"{fixture_id}: filler accidentally contains target name in {path}"
            )
        entries.append((path, bytes(buffer)))

    if seen_target != 1 or target_expected is None:
        raise SystemExit(
            f"{fixture_id}: expected exactly one target record, found {seen_target}"
        )
    if target_expected["is_active_value"] not in (0, 1):
        raise SystemExit(f"{fixture_id}: m_IsActive must be 0 or 1")

    expected_contains = [
        f'"member": "{target_expected["member"]}"',
        f'"name": "{target_expected["name"]}"',
        f'"file_offset": {target_expected["file_offset"]}',
        (f'"is_active_offset_hex": "0x{target_expected["is_active_offset"]:x}"'),
        f'"is_active_value": {target_expected["is_active_value"]}',
    ]
    return entries, {
        "member": target_expected["member"],
        "name": target_expected["name"],
        "fileOffset": target_expected["file_offset"],
        "isActiveOffset": target_expected["is_active_offset"],
        "isActiveValue": target_expected["is_active_value"],
        "expectedStdoutContains": expected_contains,
    }


def il2cpp_metadata_small() -> tuple[bytes, dict[str, Any]]:
    """Exact v29 recipe from tests/test_il2cpp.py:build_synthetic_metadata."""
    string_data = (
        b"\x00TestNamespace\x00BlackjackApplication\x00OpenShop\x00OtherMethod\x00"
    )
    string_offset = 256
    string_size = len(string_data)

    methods_data = b"".join(
        struct.pack("<iiiiiIHHHH", name_index, 0, 0, 0, -1, 0, 0, 0, 0, count)
        for name_index, count in ((36, 1), (45, 0))
    )
    methods_offset = string_offset + string_size + 4

    types_offset = methods_offset + len(methods_data) + 4
    type_data = bytearray(88)
    struct.pack_into("<i", type_data, 0, 15)
    struct.pack_into("<i", type_data, 4, 1)
    struct.pack_into("<i", type_data, 36, 0)
    struct.pack_into("<H", type_data, 64, 2)

    total_len = types_offset + len(type_data) + 16
    buffer = bytearray(total_len)
    struct.pack_into("<I", buffer, 0, IL2CPP_METADATA_MAGIC)
    struct.pack_into("<I", buffer, 4, 29)
    struct.pack_into("<I", buffer, 24, string_offset)
    struct.pack_into("<I", buffer, 28, string_size)
    struct.pack_into("<I", buffer, 48, methods_offset)
    struct.pack_into("<I", buffer, 52, len(methods_data))
    struct.pack_into("<I", buffer, 160, types_offset)
    struct.pack_into("<I", buffer, 164, len(type_data))
    buffer[string_offset : string_offset + string_size] = string_data
    buffer[methods_offset : methods_offset + len(methods_data)] = methods_data
    buffer[types_offset : types_offset + len(type_data)] = type_data

    expected = {
        "namespace": "TestNamespace",
        "type": "BlackjackApplication",
        "method": "OpenShop",
        "parametersCount": 1,
    }
    return bytes(buffer), expected


def il2cpp_metadata_large(params: dict[str, Any]) -> tuple[bytes, dict[str, Any]]:
    version = int(params["version"])
    method_count = int(params["methodCount"])
    type_count = int(params["typeCount"])
    methods_per_type = int(params["methodsPerType"])
    filler_string_bytes = int(params["fillerStringBytes"])
    target_method = str(params["targetMethodName"])
    target_type = str(params["targetTypeName"])
    target_namespace = str(params["targetNamespace"])
    target_parameter_count = int(params["targetParameterCount"])

    if version != 29:
        raise SystemExit("il2cpp-large: only the v29 record layout is implemented")
    if type_count * methods_per_type != method_count:
        raise SystemExit(
            "il2cpp-large: typeCount * methodsPerType must equal methodCount"
        )

    strings = bytearray(b"\x00")

    def add_string(value: str) -> int:
        offset = len(strings)
        strings.extend(value.encode("utf-8"))
        strings.append(0)
        return offset

    namespace_index = add_string(target_namespace)
    type_name_indexes = [
        add_string(f"SyntheticType_{index:04d}") for index in range(type_count)
    ]
    method_name_indexes = [add_string(target_method)]
    for index in range(1, method_count):
        method_name_indexes.append(add_string(f"SyntheticMethod_{index:06d}"))

    filler_offset = len(strings)
    strings.extend(b"F" * filler_string_bytes)
    strings.append(0)

    if strings.count(target_method.encode("utf-8")) != 1:
        raise SystemExit("il2cpp-large: target method name is not unique")

    methods_offset = 256 + len(strings) + 4
    methods_data = bytearray(method_count * 32)
    for index, name_index in enumerate(method_name_indexes):
        parameter_count = (
            target_parameter_count if index == 0 else min(index % 4, 0xFFFF)
        )
        struct.pack_into(
            "<iiiiiIHHHH",
            methods_data,
            index * 32,
            name_index,
            0,
            0,
            0,
            -1,
            0,
            0,
            0,
            0,
            parameter_count,
        )

    types_offset = methods_offset + len(methods_data) + 4
    types_data = bytearray(type_count * 88)
    for index in range(type_count):
        base = index * 88
        struct.pack_into("<i", types_data, base, type_name_indexes[index])
        struct.pack_into("<i", types_data, base + 4, namespace_index)
        struct.pack_into("<i", types_data, base + 36, index * methods_per_type)
        struct.pack_into("<H", types_data, base + 64, methods_per_type)

    total_len = types_offset + len(types_data) + 16
    buffer = bytearray(total_len)
    struct.pack_into("<I", buffer, 0, IL2CPP_METADATA_MAGIC)
    struct.pack_into("<I", buffer, 4, version)
    struct.pack_into("<I", buffer, 24, 256)
    struct.pack_into("<I", buffer, 28, len(strings))
    struct.pack_into("<I", buffer, 48, methods_offset)
    struct.pack_into("<I", buffer, 52, len(methods_data))
    struct.pack_into("<I", buffer, 160, types_offset)
    struct.pack_into("<I", buffer, 164, len(types_data))
    buffer[256 : 256 + len(strings)] = strings
    buffer[methods_offset : methods_offset + len(methods_data)] = methods_data
    buffer[types_offset : types_offset + len(types_data)] = types_data

    expected = {
        "namespace": target_namespace,
        "type": target_type,
        "method": target_method,
        "parametersCount": target_parameter_count,
        "fillerStringOffset": filler_offset,
    }
    return bytes(buffer), expected


def il2cpp_entries(metadata: bytes, native_lib_bytes: int) -> list[tuple[str, bytes]]:
    entries: list[tuple[str, bytes]] = [
        (
            "AndroidManifest.xml",
            b'<manifest package="com.example.synthetic.il2cpp"/>',
        ),
        (
            "assets/bin/Data/Managed/Metadata/global-metadata.dat",
            metadata,
        ),
    ]
    if native_lib_bytes:
        entries.append(
            (
                "lib/arm64-v8a/libil2cpp.so",
                b"\x7fELF synthetic il2cpp\n"
                + deterministic_bytes(native_lib_bytes, 0x51504C32),
            )
        )
    return entries


def write_apk(path: Path, entries: list[tuple[str, bytes]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, data in entries:
            info = zipfile.ZipInfo(name, date_time=ZIP_DATE_TIME)
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = ZIP_CREATE_SYSTEM
            info.external_attr = ZIP_EXTERNAL_ATTR
            archive.writestr(info, data)


def build_fixture(fixture: dict[str, Any], output_dir: Path) -> dict[str, Any]:
    fixture_id = str(fixture["id"])
    builder = str(fixture["builder"])
    params = fixture["params"]
    filename = str(fixture["filename"])
    query = str(fixture["query"])

    if builder == "unity":
        entries, expected = unity_assets(fixture_id, params)
    elif builder == "il2cpp":
        if params.get("variant") == "test-recipe":
            metadata, expected = il2cpp_metadata_small()
        else:
            metadata, expected = il2cpp_metadata_large(params)
        entries = il2cpp_entries(metadata, int(params.get("nativeLibBytes", 0)))
    else:
        raise SystemExit(f"{fixture_id}: unknown builder {builder!r}")

    path = output_dir / filename
    write_apk(path, entries)
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    size = path.stat().st_size

    if builder == "unity":
        expected_contains = expected.pop("expectedStdoutContains")
    else:
        expected_contains = [
            f'"namespace": "{expected["namespace"]}"',
            f'"type": "{expected["type"]}"',
            f'"method": "{expected["method"]}"',
            f'"parameters_count": {expected["parametersCount"]}',
        ]

    return {
        "id": fixture_id,
        "filename": filename,
        "builder": builder,
        "query": query,
        "sha256": digest,
        "sizeBytes": size,
        "expected": expected,
        "expectedStdoutContains": expected_contains,
    }


def main(argv: list[str] | None = None) -> int:
    script_dir = Path(__file__).resolve().parent
    spec = json.loads((script_dir / "spec.json").read_text(encoding="utf-8"))
    if spec.get("schemaVersion") != 1:
        print("unsupported spec schemaVersion", file=sys.stderr)
        return 2

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default=spec["defaultOutput"])
    args = parser.parse_args(argv)

    output_dir = Path(args.output)
    if not output_dir.is_absolute():
        output_dir = Path.cwd() / output_dir
    output_dir.mkdir(parents=True, exist_ok=True)

    records = [build_fixture(fixture, output_dir) for fixture in spec["fixtures"]]
    document = {
        "schemaVersion": 1,
        "kind": "apk-lab-synthetic-binary-corpus",
        "generator": "tools/apk-lab/migration/fixtures/binary/generate.py",
        "spec": "tools/apk-lab/migration/fixtures/binary/spec.json",
        "outputDirectory": args.output,
        "inputs": records,
    }
    (output_dir / "generated.json").write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    for record in records:
        print(
            f"{record['sha256']}  {record['sizeBytes']:>10}  "
            f"{record['filename']}  query={record['query']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
