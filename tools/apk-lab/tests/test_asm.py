from __future__ import annotations

import pytest
from apk_lab.asm import (
    MAX_BRANCH_DELTA,
    MIN_BRANCH_DELTA,
    assemble_statement,
    encode_branch,
    encode_mov,
    encode_ret,
    format_instruction,
    format_kotlin_byte_array,
)
from apk_lab.cli import build_parser, handle_asm
from apk_lab.models import ExitCode


def test_encode_branch_forward_and_backward():
    # Forward branch: 0x1000 -> 0x2000 (delta = 0x1000 = 4096 bytes)
    # imm26 = 4096 >> 2 = 1024 = 0x400
    # b: 0x14000400, bl: 0x94000400
    b_fwd = encode_branch("b", 0x1000, 0x2000)
    assert b_fwd == 0x14000400
    bl_fwd = encode_branch("bl", 0x1000, 0x2000)
    assert bl_fwd == 0x94000400

    # Backward branch: 0x2000 -> 0x1000 (delta = -4096 bytes)
    # imm26 = (-4096 >> 2) & 0x03ffffff = (-1024) & 0x03ffffff = 0x03fffffc00? Wait:
    # -1024 in 26-bit: 0x03fffc00
    b_bwd = encode_branch("b", 0x2000, 0x1000)
    assert b_bwd == (0x14000000 | ((-1024) & 0x03FFFFFF))
    assert encode_branch("bl", 0x2000, 0x1000) == (0x94000000 | ((-1024) & 0x03FFFFFF))

    # Large forward branch near +128 MB (MAX_BRANCH_DELTA = 134217724)
    b_max = encode_branch("b", 0, MAX_BRANCH_DELTA)
    assert b_max == (0x14000000 | ((MAX_BRANCH_DELTA >> 2) & 0x03FFFFFF))

    # Large backward branch near -128 MB (MIN_BRANCH_DELTA = -134217728)
    b_min = encode_branch("b", 0x10000000, 0x10000000 + MIN_BRANCH_DELTA)
    assert b_min == (0x14000000 | ((MIN_BRANCH_DELTA >> 2) & 0x03FFFFFF))

    # Blackjack known verified opcode: bl 0x3c98ce4 from 0x1fcf6e8
    assert encode_branch("bl", 0x1FCF6E8, 0x3C98CE4) == 0x9473257F
    assert format_instruction(0x9473257F, "hex") == "7f 25 73 94"


def test_encode_branch_out_of_range():
    # Delta exceeds MAX_BRANCH_DELTA
    with pytest.raises(ValueError, match="exceeds ARM64 26-bit range"):
        encode_branch("b", 0, MAX_BRANCH_DELTA + 4)

    # Delta below MIN_BRANCH_DELTA
    with pytest.raises(ValueError, match="exceeds ARM64 26-bit range"):
        encode_branch("b", 0x10000000, 0x10000000 + MIN_BRANCH_DELTA - 4)


def test_encode_branch_misaligned():
    with pytest.raises(ValueError, match="Program counter pc .* must be 4-byte aligned"):
        encode_branch("b", 1, 0x1000)
    with pytest.raises(ValueError, match="Target address .* must be 4-byte aligned"):
        encode_branch("b", 0x1000, 3)


def test_encode_ret_and_mov():
    # ret default (x30 / lr)
    assert encode_ret() == 0xD65F03C0
    assert encode_ret("lr") == 0xD65F03C0
    assert encode_ret("x30") == 0xD65F03C0
    # ret x0
    assert encode_ret("x0") == 0xD65F0000

    # mov x0, xzr -> 0xaa1f03e0
    assert encode_mov("x0", "xzr") == 0xAA1F03E0
    # mov w0, wzr -> 0x2a1f03e0
    assert encode_mov("w0", "wzr") == 0x2A1F03E0

    # mov w1, #1 -> 0x52800000 | (1 << 5) | 1 = 0x52800021
    assert encode_mov("w1", "#1") == 0x52800021
    assert encode_mov("w1", 1) == 0x52800021

    # mov x2, #0x10
    assert encode_mov("x2", "#0x10") == (0xD2800000 | (0x10 << 5) | 2)

    # assemble_statement
    assert assemble_statement("ret") == 0xD65F03C0
    assert assemble_statement("mov x0, xzr") == 0xAA1F03E0
    assert assemble_statement("mov w1, #1") == 0x52800021


def test_format_kotlin_byte_array():
    # Boundary byte values: 0x00, 0x7f (no toByte), 0x80, 0xff (with toByte)
    data = bytes([0x00, 0x7F, 0x80, 0xFF])
    res = format_kotlin_byte_array(data)
    assert res == "byteArrayOf(0x00, 0x7f, 0x80.toByte(), 0xff.toByte())"

    # Known ret instruction: c0 03 5f d6
    ret_bytes = 0xD65F03C0.to_bytes(4, "little")
    assert format_kotlin_byte_array(ret_bytes) == "byteArrayOf(0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())"


def test_cli_asm_integration(capsys):
    parser = build_parser()

    # bl forward with hex output
    args = parser.parse_args(["asm", "bl 0x3c98ce4", "--pc", "0x1fcf6e8"])
    code = handle_asm(args)
    assert code == ExitCode.SUCCESS
    out = capsys.readouterr().out.strip()
    assert out == "7f 25 73 94"

    # ret with kotlin output
    args = parser.parse_args(["asm", "ret", "--format", "kotlin"])
    code = handle_asm(args)
    assert code == ExitCode.SUCCESS
    out = capsys.readouterr().out.strip()
    assert out == "byteArrayOf(0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())"

    # invalid instruction
    args = parser.parse_args(["asm", "invalid_instruction_xyz"])
    code = handle_asm(args)
    assert code == ExitCode.USAGE_OR_TOOL_ERROR
