from __future__ import annotations

import io
import re
import struct
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# Magic constant for global-metadata.dat
IL2CPP_METADATA_MAGIC = 0xFAB11BAF
SUPPORTED_VERSIONS_MIN = 24
SUPPORTED_VERSIONS_MAX = 39

METADATA_HEADER_FIELD_OFFSETS = {
    "stringLiteralOffset": 8,
    "stringLiteralSize": 12,
    "stringLiteralDataOffset": 16,
    "stringLiteralDataSize": 20,
    "stringOffset": 24,
    "stringSize": 28,
    "eventsOffset": 32,
    "eventsSize": 36,
    "propertiesOffset": 40,
    "propertiesSize": 44,
    "methodsOffset": 48,
    "methodsSize": 52,
    "parameterDefaultValuesOffset": 56,
    "parameterDefaultValuesSize": 60,
    "fieldDefaultValuesOffset": 64,
    "fieldDefaultValuesSize": 68,
    "fieldAndParameterDefaultValueDataOffset": 72,
    "fieldAndParameterDefaultValueDataSize": 76,
    "fieldMarshaledSizesOffset": 80,
    "fieldMarshaledSizesSize": 84,
    "parametersOffset": 88,
    "parametersSize": 92,
    "fieldsOffset": 96,
    "fieldsSize": 100,
    "genericParametersOffset": 104,
    "genericParametersSize": 108,
    "genericParameterConstraintsOffset": 112,
    "genericParameterConstraintsSize": 116,
    "genericContainersOffset": 120,
    "genericContainersSize": 124,
    "nestedTypesOffset": 128,
    "nestedTypesSize": 132,
    "interfacesOffset": 136,
    "interfacesSize": 140,
    "vtableMethodsOffset": 144,
    "vtableMethodsSize": 148,
    "interfaceOffsetsOffset": 152,
    "interfaceOffsetsSize": 156,
    "typeDefinitionsOffset": 160,
    "typeDefinitionsSize": 164,
}


@dataclass
class Il2CppTypeDefinition:
    name: str
    namespace: str
    method_start_index: int = 0
    method_count: int = 0
    name_index: int = 0
    namespace_index: int = 0


@dataclass
class Il2CppMethodDefinition:
    name: str
    return_type_idx: int = 0
    parameter_start_index: int = 0
    parameter_count: int = 0
    name_index: int = 0


@dataclass
class Il2CppMethodMatch:
    namespace: str
    type_name: str
    method_name: str
    parameter_count: int = 0


