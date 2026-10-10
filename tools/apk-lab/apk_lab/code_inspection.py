from __future__ import annotations

import argparse
import difflib
import re
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path

from apk_lab.archives import MaterializedApk, materialize_artifact_apks
from apk_lab.inspection import find_build_tools_bin, inspect_artifact
from apk_lab.models import ExitCode
from apk_lab.morphe import parse_dexdump_class_descriptors
from apk_lab.tools import ToolManager


class CodeInspectionError(Exception):
    """Raised when class or method extraction or comparison fails."""

    def __init__(
        self, message: str, exit_code: int = ExitCode.USAGE_OR_TOOL_ERROR
    ) -> None:
        super().__init__(message)
        self.exit_code = exit_code


@dataclass(frozen=True)
class DexLocation:
    split: str
    dex_name: str
    descriptor: str
    apk_path: Path


@dataclass(frozen=True)
class ParsedMethod:
    name: str
    signature: str
    declaration: str
    content: str


def normalize_fqcn(name: str) -> tuple[str, str]:
    """Normalizes class name into (fqcn, descriptor).

    Accepts 'com.example.app.MyClass' or 'Lcom/example/app/MyClass;'.
    """
    clean = name.strip()
    if clean.startswith("L") and clean.endswith(";"):
        descriptor = clean
        fqcn = clean[1:-1].replace("/", ".")
    else:
        fqcn = clean.replace("/", ".")
        descriptor = f"L{fqcn.replace('.', '/')};"
    return fqcn, descriptor


def locate_class(
    materialized_apks: list[MaterializedApk],
    fqcn: str,
    *,
    allow_missing: bool = False,
) -> DexLocation | None:
    """Locates a class definition across classes*.dex entries in materialized APK splits."""
    _, descriptor = normalize_fqcn(fqcn)
    dexdump = find_build_tools_bin("dexdump")

    matches: list[DexLocation] = []
    searched: list[str] = []

    with tempfile.TemporaryDirectory(prefix="apk-lab-locate-") as tmp_dir:
        tmp_path = Path(tmp_dir)

        for mat in materialized_apks:
            with zipfile.ZipFile(mat.path, "r") as zf:
                dex_names = [
                    name
                    for name in zf.namelist()
                    if re.match(r"^classes\d*\.dex$", name)
                ]

                for dex_name in dex_names:
                    searched.append(f"{mat.split_name}:{dex_name}")
                    extracted_dex = tmp_path / f"{mat.split_name}_{dex_name}"
                    extracted_dex.write_bytes(zf.read(dex_name))

                    res = subprocess.run(
                        [str(dexdump), "-c", str(extracted_dex)],
                        capture_output=True,
                        text=True,
                        errors="replace",
                        check=False,
                    )
                    if res.returncode == 0:
                        descriptors = parse_dexdump_class_descriptors(res.stdout)
                        if descriptor in descriptors:
                            matches.append(
                                DexLocation(
                                    split=mat.split_name,
                                    dex_name=dex_name,
                                    descriptor=descriptor,
                                    apk_path=mat.path,
                                )
                            )
                    extracted_dex.unlink(missing_ok=True)

    if not matches:
        if allow_missing:
            return None
        raise CodeInspectionError(
            f"Class '{fqcn}' not found in artifact. Searched splits and DEX files: {searched}",
            ExitCode.USAGE_OR_TOOL_ERROR,
        )

    if len(matches) > 1:
        locs = [f"{m.split}:{m.dex_name}" for m in matches]
        raise CodeInspectionError(
            f"Duplicate class definitions found for '{fqcn}': {locs}",
            ExitCode.USAGE_OR_TOOL_ERROR,
        )

    return matches[0]


def extract_smali_class(
    tool_manager: ToolManager,
    location: DexLocation,
) -> str:
    _, descriptor = normalize_fqcn(location.descriptor)
    with tempfile.TemporaryDirectory(prefix="apk-lab-baksmali-") as tmp_dir:
        tmp_path = Path(tmp_dir)

        with zipfile.ZipFile(location.apk_path, "r") as zf:
            dex_bytes = zf.read(location.dex_name)
        extracted_dex = tmp_path / location.dex_name
        extracted_dex.write_bytes(dex_bytes)

        out_dir = tmp_path / "smali_out"
        cmd = [
            "d",
            str(extracted_dex),
            "--classes",
            descriptor,
            "--code-offsets",
            "-o",
            str(out_dir),
        ]
        proc = tool_manager.run_tool_cmd(
            "baksmali", cmd, capture_output=True, check=False
        )
        if proc.returncode != 0:
            raise CodeInspectionError(
                f"baksmali failed (code {proc.returncode}): {proc.stderr[:300]}",
                ExitCode.INFRASTRUCTURE_FAILURE,
            )

        # baksmali outputs <out_dir>/path/to/Class.smali
        rel_path = descriptor[1:-1] + ".smali"
        smali_file = out_dir / rel_path
        if not smali_file.is_file():
            # Try searching for any .smali file
            smali_files = list(out_dir.rglob("*.smali"))
            if smali_files:
                smali_file = smali_files[0]
            else:
                raise CodeInspectionError(
                    f"baksmali did not emit expected smali file for {descriptor}",
                    ExitCode.INFRASTRUCTURE_FAILURE,
                )

        return smali_file.read_text(encoding="utf-8")


