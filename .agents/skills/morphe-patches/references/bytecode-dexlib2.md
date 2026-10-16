# Dexlib2 Bytecode Manipulation & Smali Reference

This reference covers navigating Dexlib2 AST structures, locating classes and methods, using standard bytecode neutralizers, injecting Smali instructions, observing Dalvik register width limits, and validating control-flow integrity.

---

## 1. Dexlib2 AST Navigation

Inside a `bytecodePatch` or `compositeFeaturePatch`'s `execute { ... }` block, the receiver is `BytecodePatchContext`.

### Class Discovery
Always prefer safe resolution with explicit error throwing over unsafe operations:

```kotlin
// Safe resolution with diagnostic error
val classDef = mutableClassDefByOrNull("Lcom/example/analytics/Tracker;")
    ?: throw PatchException("Failed to find class Lcom/example/analytics/Tracker;")

// Optional resolution for version-dependent classes
val optionalDef = mutableClassDefByOrNull("Lcom/example/beta/FeatureFlag;")
if (optionalDef == null) {
    // Graceful continuation when explicitly version-optional
    return@execute
}
```

### Method Discovery & Anchoring
Avoid matching on broad names alone (e.g. `init` or `onClick`). Anchor methods using:
1. Exact method name.
2. Parameter types (list of descriptors).
3. Return type descriptor (`"V"` for void, `"Z"` for boolean, `"I"` for int, `"Ljava/lang/String;"`, etc.).
4. Presence of concrete implementation (`implementation != null`).
5. Distinctive instruction reference (string literal, called method, field access).

```kotlin
val targetMethod = classDef.methods.firstOrNull { method ->
    method.name == "onCreate" &&
        method.parameterTypes == listOf("Landroid/os/Bundle;") &&
        method.returnType == "V" &&
        method.implementation != null
} ?: throw PatchException("Could not find onCreate(Bundle)V in ${classDef.type}")
```

### Locating Instruction Anchors
Inspect `implementation.instructions` to anchor edits to specific opcodes or references:

```kotlin
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.StringReference
import com.android.tools.smali.dexlib2.iface.reference.MethodReference

val instructions = targetMethod.implementation!!.instructions.toList()

// Find instruction by string reference
val anchorIndex = instructions.indexOfFirst { inst ->
    inst.opcode == Opcode.CONST_STRING &&
        (inst as? ReferenceInstruction)?.reference?.let { ref ->
            (ref as? StringReference)?.string == "enable_tracking"
        } == true
}
if (anchorIndex == -1) {
    throw PatchException("Anchor string 'enable_tracking' not found in ${targetMethod.name}")
}

// Find instruction by method invocation reference
val invokeIndex = instructions.indexOfFirst { inst ->
    (inst as? ReferenceInstruction)?.reference?.let { ref ->
        val mRef = ref as? MethodReference
        mRef?.name == "sendEvent" && mRef.definingClass == "Lcom/tracker/Analytics;"
    } == true
}
```

---

## 2. Standard Bytecode Neutralizers

Before writing custom Smali injection, check whether existing neutralizers from `app.aidan.patches.shared.BytecodeNeutralizers` can fulfill the requirement. These utilities handle overload traversal, parameter signature matching, and idempotency.

```kotlin
import app.aidan.patches.shared.disableOptionalVoidMethods
import app.aidan.patches.shared.disableRequiredVoidMethods
import app.aidan.patches.shared.disableRequiredVoidMethodsByName
import app.aidan.patches.shared.returnOptionalBoolean
import app.aidan.patches.shared.returnRequiredBoolean
import app.aidan.patches.shared.returnOptionalNullObject
import app.aidan.patches.shared.returnRequiredNullObject
import app.aidan.patches.shared.returnOptionalString
```

### Neutralizer Catalog & Semantics

| Helper Method | Required / Optional | Semantics |
|---|---|---|
| `disableRequiredVoidMethods(classDescriptor, *methodNames)` | **Required** | Class must exist. At least one of the named void methods must be present and will be injected with `return-void` at index 0. |
| `disableRequiredVoidMethodsByName(classDescriptor, *methodNames)` | **Required** | Class must exist. **Every** named method must be present and neutralized with `return-void`. |
| `disableOptionalVoidMethods(classDescriptor, *methodNames)` | Optional | If class exists, all matching implemented void methods are neutralized with `return-void`. Missing classes or methods do not throw. |
| `returnRequiredBoolean(classDescriptor, methodName, value)` | **Required** | Class and method must exist. Injects `const/4 v0, <0x0|0x1>` and `return v0` at index 0 for all matching boolean-returning overloads. |
| `returnOptionalBoolean(classDescriptor, methodName, value)` | Optional | Gracefully succeeds if class or method is absent; otherwise forces return value. |
| `returnRequiredNullObject(classDescriptor, methodName)` | **Required** | Class and method must exist. Injects `const/4 v0, 0x0` and `return-object v0` for class-returning overloads. |
| `returnOptionalNullObject(classDescriptor, methodName)` | Optional | Same as above, but ignores missing classes or methods. |
| `returnOptionalString(classDescriptor, methodName, value)` | Optional | If method exists, injects `const-string v0, "<escaped>"` and `return-object v0`. |