class Il2CppMetadata:
    """Parser for Unity IL2CPP global-metadata.dat binary files."""

    def __init__(self, data: bytes) -> None:
        if len(data) < 256:
            raise ValueError(f"Metadata file too small ({len(data)} bytes, minimum 256)")

        self.data = data
        self.sanity = struct.unpack_from("<I", data, 0)[0]
        if self.sanity != IL2CPP_METADATA_MAGIC:
            raise ValueError(
                f"Invalid metadata file magic: 0x{self.sanity:08x} "
                f"(expected 0x{IL2CPP_METADATA_MAGIC:08x})"
            )

        self.version = struct.unpack_from("<I", data, 4)[0]
        if not (SUPPORTED_VERSIONS_MIN <= self.version <= SUPPORTED_VERSIONS_MAX):
            # We record version but warn/proceed if reasonable
            pass

        self.headers: dict[str, int] = {}
        for field, offset in METADATA_HEADER_FIELD_OFFSETS.items():
            if offset + 4 <= len(data):
                self.headers[field] = struct.unpack_from("<I", data, offset)[0]
            else:
                self.headers[field] = 0

        self.string_offset = self.headers.get("stringOffset", 0)
        self.string_size = self.headers.get("stringSize", 0)
        self.methods_offset = self.headers.get("methodsOffset", 0)
        self.methods_size = self.headers.get("methodsSize", 0)
        self.type_definitions_offset = self.headers.get("typeDefinitionsOffset", 0)
        self.type_definitions_size = self.headers.get("typeDefinitionsSize", 0)

        self.method_definitions: list[Il2CppMethodDefinition] = []
        self.type_definitions: list[Il2CppTypeDefinition] = []

        self._parse_method_definitions()
        self._parse_type_definitions()

    def get_string_from_index(self, index: int) -> str:
        """Reads a null-terminated UTF-8 string from the string table at the given offset."""
        if index < 0 or index >= self.string_size:
            return ""
        abs_offset = self.string_offset + index
        if abs_offset >= len(self.data):
            return ""
        end = self.data.find(b"\x00", abs_offset)
        if end == -1:
            end = min(abs_offset + 256, len(self.data))
        return self.data[abs_offset:end].decode("utf-8", errors="replace")

    def _parse_method_definitions(self) -> None:
        """Parses the method definitions table.

        Standard struct size for Il2CppMethodDefinition in v24..v29 is 32 bytes:
          0: nameIndex (int32)
          4: declaringType (int32)
          8: returnType (int32)
         12: parameterStart (int32)
         16: genericContainerIndex (int32)
         20: token (uint32)
         24: flags (uint16)
         26: iflags (uint16)
         28: slot (uint16)
         30: parameterCount (uint16)
        """
        method_def_size = 32
        if self.methods_offset == 0 or self.methods_size == 0:
            return

        count = self.methods_size // method_def_size
        for i in range(count):
            offset = self.methods_offset + i * method_def_size
            if offset + method_def_size > len(self.data):
                break
            name_idx, _decl_type, return_type_idx, param_start, _gen, _token, _fl, _ifl, _slot, param_count = struct.unpack_from(
                "<iiiiiIHHHH", self.data, offset
            )
            name = self.get_string_from_index(name_idx)
            self.method_definitions.append(
                Il2CppMethodDefinition(
                    name=name,
                    return_type_idx=return_type_idx,
                    parameter_start_index=param_start,
                    parameter_count=param_count,
                    name_index=name_idx,
                )
            )

    def _parse_type_definitions(self) -> None:
        """Parses the type definitions table.

        In v24-v29, Il2CppTypeDefinition size is typically 88, 96, or 100 bytes.
        Key field offsets:
          0: nameIndex (int32)
          4: namespaceIndex (int32)
         44: methodStart (int32)
         72: method_count (uint16)
        """
        if self.type_definitions_offset == 0 or self.type_definitions_size == 0:
            return

        type_def_size = 88
        if self.type_definitions_size % 88 != 0:
            if self.type_definitions_size % 100 == 0:
                type_def_size = 100
            elif self.type_definitions_size % 96 == 0:
                type_def_size = 96

        count = self.type_definitions_size // type_def_size
        for i in range(count):
            offset = self.type_definitions_offset + i * type_def_size
            if offset + type_def_size > len(self.data):
                break
            name_idx = struct.unpack_from("<i", self.data, offset)[0]
            namespace_idx = struct.unpack_from("<i", self.data, offset + 4)[0]

            method_start = 0
            method_count = 0
            if offset + 48 <= len(self.data):
                method_start = struct.unpack_from("<i", self.data, offset + 44)[0]
            if offset + 74 <= len(self.data):
                method_count = struct.unpack_from("<H", self.data, offset + 72)[0]

            name = self.get_string_from_index(name_idx)
            namespace = self.get_string_from_index(namespace_idx)

            self.type_definitions.append(
                Il2CppTypeDefinition(
                    name=name,
                    namespace=namespace,
                    method_start_index=method_start,
                    method_count=method_count,
                    name_index=name_idx,
                    namespace_index=namespace_idx,
                )
            )