def extract_java_class(
    tool_manager: ToolManager,
    location: DexLocation,
) -> str:
    """Decompiles only the target class from the containing physical APK using pinned JADX."""
    fqcn, _ = normalize_fqcn(location.descriptor)
    with tempfile.TemporaryDirectory(prefix="apk-lab-jadx-") as tmp_dir:
        tmp_path = Path(tmp_dir)
        out_file = tmp_path / "Output.java"

        cmd = [
            "--single-class",
            fqcn,
            "--single-class-output",
            str(out_file),
            "--show-bad-code",
            "--no-res",
            "--output-format",
            "java",
            str(location.apk_path),
        ]
        proc = tool_manager.run_tool_cmd("jadx", cmd, capture_output=True, check=False)
        # jadx exits with code 3 on warnings/partial decompilation
        if proc.returncode not in (0, 3) or not out_file.is_file():
            raise CodeInspectionError(
                f"JADX decompilation failed (code {proc.returncode}): {proc.stderr[:300]}",
                ExitCode.INFRASTRUCTURE_FAILURE,
            )

        return out_file.read_text(encoding="utf-8")


def parse_smali_methods(smali_text: str) -> list[ParsedMethod]:
    """Parses methods from Smali text from .method to .end method."""
    methods: list[ParsedMethod] = []
    pattern = re.compile(
        r"^(\.method\s+.*?([^\s(]+)\(([^)]*)\)([^\s]+))\s*$(.*?)^(\.end\s+method)\s*$",
        re.MULTILINE | re.DOTALL,
    )

    for m in pattern.finditer(smali_text):
        decl = m.group(1).strip()
        method_name = m.group(2)
        params = m.group(3)
        ret = m.group(4)
        sig = f"{method_name}({params}){ret}"
        full_content = m.group(0).strip()
        methods.append(
            ParsedMethod(
                name=method_name,
                signature=sig,
                declaration=decl,
                content=full_content,
            )
        )

    return methods