### Example: Disabling Analytics & Telemetry
```kotlin
execute {
    // Required: crash reporter class must be present
    disableRequiredVoidMethodsByName(
        "Lcom/example/analytics/Tracker;",
        "trackEvent",
        "logException",
        "flushQueue"
    )

    // Optional: in-app survey prompt might not exist in all versions
    returnOptionalBoolean(
        "Lcom/example/features/SurveyGate;",
        "shouldDisplaySurvey",
        false
    )
}
```

---

## 3. Custom Smali Instruction Injection

When neutralizers are insufficient (e.g. conditional branches, bridge invocations, state updates), use `addInstructions` or `replaceInstruction`.

```kotlin
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.extensions.InstructionExtensions.replaceInstruction
```

### Register Allocation & Parameter Registers
1. **Local vs Parameter Registers**:
   - In Smali, parameters use `p0`, `p1`, `p2`, etc.
   - For **instance (virtual/direct)** methods:
     - `p0` is `this`.
     - `p1` is the 1st parameter.
   - For **static** methods:
     - `p0` is the 1st parameter.
     - `p1` is the 2nd parameter.
   - Local registers use `v0`, `v1`, etc.
2. **Register Count**:
   - `addInstructions` inserts Smali text. If injecting local registers (e.g. `v0`, `v1`), ensure they do not clobber live variables needed by subsequent instructions, or allocate a register that is immediately overwritten.
3. **Format `22c` Opcode Register Width Restrictions**:
   - Dalvik opcodes follow format specifications defined by the DEX bytecode specification.
   - Format `22c` opcodes (such as `instance-of vA, vB, type@CCCC` or `check-cast`) use **4-bit register indices**.
   - Therefore, `vA` and `vB` **MUST** be $\le 15$ (`v0` through `v15`). Specifying `v16` or higher will fail DEX verification with an `IndexOutOfBoundsException` or `VerifyError` at runtime.

### Example: Calling a Java Extension Bridge
```kotlin
// Inject at method entry (index 0)
// p0 is the Activity instance in an instance method
targetMethod.addInstructions(
    0,
    """
    invoke-static {p0}, Lapp/aidan/extension/sezzle/ConsentGate;->maybeShow(Landroid/app/Activity;)V
    """.trimIndent()
)
```

### Example: Replacing an Instruction with Idempotency Guard
```kotlin
val instructions = targetMethod.implementation!!.instructions.toList()
val branchIndex = instructions.indexOfFirst { it.opcode == Opcode.IF_EQZ }

// Guard idempotency: check if already patched
val prev = instructions.getOrNull(branchIndex - 1)
val alreadyPatched = prev?.opcode == Opcode.CONST_4 &&
    (prev as? OneRegisterInstruction)?.registerA == 0

if (!alreadyPatched) {
    // Replace conditional branch or force condition
    targetMethod.addInstructions(
        branchIndex,
        """
        const/4 v0, 0x1
        """.trimIndent()
    )
}
```

---

## 4. Pre-Validation with APK Lab

Before building the full bundle, statically validate register widths, Smali opcodes, literal bounds, and control-flow graphs using `apk-lab validate-smali`:

```bash
# Validate Smali code snippet directly
uv run --project tools/apk-lab apk-lab validate-smali --smali "
    const/4 v0, 0x1
    return v0
"

# Validate a Smali file extracted during inspection
uv run --project tools/apk-lab apk-lab validate-smali path/to/Class.smali
```

`apk-lab validate-smali` verifies:
- Format `22c` register bounds ($v0 \dots v15$).
- Integer literal ranges (e.g. `const/4` requires $-8 \dots 7$, `const/16` requires $-32768 \dots 32767$).
- Target label existence for branch instructions (`goto`, `if-eqz`).
- Valid descriptor formatting.

---

## 5. Early Return & Control-Flow Safety Rules

1. **Clean Method Entry Neutralization**:
   - When injecting an early return (`return-void`, `return v0`, etc.) at index 0, ensure no side effects (such as critical superclass constructors `invoke-direct {p0}, Ljava/lang/Object;-><init>()V`) are bypassed in `<init>` methods.
   - In `<init>` or `<clinit>` constructors, never inject an early return before the superclass constructor call.
2. **Try/Catch Boundary Preservation**:
   - Inserting instructions at index 0 shifts subsequent instruction indices.
   - Dexlib2 automatically adjusts try-catch block offsets when using `addInstructions`, but avoid splitting an instruction pair that assumes adjacent execution (such as `packed-switch` or `fill-array-data` payloads).
3. **Double Patching Protection**:
   - Always check `instructions` to determine if the target instruction or bridge call already exists before injecting.