class Il2CppSymbolMap:
    """Aggregates Il2Cpp types, method names, and provides fast symbol search."""

    def __init__(self, metadata: Il2CppMetadata) -> None:
        self.metadata = metadata
        self.matches: list[Il2CppMethodMatch] = []

        for tdef in metadata.type_definitions:
            for i in range(tdef.method_count):
                m_idx = tdef.method_start_index + i
                if 0 <= m_idx < len(metadata.method_definitions):
                    mdef = metadata.method_definitions[m_idx]
                    self.matches.append(
                        Il2CppMethodMatch(
                            namespace=tdef.namespace,
                            type_name=tdef.name,
                            method_name=mdef.name,
                            parameter_count=mdef.parameter_count,
                        )
                    )

    def search(self, query: str | None = None) -> list[Il2CppMethodMatch]:
        """Filters methods by case-insensitive substring or regex query across namespace, type, or method."""
        if not query:
            return list(self.matches)

        # Check if query is a valid regex
        try:
            pattern = re.compile(query, re.IGNORECASE)
            use_regex = True
        except re.error:
            pattern = None
            use_regex = False
        q_lower = query.lower()

        results: list[Il2CppMethodMatch] = []
        for m in self.matches:
            full_sig = f"{m.namespace}.{m.type_name}.{m.method_name}"
            if use_regex and pattern is not None and (
                pattern.search(m.method_name)
                or pattern.search(m.type_name)
                or pattern.search(full_sig)
            ):
                    results.append(m)
                    continue

            if (
                q_lower in m.method_name.lower()
                or q_lower in m.type_name.lower()
                or q_lower in full_sig.lower()
            ):
                results.append(m)

        return results


def extract_il2cpp_metadata_from_artifact(
    artifact_path: Path,
) -> tuple[bytes, dict[str, bytes]]:
    """Extracts `global-metadata.dat` and native `.so` files from an APK or multi-split container.

    Returns (metadata_bytes, {lib_relative_path: lib_bytes}).
    Raises FileNotFoundError if global-metadata.dat cannot be located.
    """
    metadata_rel_path = "assets/bin/Data/Managed/Metadata/global-metadata.dat"
    metadata_bytes: bytes | None = None
    native_libs: dict[str, bytes] = {}

    if not artifact_path.is_file():
        raise FileNotFoundError(f"Artifact file not found: {artifact_path}")

    with zipfile.ZipFile(artifact_path, "r") as zf:
        namelist = zf.namelist()

        # Check if direct APK
        if metadata_rel_path in namelist:
            metadata_bytes = zf.read(metadata_rel_path)
            for name in namelist:
                if name.startswith("lib/") and name.endswith(".so"):
                    native_libs[name] = zf.read(name)
            return metadata_bytes, native_libs

        # Check for split container (APKM, APKS, XAPK) containing inner APKs
        apk_members = [m for m in namelist if m.endswith(".apk")]
        if apk_members:
            for apk_member in apk_members:
                inner_bytes = zf.read(apk_member)
                with zipfile.ZipFile(io.BytesIO(inner_bytes), "r") as inner_zf:
                    inner_names = inner_zf.namelist()
                    if metadata_rel_path in inner_names:
                        metadata_bytes = inner_zf.read(metadata_rel_path)
                    for name in inner_names:
                        if name.startswith("lib/") and name.endswith(".so"):
                            native_libs[name] = inner_zf.read(name)

            if metadata_bytes is not None:
                return metadata_bytes, native_libs

    raise FileNotFoundError("global-metadata.dat not found in artifact")


def analyze_il2cpp(
    artifact_path: Path, query: str | None = None
) -> list[dict[str, Any]]:
    """Analyzes an artifact for IL2CPP metadata and returns matching symbol dictionaries."""
    metadata_bytes, _native_libs = extract_il2cpp_metadata_from_artifact(artifact_path)
    metadata = Il2CppMetadata(metadata_bytes)
    symbol_map = Il2CppSymbolMap(metadata)
    matches = symbol_map.search(query)

    records: list[dict[str, Any]] = []
    for match in matches:
        records.append(
            {
                "namespace": match.namespace,
                "type": match.type_name,
                "method": match.method_name,
                "parameters_count": match.parameter_count,
            }
        )
    return records
