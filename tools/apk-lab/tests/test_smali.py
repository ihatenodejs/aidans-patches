from __future__ import annotations

import subprocess
from unittest.mock import MagicMock

from apk_lab.smali import (
    parse_smali_register,
    validate_smali_snippet,
)
from apk_lab.tools import ToolManager


def test_register_mapping_instance_vs_static():
    # Instance method: locals=20, params=2 -> p0=v20 (this), p1=v21 (arg 1), p2=v22 (arg 2)
    reg_p0, err = parse_smali_register(
        "p0", 1, locals_count=20, parameter_count=2, is_static=False, total_registers=23
    )
    assert err is None
    assert reg_p0.index == 20
    assert reg_p0.is_param is True

    reg_p1, err = parse_smali_register(
        "p1", 1, locals_count=20, parameter_count=2, is_static=False, total_registers=23
    )
    assert err is None
    assert reg_p1.index == 21

    # p3 out of range for params=2
    _, err_p3 = parse_smali_register(
        "p3", 1, locals_count=20, parameter_count=2, is_static=False, total_registers=23
    )
    assert err_p3 is not None
    assert err_p3.code == "PARAMETER_OUT_OF_RANGE"

    # Static method: locals=20, params=2 -> p0=v20 (arg 1), p1=v21 (arg 2)
    reg_stat_p0, err = parse_smali_register(
        "p0", 1, locals_count=20, parameter_count=2, is_static=True, total_registers=22
    )
    assert err is None
    assert reg_stat_p0.index == 20

    reg_stat_p1, err = parse_smali_register(
        "p1", 1, locals_count=20, parameter_count=2, is_static=True, total_registers=22
    )
    assert err is None
    assert reg_stat_p1.index == 21

    # p2 out of range for static params=2
    _, err_stat_p2 = parse_smali_register(
        "p2", 1, locals_count=20, parameter_count=2, is_static=True, total_registers=22
    )
    assert err_stat_p2 is not None
    assert err_stat_p2.code == "PARAMETER_OUT_OF_RANGE"


def test_validate_smali_format_22c_overflow_with_remediation():
    snippet = "instance-of v0, p1, Lcom/example/Target;"
    report = validate_smali_snippet(
        snippet=snippet,
        locals_count=20,
        parameter_count=2,
        is_static=False,
    )
    assert not report.is_valid
    errors = [d for d in report.diagnostics if d.severity == "error"]
    assert len(errors) == 1
    assert errors[0].code == "FORMAT_22C_REGISTER_OVERFLOW"
    assert "p1" in errors[0].message
    assert "v21" in errors[0].message
    assert "move-object/from16" in errors[0].message


def test_validate_smali_format_21c_valid():
    snippet = "check-cast v10, Lcom/example/Target;"
    report = validate_smali_snippet(
        snippet=snippet,
        locals_count=15,
        parameter_count=0,
        is_static=True,
    )
    assert report.is_valid


def test_validate_smali_const4_literal_range():
    # Valid -8..7
    rep_ok = validate_smali_snippet(
        "const/4 v0, 7", locals_count=5, parameter_count=0, is_static=True
    )
    assert rep_ok.is_valid

    rep_ok2 = validate_smali_snippet(
        "const/4 v0, -8", locals_count=5, parameter_count=0, is_static=True
    )
    assert rep_ok2.is_valid

    # Out of range: 8
    rep_err = validate_smali_snippet(
        "const/4 v0, 8", locals_count=5, parameter_count=0, is_static=True
    )
    assert not rep_err.is_valid
    err = rep_err.diagnostics[0]
    assert err.code == "CONST4_LITERAL_OUT_OF_RANGE"
    assert "const/16" in err.message


def test_validate_smali_invoke_35c_overflow():
    snippet = "invoke-virtual {v16}, Lcom/example/Target;->foo()V"
    report = validate_smali_snippet(
        snippet=snippet,
        locals_count=20,
        parameter_count=0,
        is_static=True,
    )
    assert not report.is_valid
    err = report.diagnostics[0]
    assert err.code == "INVOKE_35C_REGISTER_OVERFLOW"
    assert "invoke-virtual/range" in err.message


def test_validate_smali_missing_and_duplicate_labels():
    # Missing label
    rep_missing = validate_smali_snippet(
        "goto :undefined", locals_count=1, parameter_count=0, is_static=True
    )
    assert not rep_missing.is_valid
    assert rep_missing.diagnostics[0].code == "MISSING_BRANCH_TARGET"

    # Duplicate label
    snippet_dup = """:my_label
const/4 v0, 1
:my_label
return-void
"""
    rep_dup = validate_smali_snippet(
        snippet_dup, locals_count=1, parameter_count=0, is_static=True
    )
    assert not rep_dup.is_valid
    assert rep_dup.diagnostics[0].code == "DUPLICATE_LABEL"


def test_validate_smali_cfg_unreachable_and_backward_edge():
    snippet = """goto :end
const/4 v0, 1
:end
if-eqz v0, :end
return-void
"""
    report = validate_smali_snippet(
        snippet, locals_count=2, parameter_count=0, is_static=True
    )
    # Valid because warnings don't make is_valid False
    assert report.is_valid

    warnings = [d for d in report.diagnostics if d.severity == "warning"]
    codes = {w.code for w in warnings}
    assert "UNREACHABLE_CODE" in codes
    assert "BACKWARD_BRANCH" in codes


def test_validate_smali_cfg_closed_cycle_warning():
    snippet = """:loop
add-int/lit8 v0, v0, 1
goto :loop
"""
    report = validate_smali_snippet(
        snippet, locals_count=2, parameter_count=0, is_static=True
    )
    assert report.is_valid
    warnings = [d for d in report.diagnostics if d.severity == "warning"]
    codes = {w.code for w in warnings}
    assert "POSSIBLE_CLOSED_CYCLE" in codes


def test_validate_smali_assembler_oracle_line_adjustment():
    tm = MagicMock(spec=ToolManager)
    tm.is_tool_installed.return_value = True

    # Smali assemble output reporting error on line 6 (header is 4 lines, so snippet line is 2)
    err_out = "Synthetic.smali:[6,4] Invalid instruction 'bad_opcode'\n"
    tm.run_tool_cmd.return_value = subprocess.CompletedProcess([], 1, "", err_out)

    snippet = """const/4 v0, 1
bad_opcode
"""
    report = validate_smali_snippet(
        snippet, locals_count=1, parameter_count=0, is_static=True, tool_manager=tm
    )
    assert not report.is_valid
    err = next(d for d in report.diagnostics if d.code == "ASSEMBLER_SYNTAX_ERROR")
    assert err.line == 2
    assert "bad_opcode" in err.message
