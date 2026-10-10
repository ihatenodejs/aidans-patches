from __future__ import annotations

import argparse
import re
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

from apk_lab.models import ExitCode
from apk_lab.tools import ToolManager


@dataclass(frozen=True)
class Register:
    name: str  # e.g. "v0", "p1"
    index: int  # absolute 0-based register index
    is_param: bool
    param_index: int | None = None


@dataclass(frozen=True)
class SmaliDiagnostic:
    line: int
    severity: Literal["error", "warning"]
    code: str
    message: str


@dataclass
class SmaliValidationReport:
    snippet: str
    locals_count: int
    params_count: int
    is_static: bool
    total_registers: int
    diagnostics: list[SmaliDiagnostic] = field(default_factory=list)

    @property
    def is_valid(self) -> bool:
        return not any(d.severity == "error" for d in self.diagnostics)

    def format_human(self) -> str:
        lines: list[str] = []
        errors = [d for d in self.diagnostics if d.severity == "error"]
        warnings = [d for d in self.diagnostics if d.severity == "warning"]
        if errors:
            lines.append(
                f"Validation failed with {len(errors)} error(s) and {len(warnings)} warning(s):"
            )
        elif warnings:
            lines.append(f"Validation succeeded with {len(warnings)} warning(s):")
        else:
            lines.append("Validation succeeded with 0 errors and 0 warnings.")

        for d in self.diagnostics:
            prefix = "ERROR" if d.severity == "error" else "WARNING"
            lines.append(f"  [{prefix}] Line {d.line} ({d.code}): {d.message}")
        return "\n".join(lines)


# Opcode specifications
# Categories:
# - format_22c: instance-of, iget*, iput* (2 registers, each 4-bit v0..v15)
# - format_21c: check-cast, new-instance, sget*, sput*, const-string, const-class (1 register, 8-bit v0..v255)
# - format_11n: const/4 (1 register 4-bit v0..v15, signed 4-bit literal -8..7)
# - format_21s: const/16 (1 register 8-bit v0..v255, signed 16-bit literal -32768..32767)
# - format_31i: const (1 register 8-bit v0..v255, 32-bit literal)
# - format_12x: move, move-wide, move-object, etc. (2 registers, each 4-bit v0..v15)
# - format_22x: move/from16, move-wide/from16, move-object/from16 (dest 8-bit v0..255, src 16-bit v0..65535)
# - format_32x: move/16, move-wide/16, move-object/16 (dest 16-bit, src 16-bit)
# - format_35c: invoke-virtual, invoke-super, invoke-direct, invoke-static, invoke-interface (list of 4-bit v0..v15)
# - format_3rc: invoke-virtual/range, etc. (range of 16-bit)
# - format_22t: if-eq, if-ne, if-lt, if-ge, if-gt, if-le (2 registers 4-bit v0..v15, branch label)
# - format_21t: if-eqz, if-nez, if-ltz, if-gez, if-gtz, if-lez (1 register 8-bit v0..v255, branch label)
# - format_10t_20t_30t: goto, goto/16, goto/32 (branch label)
# - returns: return-void, return, return-wide, return-object, throw

FORMAT_22C_OPCODES = {
    "instance-of",
    "iget",
    "iget-wide",
    "iget-object",
    "iget-boolean",
    "iget-byte",
    "iget-char",
    "iget-short",
    "iput",
    "iput-wide",
    "iput-object",
    "iput-boolean",
    "iput-byte",
    "iput-char",
    "iput-short",
}

FORMAT_21C_OPCODES = {
    "check-cast",
    "new-instance",
    "const-string",
    "const-string/jumbo",
    "const-class",
    "sget",
    "sget-wide",
    "sget-object",
    "sget-boolean",
    "sget-byte",
    "sget-char",
    "sget-short",
    "sput",
    "sput-wide",
    "sput-object",
    "sput-boolean",
    "sput-byte",
    "sput-char",
    "sput-short",
}

