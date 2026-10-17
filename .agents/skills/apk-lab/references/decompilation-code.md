# Decompilation, Bytecode Inspection & Smali Validation

Extract APK artifacts into managed workspaces, perform surgical single-class/single-method disassembly and Java decompilation, compare bytecode diffs, and statically validate Smali snippets before bytecode injection.

---

## 1. `analyze` — Managed Extraction & Decompilation

Unpacks an APK or split container (APKM, XAPK, APKS) into an isolated, reproducible run directory under `.apk-lab/`, disassembles bytecode, extracts native `.so` libraries, and optionally decompiles Java or resources.

### Capabilities
- **Native Library Extraction**: Extracts `.so` shared libraries from the base APK and all architecture splits into `<workspace>/lib/<arch>/` (e.g. `lib/arm64-v8a/libil2cpp.so`). Conflicting identical libraries are deduplicated; conflicting differing libraries raise an error.
- **Bytecode Disassembly**: Uses pinned baksmali to disassemble all `classes*.dex` into Smali.
- **Java Decompilation**: Optional JADX decompilation with class filtering.
- **Apktool Resource Decoding**: Optional resource decoding with Apktool.
- **Reproducibility**: Writes `.marker.json` with tool versions, input hashes, and command configuration digest.

### CLI Invocation

```bash
# Default: metadata extraction + Smali disassembly + native libs
uv run --project tools/apk-lab apk-lab analyze path/to/app.apkm

# Include JADX Java decompilation
uv run --project tools/apk-lab apk-lab analyze path/to/app.apkm --jadx

# Decompile specific classes only with JADX
uv run --project tools/apk-lab apk-lab analyze path/to/app.apkm --jadx --class com.example.app.MainActivity

# Include Apktool resource decoding
uv run --project tools/apk-lab apk-lab analyze path/to/app.apkm --apktool
```

### CLI Arguments
- `artifact` (required positional): Path to APK or split container.
- `--jadx`: Enable JADX Java decompilation.
- `--apktool`: Enable Apktool resource decoding.
- `--smali`: Force Smali disassembly (enabled by default unless only JADX/Apktool requested).
- `--class CLASS ...`: Specific class patterns for JADX decompilation.

### Exit Codes
- `0`: Analysis succeeded. Prints workspace path and matching cleanup command.
- `2`: Usage error or invalid arguments.
- `3`: Invalid artifact or security error.
- `4`: Tool or infrastructure failure.
---

## 2. `inspect-code` — Surgical Class & Method Inspection

Extracts or decompiles **only a single class or method** directly from an APK without decompiling the entire multi-gigabyte application, and provides side-by-side unified diffs between unpatched and patched APKs.

### Localization & Extraction Mechanism
1. **Multi-DEX Search**: Scans `classes*.dex` across all splits using `dexdump` on each extracted DEX to find the exact split and DEX file containing the class.
2. **Smali Disassembly**: Invokes pinned baksmali with `--classes <descriptor> --code-offsets` on the containing DEX.
3. **Java Decompilation**: Invokes pinned JADX with `--single-class <fqcn> --show-bad-code --no-res` directly on the containing APK.
4. **Method Slicing**:
   - Smali: Slices `.method` through `.end method`.
   - Java: Uses a lexical brace-balancing parser (skipping strings and comments) to extract the depth-1 method body.
5. **Diffing**: Unified diff against an unpatched artifact, truncated at 500 lines if large.

### CLI Invocation

```bash
# Disassemble a specific class to Smali
uv run --project tools/apk-lab apk-lab inspect-code path/to/app.apkm \
  --class com.example.app.MainActivity \
  --format smali

# Extract a specific method by name
uv run --project tools/apk-lab apk-lab inspect-code path/to/app.apkm \
  --class com.example.app.MainActivity \
  --method onCreate \
  --format smali

# Extract a specific method overload using Dalvik descriptor (Smali format only!)
uv run --project tools/apk-lab apk-lab inspect-code path/to/app.apkm \
  --class com.example.app.MainActivity \
  --method "onCreate(Landroid/os/Bundle;)V" \
  --format smali

# Decompile a method to Java and compare against unpatched APK
uv run --project tools/apk-lab apk-lab inspect-code patched.apk \
  --class com.example.app.MainActivity \
  --method onCreate \
  --format java \
  --compare unpatched.apk
```

### CLI Arguments
- `artifact` (required positional): Path to APK/APKM/XAPK/APKS.
- `--class CLASS` (required): Fully-qualified class name (e.g. `com.example.app.MainActivity`).
- `--method METHOD`: Method name or Dalvik descriptor.
  - **Rule**: Dalvik descriptors (e.g. `foo(I)V`) are **only valid with `--format smali`**. Passing descriptors with `--format java` fails fast with exit code `2`.