def parse_java_methods(java_text: str) -> list[ParsedMethod]:
    """Lexically parses Java method boundaries balancing braces and skipping literals/comments."""
    methods: list[ParsedMethod] = []

    # Strip line comments and block comments while preserving newlines
    # to maintain clean indexing
    cleaned: list[str] = []
    i = 0
    n = len(java_text)
    in_line_comment = False
    in_block_comment = False
    in_string = False
    in_char = False

    while i < n:
        c = java_text[i]
        nxt = java_text[i + 1] if i + 1 < n else ""

        if in_line_comment:
            if c == "\n":
                in_line_comment = False
                cleaned.append("\n")
            else:
                cleaned.append(" ")
            i += 1
            continue

        if in_block_comment:
            if c == "*" and nxt == "/":
                in_block_comment = False
                cleaned.append("  ")
                i += 2
            elif c == "\n":
                cleaned.append("\n")
                i += 1
            else:
                cleaned.append(" ")
                i += 1
            continue

        if in_string:
            if c == "\\" and nxt:
                cleaned.append("  ")
                i += 2
            elif c == '"':
                in_string = False
                cleaned.append('"')
                i += 1
            else:
                cleaned.append(c)
                i += 1
            continue

        if in_char:
            if c == "\\" and nxt:
                cleaned.append("  ")
                i += 2
            elif c == "'":
                in_char = False
                cleaned.append("'")
                i += 1
            else:
                cleaned.append(c)
                i += 1
            continue

        # Normal code
        if c == "/" and nxt == "/":
            in_line_comment = True
            cleaned.append("  ")
            i += 2
            continue
        if c == "/" and nxt == "*":
            in_block_comment = True
            cleaned.append("  ")
            i += 2
            continue
        if c == '"':
            in_string = True
            cleaned.append('"')
            i += 1
            continue
        if c == "'":
            in_char = True
            cleaned.append("'")
            i += 1
            continue

        cleaned.append(c)
        i += 1

    clean_str = "".join(cleaned)

    # Walk through clean_str and balance braces
    # Depth 0: package, imports, class declaration
    # Depth 1: class members (methods, fields, inner classes)
    brace_depth = 0
    class_start_idx = -1
    for idx, char in enumerate(clean_str):
        if char == "{":
            brace_depth += 1
            if brace_depth == 1:
                class_start_idx = idx
                break

    if class_start_idx == -1:
        return []

    # Scan for methods at depth 1
    depth = 1
    method_start_idx = -1
    method_name = ""
    method_sig = ""
    method_decl = ""

    idx = class_start_idx + 1
    while idx < n:
        char = clean_str[idx]

        if char == "{":
            depth += 1
            if depth == 2:
                # Potential method open brace!
                # Look backwards from idx to find method signature
                header = clean_str[
                    class_start_idx + 1
                    if method_start_idx == -1
                    else method_start_idx : idx
                ]
                # Check if header contains a method declaration: e.g. `name(...)`
                sig_match = re.search(
                    r"([A-Za-z0-9_$]+)\s*\([^)]*\)\s*(?:throws\s+[^{]+)?$",
                    header.strip(),
                )
                if sig_match:
                    method_name = sig_match.group(1)
                    method_sig = sig_match.group(0).strip()
                    method_decl = java_text[
                        class_start_idx + 1
                        if method_start_idx == -1
                        else method_start_idx : idx
                    ].strip()
                    method_start_idx = idx - len(method_decl)
                else:
                    # Inner class or static initializer
                    method_start_idx = -1
        elif char == "}":
            depth -= 1
            if depth == 1 and method_name and method_start_idx != -1:
                # Completed method body
                full_body = java_text[method_start_idx : idx + 1].strip()
                methods.append(
                    ParsedMethod(
                        name=method_name,
                        signature=method_sig,
                        declaration=method_decl,
                        content=full_body,
                    )
                )
                method_name = ""
                method_sig = ""
                method_decl = ""
                method_start_idx = idx + 1
            elif depth == 1:
                method_start_idx = idx + 1

        idx += 1

    return methods


def extract_method_from_class(
    code_text: str,
    method_selector: str,
    fmt: str,
    fqcn: str,
) -> str:
    """Extracts a single method by name or descriptor, enforcing ambiguity and descriptor rules."""
    if fmt == "java" and "(" in method_selector:
        raise CodeInspectionError(
            f"Dalvik descriptors like '{method_selector}' cannot be used with --format java. Use bare method name.",
            ExitCode.USAGE_OR_TOOL_ERROR,
        )

    if fmt == "smali":
        methods = parse_smali_methods(code_text)
    else:
        methods = parse_java_methods(code_text)

    if "(" in method_selector:
        # Exact descriptor match (Smali only)
        matching = [m for m in methods if m.signature == method_selector]
    else:
        # Bare name match
        matching = [m for m in methods if m.name == method_selector]

    if not matching:
        available = [m.signature for m in methods]
        raise CodeInspectionError(
            f"Method '{method_selector}' not found in class '{fqcn}'. Available methods: {available}",
            ExitCode.USAGE_OR_TOOL_ERROR,
        )

    if len(matching) > 1:
        overloads = [m.signature for m in matching]
        raise CodeInspectionError(
            f"Ambiguous method name '{method_selector}' in class '{fqcn}'. Available overloads: {overloads}",
            ExitCode.USAGE_OR_TOOL_ERROR,
        )

    return matching[0].content


def format_colored_diff(diff_lines: list[str]) -> str:
    """Applies ANSI red/green/cyan colors to diff lines when running in a TTY."""
    use_color = sys.stdout.isatty()
    output: list[str] = []

    for line in diff_lines:
        line_clean = line.rstrip("\r\n")
        if use_color:
            if line_clean.startswith(("+++", "---")):
                output.append(f"\033[1m{line_clean}\033[0m")
            elif line_clean.startswith("+"):
                output.append(f"\033[32m{line_clean}\033[0m")
            elif line_clean.startswith("-"):
                output.append(f"\033[31m{line_clean}\033[0m")
            elif line_clean.startswith("@"):
                output.append(f"\033[36m{line_clean}\033[0m")
            else:
                output.append(line_clean)
        else:
            output.append(line_clean)

    return "\n".join(output)