INVOKE_35C_OPCODES = {
    "invoke-virtual",
    "invoke-super",
    "invoke-direct",
    "invoke-static",
    "invoke-interface",
}

INVOKE_3RC_OPCODES = {
    "invoke-virtual/range",
    "invoke-super/range",
    "invoke-direct/range",
    "invoke-static/range",
    "invoke-interface/range",
}

BRANCH_22T_OPCODES = {
    "if-eq",
    "if-ne",
    "if-lt",
    "if-ge",
    "if-gt",
    "if-le",
}

BRANCH_21T_OPCODES = {
    "if-eqz",
    "if-nez",
    "if-ltz",
    "if-gez",
    "if-gtz",
    "if-lez",
}

GOTO_OPCODES = {"goto", "goto/16", "goto/32"}

RETURN_THROW_OPCODES = {
    "return-void",
    "return",
    "return-wide",
    "return-object",
    "throw",
}


def parse_smali_register(
    token: str,
    line_no: int,
    locals_count: int,
    parameter_count: int,
    is_static: bool,
    total_registers: int,
) -> tuple[Register | None, SmaliDiagnostic | None]:
    """Parses a register token (e.g. 'v0', 'p1') and maps it to absolute register index."""
    tok = token.strip().rstrip(",")
    if not tok:
        return None, None

    if tok.startswith("v"):
        try:
            num = int(tok[1:])
        except ValueError:
            return None, SmaliDiagnostic(
                line=line_no,
                severity="error",
                code="INVALID_REGISTER_SYNTAX",
                message=f"Invalid register syntax '{tok}'",
            )
        if num < 0 or num >= total_registers:
            return None, SmaliDiagnostic(
                line=line_no,
                severity="error",
                code="REGISTER_OUT_OF_RANGE",
                message=f"Register '{tok}' out of range for total {total_registers} registers (v0..v{total_registers - 1})",
            )
        return Register(name=tok, index=num, is_param=False), None

    if tok.startswith("p"):
        try:
            p_idx = int(tok[1:])
        except ValueError:
            return None, SmaliDiagnostic(
                line=line_no,
                severity="error",
                code="INVALID_REGISTER_SYNTAX",
                message=f"Invalid register syntax '{tok}'",
            )

        if is_static:
            if p_idx < 0 or p_idx >= parameter_count:
                return None, SmaliDiagnostic(
                    line=line_no,
                    severity="error",
                    code="PARAMETER_OUT_OF_RANGE",
                    message=f"Parameter register '{tok}' out of range for static method with {parameter_count} parameter(s) (p0..p{max(0, parameter_count - 1)})",
                )
            abs_idx = locals_count + p_idx
            return Register(
                name=tok, index=abs_idx, is_param=True, param_index=p_idx
            ), None
        else:
            # instance method: p0 is this
            max_p = parameter_count
            if p_idx < 0 or p_idx > max_p:
                return None, SmaliDiagnostic(
                    line=line_no,
                    severity="error",
                    code="PARAMETER_OUT_OF_RANGE",
                    message=f"Parameter register '{tok}' out of range for instance method with {parameter_count} parameter(s) (p0=this, p1..p{max_p})",
                )
            abs_idx = locals_count + p_idx
            return Register(
                name=tok, index=abs_idx, is_param=True, param_index=p_idx
            ), None

    return None, None


def parse_literal_int(tok: str) -> int:
    """Parses numeric literal in decimal or hex (e.g. 10, -5, 0x10, -0x8)."""
    clean = tok.strip().rstrip(",")
    if clean.startswith("#"):
        clean = clean[1:].strip()
    return int(clean, 0)