- `--format {smali,java}`: Extraction format (default: `smali`).
- `--compare ARTIFACT`: Baseline APK to produce a unified diff against.

### Exit Codes
- `0`: Inspection or comparison succeeded.
- `2`: Class not found, ambiguous method overload, invalid selector, or tool error.

---

## 3. `validate-smali` — Pre-Injection Register & CFG Validator

Statically validates Smali instruction snippets before injecting them into Morphe Kotlin DSL `addInstructions` / `replaceInstruction` blocks.

### Validation Checks
1. **Register Mapping**: Maps `p0..pN` registers to absolute indices (`v<locals + idx>`).
   - Instance methods: `p0` is `this`, `p1..pN` are parameters.
   - Static methods (`--is-static`): `p0..p(N-1)` are parameters.
2. **Opcode Register Widths**:
   - **Format 22c** (`instance-of`, `iget`, `iput`): 4-bit operands (`v0..v15`). Register $> 15$ triggers an error with remediation guidance (e.g. use `move-object/from16`).
   - **Format 21c** (`check-cast`, `new-instance`): 8-bit destination (`v0..v255`).
   - **Format 11n** (`const/4`): signed 4-bit literal range (`-8..7`).
   - **Format 21s** (`const/16`): signed 16-bit literal range (`-32768..32767`).
   - **Format 35c** (`invoke-*`): 4-bit register list (`v0..v15`). Registers $> 15$ trigger an error suggesting `/range` variants.
3. **CFG Analysis**:
   - Label validation: Missing branch targets or duplicate labels.
   - Warnings: `BACKWARD_BRANCH` (loops), `UNREACHABLE_CODE` (instructions after unconditional returns), and `POSSIBLE_CLOSED_CYCLE` (infinite loops without exit).
4. **Assembler Oracle**:
   - Wraps the snippet into a synthetic class and invokes pinned `smali assemble` to verify syntactical acceptance by the Dalvik assembler.

### CLI Invocation

```bash
# Validate instruction register widths and ranges
uv run --project tools/apk-lab apk-lab validate-smali \
  --snippet "instance-of v0, p1, Lcom/example/Target;" \
  --locals 20 \
  --params 2

# Validate a static method snippet with a branch
uv run --project tools/apk-lab apk-lab validate-smali \
  --snippet "const/4 v0, 1\n:loop\nadd-int/lit8 v0, v0, 1\ngoto :loop" \
  --locals 2 \
  --params 0 \
  --is-static
```

### CLI Arguments
- `--snippet TEXT` (required): Raw Smali instructions (newline-separated).
- `--locals N` (required): `.locals` count of target method.
- `--params N`: Number of declared single-width parameters (assumes 1 register per param; use `--descriptor` for wide types; defaults to 0 if omitted).
- `--descriptor DESC`: Method descriptor (e.g. `'(IJLjava/lang/String;)V'`) defining exact parameter types and 64-bit wide register widths.
- `--is-static`: Set if target method is static (does not have implicit `p0 = this`).
### Exit Codes
- `0`: Snippet passed all checks without errors.
- `2`: Validation error found (register overflow, invalid literal, malformed branch, or assembler failure).

---

## 4. Python API Reference

```python
from pathlib import Path
from apk_lab.code_inspection import (
    locate_class,
    extract_smali_class,
    extract_java_class,
    extract_method_from_class,
    inspect_or_compare_code,
)
from apk_lab.smali import validate_smali_snippet, SmaliValidationReport
from apk_lab.archives import materialize_artifact_apks
from apk_lab.inspection import inspect_artifact
from apk_lab.tools import ToolManager

tool_mgr = ToolManager()

# Surgical class localization and extraction
artifact_path = Path("path/to/app.apkm")
inspection = inspect_artifact(artifact_path)
with materialize_artifact_apks(artifact_path, inspection) as mat_apks:
    loc = locate_class(mat_apks, "com.example.app.MainActivity")
    if loc:
        smali_code = extract_smali_class(tool_mgr, loc)
        method_body = extract_method_from_class(smali_code, "onCreate", "smali", loc.descriptor)
        print(method_body)

# Validate Smali snippet
report = validate_smali_snippet(
    snippet="instance-of v0, p1, Lcom/example/Target;",
    locals_count=20,
    parameter_count=2,
    is_static=False,
    tool_manager=tool_mgr,
)

print(f"Is valid: {report.is_valid}")
for diag in report.diagnostics:
    print(f"[{diag.severity.upper()}] Line {diag.line}: {diag.code} - {diag.message}")
```