def inspect_or_compare_code(
    artifact_path: Path,
    fqcn: str,
    method: str | None = None,
    fmt: str = "smali",
    compare_artifact_path: Path | None = None,
    tool_manager: ToolManager | None = None,
) -> int:
    """Extracts targeted class/method code or performs a unified diff between two artifacts."""
    tm = tool_manager or ToolManager()
    clean_fqcn, _ = normalize_fqcn(fqcn)

    # 1. Standalone inspection mode (no comparison)
    if compare_artifact_path is None:
        inspection = inspect_artifact(artifact_path)
        with materialize_artifact_apks(artifact_path, inspection) as apks:
            location = locate_class(apks, clean_fqcn, allow_missing=False)
            assert location is not None

            if fmt == "smali":
                full_code = extract_smali_class(tm, location)
            else:
                full_code = extract_java_class(tm, location)

            if method:
                target_code = extract_method_from_class(
                    full_code, method, fmt, clean_fqcn
                )
                print(f"Location: {location.split}:{location.dex_name}")
                print(target_code)
            else:
                print(f"Location: {location.split}:{location.dex_name}")
                print(full_code)

        return ExitCode.SUCCESS

    # 2. Comparison mode
    unpatched_path = Path(compare_artifact_path).resolve()
    patched_path = Path(artifact_path).resolve()

    unpatched_inspection = inspect_artifact(unpatched_path)
    patched_inspection = inspect_artifact(patched_path)

    with (
        materialize_artifact_apks(unpatched_path, unpatched_inspection) as old_apks,
        materialize_artifact_apks(patched_path, patched_inspection) as new_apks,
    ):
        old_loc = locate_class(old_apks, clean_fqcn, allow_missing=True)
        new_loc = locate_class(new_apks, clean_fqcn, allow_missing=True)

        if old_loc is None and new_loc is None:
            raise CodeInspectionError(
                f"Class '{clean_fqcn}' not found in either artifact.",
                ExitCode.USAGE_OR_TOOL_ERROR,
            )

        if old_loc is None:
            assert new_loc is not None
            print(
                f"Class '{clean_fqcn}' was added in patched artifact ({new_loc.split}:{new_loc.dex_name})."
            )
            return ExitCode.SUCCESS

        if new_loc is None:
            assert old_loc is not None
            print(
                f"Class '{clean_fqcn}' was removed in patched artifact (was in {old_loc.split}:{old_loc.dex_name})."
            )
            return ExitCode.SUCCESS

        # Both classes exist: extract code
        if fmt == "smali":
            old_code = extract_smali_class(tm, old_loc)
            new_code = extract_smali_class(tm, new_loc)
        else:
            old_code = extract_java_class(tm, old_loc)
            new_code = extract_java_class(tm, new_loc)

        selector_label = f"{clean_fqcn}:{method}" if method else clean_fqcn

        if method:
            # Check method presence on both sides
            old_method: str | None = None
            new_method: str | None = None

            try:
                old_method = extract_method_from_class(
                    old_code, method, fmt, clean_fqcn
                )
            except CodeInspectionError:
                old_method = None

            try:
                new_method = extract_method_from_class(
                    new_code, method, fmt, clean_fqcn
                )
            except CodeInspectionError:
                new_method = None

            if old_method is None and new_method is None:
                raise CodeInspectionError(
                    f"Method '{method}' not found in class '{clean_fqcn}' in either artifact.",
                    ExitCode.USAGE_OR_TOOL_ERROR,
                )

            if old_method is None:
                print(f"Method '{selector_label}' was added in patched artifact.")
                return ExitCode.SUCCESS

            if new_method is None:
                print(f"Method '{selector_label}' was removed in patched artifact.")
                return ExitCode.SUCCESS

            target_old = old_method
            target_new = new_method
        else:
            target_old = old_code
            target_new = new_code

        if target_old == target_new:
            print(f"Unchanged: {selector_label}")
            return ExitCode.SUCCESS

        old_label = f"unpatched/{old_loc.split}:{old_loc.dex_name}:{selector_label}"
        new_label = f"patched/{new_loc.split}:{new_loc.dex_name}:{selector_label}"

        old_lines = target_old.splitlines(keepends=True)
        new_lines = target_new.splitlines(keepends=True)

        diff = list(
            difflib.unified_diff(
                old_lines,
                new_lines,
                fromfile=old_label,
                tofile=new_label,
            )
        )

        if len(diff) > 500:
            diff = diff[:500] + ["\n... diff truncated at 500 lines ...\n"]

        formatted = format_colored_diff(diff)
        print(formatted)

    return ExitCode.SUCCESS


def run_inspect_code(args: argparse.Namespace) -> int:
    """CLI handler for inspect-code subcommand."""
    return inspect_or_compare_code(
        artifact_path=Path(args.artifact),
        fqcn=args.class_name,
        method=args.method,
        fmt=args.format,
        compare_artifact_path=Path(args.compare) if args.compare else None,
    )