def validate_smali_snippet(
    snippet: str,
    locals_count: int,
    parameter_count: int,
    is_static: bool = False,
    tool_manager: ToolManager | None = None,
) -> SmaliValidationReport:
    """Performs static syntax, opcode constraint, CFG, and assembler validation on Smali snippet."""
    if locals_count < 0:
        raise ValueError("locals_count must be non-negative")
    if parameter_count < 0:
        raise ValueError("parameter_count must be non-negative")

    total_regs = locals_count + parameter_count + (0 if is_static else 1)
    report = SmaliValidationReport(
        snippet=snippet,
        locals_count=locals_count,
        params_count=parameter_count,
        is_static=is_static,
        total_registers=total_regs,
    )

    lines = snippet.splitlines()

    # Pre-pass: collect labels and directives
    label_lines: dict[str, int] = {}
    instructions: list[tuple[int, str, list[str]]] = []

    for line_idx, raw_line in enumerate(lines, start=1):
        line = raw_line.strip()
        # Strip comments
        if "#" in line:
            line = line.split("#", 1)[0].strip()
        if not line:
            continue

        # Check for disallowed method/class directives
        if line.startswith((".class", ".super", ".method", ".end method")):
            report.diagnostics.append(
                SmaliDiagnostic(
                    line=line_idx,
                    severity="error",
                    code="METHOD_DIRECTIVE_DISALLOWED",
                    message="Method or class directives are not allowed in snippets",
                )
            )
            continue

        if line.startswith(":"):
            label_name = line.split()[0]
            if label_name in label_lines:
                report.diagnostics.append(
                    SmaliDiagnostic(
                        line=line_idx,
                        severity="error",
                        code="DUPLICATE_LABEL",
                        message=f"Duplicate label definition '{label_name}' (previously defined on line {label_lines[label_name]})",
                    )
                )
            else:
                label_lines[label_name] = line_idx
            continue

        # Split instruction into opcode and operand tokens
        parts = line.split(None, 1)
        opcode = parts[0]
        operands_str = parts[1] if len(parts) > 1 else ""

        # Parse operand tokens
        tokens = [t.strip() for t in operands_str.split(",") if t.strip()]
        instructions.append((line_idx, opcode, tokens))

    # Pass 1: Opcode and Register constraints
    for line_idx, opcode, tokens in instructions:
        # 1. Format 22c: instance-of, iget*, iput*
        if opcode in FORMAT_22C_OPCODES:
            if len(tokens) >= 2:
                for tok in tokens[:2]:
                    reg, diag = parse_smali_register(
                        tok,
                        line_idx,
                        locals_count,
                        parameter_count,
                        is_static,
                        total_regs,
                    )
                    if diag:
                        report.diagnostics.append(diag)
                    elif reg and reg.index > 15:
                        remediation = (
                            f"move-object/from16 v1, {tok}"
                            if "object" in opcode or opcode == "instance-of"
                            else f"move/from16 v1, {tok}"
                        )
                        report.diagnostics.append(
                            SmaliDiagnostic(
                                line=line_idx,
                                severity="error",
                                code="FORMAT_22C_REGISTER_OVERFLOW",
                                message=(
                                    f"Opcode '{opcode}' (format 22c) requires 4-bit registers (v0..v15). "
                                    f"'{tok}' maps to v{reg.index}. Use '{remediation}' first."
                                ),
                            )
                        )

        # 2. Format 21c: check-cast, new-instance, etc.
        elif opcode in FORMAT_21C_OPCODES:
            if tokens:
                reg, diag = parse_smali_register(
                    tokens[0],
                    line_idx,
                    locals_count,
                    parameter_count,
                    is_static,
                    total_regs,
                )
                if diag:
                    report.diagnostics.append(diag)
                elif reg and reg.index > 255:
                    report.diagnostics.append(
                        SmaliDiagnostic(
                            line=line_idx,
                            severity="error",
                            code="FORMAT_21C_REGISTER_OVERFLOW",
                            message=(
                                f"Opcode '{opcode}' (format 21c) destination register '{tokens[0]}' "
                                f"maps to v{reg.index}, which exceeds 8-bit maximum (v0..v255)."
                            ),
                        )
                    )

        # 3. Format 11n: const/4
        elif opcode == "const/4":
            if len(tokens) >= 2:
                reg, diag = parse_smali_register(
                    tokens[0],
                    line_idx,
                    locals_count,
                    parameter_count,
                    is_static,
                    total_regs,
                )
                if diag:
                    report.diagnostics.append(diag)
                elif reg and reg.index > 15:
                    report.diagnostics.append(
                        SmaliDiagnostic(
                            line=line_idx,
                            severity="error",
                            code="FORMAT_11N_REGISTER_OVERFLOW",
                            message=f"Opcode 'const/4' requires 4-bit register (v0..v15). '{tokens[0]}' maps to v{reg.index}.",
                        )
                    )
                try:
                    val = parse_literal_int(tokens[1])
                    if not (-8 <= val <= 7):
                        report.diagnostics.append(
                            SmaliDiagnostic(
                                line=line_idx,
                                severity="error",
                                code="CONST4_LITERAL_OUT_OF_RANGE",
                                message=f"Literal '{tokens[1]}' out of range for 'const/4' (-8..7). Use 'const/16' instead.",
                            )
                        )
                except ValueError:
                    pass

        # 4. Format 21s: const/16
        elif opcode == "const/16":
            if len(tokens) >= 2:
                reg, diag = parse_smali_register(
                    tokens[0],
                    line_idx,
                    locals_count,
                    parameter_count,
                    is_static,
                    total_regs,
                )
                if diag:
                    report.diagnostics.append(diag)
                elif reg and reg.index > 255:
                    report.diagnostics.append(
                        SmaliDiagnostic(
                            line=line_idx,
                            severity="error",
                            code="FORMAT_21S_REGISTER_OVERFLOW",
                            message=f"Opcode 'const/16' destination register '{tokens[0]}' maps to v{reg.index}, which exceeds 8-bit limit (v0..v255).",
                        )
                    )
                try:
                    val = parse_literal_int(tokens[1])
                    if not (-32768 <= val <= 32767):
                        report.diagnostics.append(
                            SmaliDiagnostic(
                                line=line_idx,
                                severity="error",
                                code="CONST16_LITERAL_OUT_OF_RANGE",
                                message=f"Literal '{tokens[1]}' out of range for 'const/16' (-32768..32767). Use 'const' instead.",
                            )
                        )
                except ValueError:
                    pass

        # 5. Format 35c: invoke-*
        elif opcode in INVOKE_35C_OPCODES:
            # tokens[0] is typically {vA, vB, ...}
            raw_regs_match = re.search(r"\{([^}]*)\}", " ".join(tokens))
            if raw_regs_match:
                reg_tokens = [
                    r.strip() for r in raw_regs_match.group(1).split(",") if r.strip()
                ]
                for r_tok in reg_tokens:
                    reg, diag = parse_smali_register(
                        r_tok,
                        line_idx,
                        locals_count,
                        parameter_count,
                        is_static,
                        total_regs,
                    )
                    if diag:
                        report.diagnostics.append(diag)
                    elif reg and reg.index > 15:
                        report.diagnostics.append(
                            SmaliDiagnostic(
                                line=line_idx,
                                severity="error",
                                code="INVOKE_35C_REGISTER_OVERFLOW",
                                message=(
                                    f"Opcode '{opcode}' (format 35c) requires 4-bit registers (v0..v15). "
                                    f"'{r_tok}' maps to v{reg.index}. Use '{opcode}/range' for registers above v15."
                                ),
                            )
                        )

        # 6. Branch instructions target checks
        if opcode in GOTO_OPCODES:
            target = tokens[0] if tokens else ""
            if not target.startswith(":"):
                report.diagnostics.append(
                    SmaliDiagnostic(
                        line=line_idx,
                        severity="error",
                        code="INVALID_BRANCH_SYNTAX",
                        message=f"Goto instruction requires label target starting with ':', got '{target}'",
                    )
                )
            elif target not in label_lines:
                report.diagnostics.append(
                    SmaliDiagnostic(
                        line=line_idx,
                        severity="error",
                        code="MISSING_BRANCH_TARGET",
                        message=f"Target label '{target}' not defined in snippet",
                    )
                )

        elif opcode in BRANCH_22T_OPCODES or opcode in BRANCH_21T_OPCODES:
            target = tokens[-1] if tokens else ""
            if not target.startswith(":"):
                report.diagnostics.append(
                    SmaliDiagnostic(
                        line=line_idx,
                        severity="error",
                        code="INVALID_BRANCH_SYNTAX",
                        message=f"Conditional branch requires label target starting with ':', got '{target}'",
                    )
                )
            elif target not in label_lines:
                report.diagnostics.append(
                    SmaliDiagnostic(
                        line=line_idx,
                        severity="error",
                        code="MISSING_BRANCH_TARGET",
                        message=f"Target label '{target}' not defined in snippet",
                    )
                )

    # Pass 2: Control Flow Graph (CFG) analysis
    # Map instruction indices to lines
    cfg_nodes: list[int] = [i[0] for i in instructions]
    if cfg_nodes:
        # Build adjacency list: node_line -> list of successor lines
        adj: dict[int, list[int]] = {ln: [] for ln in cfg_nodes}
        inst_by_line = {i[0]: (i[1], i[2]) for i in instructions}

        # Associate each label with the line of the instruction that immediately follows it
        label_to_inst_line: dict[str, int] = {}
        for lbl, lbl_line in label_lines.items():
            following = [ln for ln in cfg_nodes if ln > lbl_line]
            if following:
                label_to_inst_line[lbl] = following[0]

        for idx, ln in enumerate(cfg_nodes):
            opcode, tokens = inst_by_line[ln]
            has_next = idx + 1 < len(cfg_nodes)
            next_line = cfg_nodes[idx + 1] if has_next else None

            if opcode in GOTO_OPCODES:
                target_lbl = tokens[0] if tokens else ""
                if target_lbl in label_to_inst_line:
                    adj[ln].append(label_to_inst_line[target_lbl])

            elif opcode in BRANCH_22T_OPCODES or opcode in BRANCH_21T_OPCODES:
                target_lbl = tokens[-1] if tokens else ""
                if target_lbl in label_to_inst_line:
                    adj[ln].append(label_to_inst_line[target_lbl])
                if next_line:
                    adj[ln].append(next_line)

            elif opcode in RETURN_THROW_OPCODES:
                # Terminal instruction: does not fall through
                pass
            else:
                # Normal instruction: falls through to next line
                if next_line:
                    adj[ln].append(next_line)

        # Reachability analysis from entry (cfg_nodes[0])
        entry = cfg_nodes[0]
        reachable: set[int] = set()
        queue = [entry]
        while queue:
            curr = queue.pop()
            if curr not in reachable:
                reachable.add(curr)
                for succ in adj.get(curr, []):
                    if succ not in reachable:
                        queue.append(succ)

        for ln in cfg_nodes:
            if ln not in reachable:
                report.diagnostics.append(
                    SmaliDiagnostic(
                        line=ln,
                        severity="warning",
                        code="UNREACHABLE_CODE",
                        message=f"Unreachable instruction '{inst_by_line[ln][0]}' at line {ln}",
                    )
                )

        # Backward edges and closed cycle detection
        # A backward edge exists if an edge goes from a node to an earlier node in line order
        for u in cfg_nodes:
            if u in reachable:
                for v in adj.get(u, []):
                    if v <= u:
                        matched_lbls = [
                            lbl
                            for lbl, l_line in label_to_inst_line.items()
                            if l_line == v
                        ]
                        lbl_name = matched_lbls[0] if matched_lbls else f"line {v}"
                        report.diagnostics.append(
                            SmaliDiagnostic(
                                line=u,
                                severity="warning",
                                code="BACKWARD_BRANCH",
                                message=f"Backward branch to '{lbl_name}' detected (loop)",
                            )
                        )

        # Strongly connected components (Tarjan's algorithm) to find closed cycles
        index = 0
        indices: dict[int, int] = {}
        lowlinks: dict[int, int] = {}
        on_stack: dict[int, bool] = {}
        stack: list[int] = []
        sccs: list[list[int]] = []

        def strongconnect(v_node: int) -> None:
            nonlocal index
            indices[v_node] = index
            lowlinks[v_node] = index
            index += 1
            stack.append(v_node)
            on_stack[v_node] = True

            for w in adj.get(v_node, []):
                if w not in indices:
                    strongconnect(w)
                    lowlinks[v_node] = min(lowlinks[v_node], lowlinks[w])
                elif on_stack.get(w, False):
                    lowlinks[v_node] = min(lowlinks[v_node], indices[w])

            if lowlinks[v_node] == indices[v_node]:
                scc: list[int] = []
                while True:
                    w = stack.pop()
                    on_stack[w] = False
                    scc.append(w)
                    if w == v_node:
                        break
                if len(scc) > 1 or (len(scc) == 1 and v_node in adj.get(v_node, [])):
                    sccs.append(scc)

        for node in reachable:
            if node not in indices:
                strongconnect(node)

        for scc in sccs:
            # Check if SCC has any outgoing edge outside SCC
            has_exit = False
            has_return = False
            for member in scc:
                op, _ = inst_by_line[member]
                if op in RETURN_THROW_OPCODES:
                    has_return = True
                for nxt in adj.get(member, []):
                    if nxt not in scc:
                        has_exit = True
            if not has_exit and not has_return:
                min_line = min(scc)
                report.diagnostics.append(
                    SmaliDiagnostic(
                        line=min_line,
                        severity="warning",
                        code="POSSIBLE_CLOSED_CYCLE",
                        message="Possible closed loop detected: cycle has no exit or return/throw",
                    )
                )

    # Pass 3: Assembler Oracle (invoke pinned smali assemble)
    # Only if report has no errors so far
    tm = tool_manager or ToolManager()
    if report.is_valid and tm.is_tool_installed("smali"):
        with tempfile.TemporaryDirectory(prefix="apk-lab-smali-val-") as tmp_dir:
            tmp_path = Path(tmp_dir)
            synth_smali = tmp_path / "Synthetic.smali"
            out_dex = tmp_path / "classes.dex"

            # Construct synthetic class wrapper
            param_types = "".join(["Ljava/lang/Object;"] * parameter_count)
            method_mods = "public static" if is_static else "public"
            synth_lines = [
                ".class public LSynthetic;",
                ".super Ljava/lang/Object;",
                f".method {method_mods} testMethod({param_types})V",
                f"    .locals {locals_count}",
            ]
            header_line_count = len(synth_lines)
            synth_lines.append(snippet)
            synth_lines.append("    return-void")
            synth_lines.append(".end method")

            synth_smali.write_text("\n".join(synth_lines), encoding="utf-8")

            proc = tm.run_tool_cmd(
                "smali",
                ["assemble", str(synth_smali), "-o", str(out_dex)],
                capture_output=True,
                check=False,
            )
            if proc.returncode != 0:
                err_text = f"{proc.stderr}\n{proc.stdout}"
                for err_line in err_text.splitlines():
                    match = re.search(r"\[(\d+),\d+\]\s*(.*)", err_line)
                    if match:
                        raw_line_no = int(match.group(1))
                        adjusted_line = max(1, raw_line_no - header_line_count)
                        msg = match.group(2)
                        report.diagnostics.append(
                            SmaliDiagnostic(
                                line=adjusted_line,
                                severity="error",
                                code="ASSEMBLER_SYNTAX_ERROR",
                                message=msg,
                            )
                        )

    return report


def run_validate_smali(args: argparse.Namespace) -> int:
    """CLI handler for validate-smali subcommand."""
    report = validate_smali_snippet(
        snippet=args.snippet,
        locals_count=args.locals,
        parameter_count=args.params,
        is_static=args.is_static,
    )
    print(report.format_human())
    return ExitCode.SUCCESS if report.is_valid else ExitCode.USAGE_OR_TOOL_ERROR
