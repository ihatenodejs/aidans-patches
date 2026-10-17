# Java Extension Runtime Bridges Reference

This reference covers the design, packaging, Smali ABI linkage, reflection patterns, Android lifecycle safety, and UI threading requirements for Morphe Java runtime bridges under `extensions/`.

---

## 1. Architecture and Packaging

Morphe patches modify existing DEX code, but authoring complex features (such as custom dialogs, WebView renderers, or dynamic interceptors) directly in Smali is impractical. Java extension bridges allow writing high-level Java code that is packaged into an extension bundle (`.mpe`) and merged into the target APK's DEX.

### Directory Structure & Build Output
- **Java Source Root**: `extensions/extension/src/main/java/app/aidan/extension/<app-or-domain>/`
- **Manifest**: `extensions/extension/src/main/AndroidManifest.xml` (keep minimal; only declare components strictly required by Android runtime).
- **Gradle Module**: `extensions/extension/build.gradle.kts`
- **Output Artifact**: `extensions/extension.mpe`
- **Compilation Command**:
  ```bash
  ./gradlew :extensions:extension:compileReleaseJavaWithJavac --console=plain
  ```

### Linking with Kotlin Patches
In the companion Kotlin patch under `patches/src/main/kotlin/app/aidan/patches/...`, link the extension using `extendWith(...)`:

```kotlin
val myFeaturePatch = bytecodePatch(
    name = "My Feature",
    description = "...",
    default = true,
) {
    // Declares runtime dependency on the extension bundle
    extendWith("extensions/extension.mpe")

    execute {
        // Injected Smali invokes bridge methods
    }
}
```

---

## 2. Bridge Class Conventions & Smali ABI

### Class Structure Template
Bridge classes should follow strict conventions:
1. `public final class <BridgeName>`
2. `private <BridgeName>() {}` (prevent instantiation; static utilities only)
3. Narrow, static entry points.
4. Pass required Android context (`Context`, `Activity`) explicitly from Smali; avoid global singleton state.

```java
package app.aidan.extension.sezzle;

import android.app.Activity;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

public final class ConsentGate {
    private static final String TAG = "ConsentGate";

    private ConsentGate() {}

    /**
     * Entry point invoked from Activity.onResume() Smali injection.
     */
    public static void maybeShow(Activity activity) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) {
            return;
        }

        new Handler(Looper.getMainLooper()).post(() -> {
            showDialogSafely(activity);
        });
    }

    private static void showDialogSafely(Activity activity) {
        // Implementation...
    }
}
```

### Exact Descriptor Matching (Smali $\leftrightarrow$ Java ABI)
The public descriptor of a bridge method is an immutable binary contract. Any signature change in Java requires an identical change in the injected Smali:

| Java Method Signature | Smali Invocations Descriptor |
|---|---|
| `public static void maybeShow(Activity activity)` | `invoke-static {p0}, Lapp/aidan/extension/sezzle/ConsentGate;->maybeShow(Landroid/app/Activity;)V` |
| `public static boolean isFeatureAllowed(String key)` | `invoke-static {v0}, Lapp/aidan/extension/shared/FeatureGate;->isFeatureAllowed(Ljava/lang/String;)Z` |
| `public static String transformUrl(String rawUrl, int flags)` | `invoke-static {v0, v1}, Lapp/aidan/extension/web/UrlRouter;->transformUrl(Ljava/lang/String;I)Ljava/lang/String;` |

*Critical*: Never use primitive wrapper types (`java.lang.Boolean`, `java.lang.Integer`) unless nullability is strictly required; prefer primitive `boolean` (`Z`) and `int` (`I`) to avoid unnecessary autoboxing and descriptor complexity.

---

## 3. Reflection over Obfuscated Classes

Target applications are proprietary and obfuscated (e.g. `com.example.a.b.c`). Their classes do not exist in the Gradle classpath of `extensions/extension/`.

### Strict Rule: No Compile-Time Imports
**NEVER** attempt to import or reference target application package names in Java bridge files. Doing so causes compile-time failure.

### Reflection Strategy
Access target app classes, methods, and fields exclusively through dynamic Java reflection:

```java
public static Object extractSessionToken(Object targetService) {
    if (targetService == null) return null;

    try {
        // 1. Resolve class dynamically
        Class<?> clazz = targetService.getClass();

        // 2. Locate method by name and parameter types
        java.lang.reflect.Method getTokenMethod = clazz.getMethod("getAuthToken");
        getTokenMethod.setAccessible(true);

        // 3. Invoke reflectively
        return getTokenMethod.invoke(targetService);
    } catch (NoSuchMethodException e) {
        Log.w(TAG, "Method getAuthToken not found on " + targetService.getClass().getName(), e);
        return null;
    } catch (Exception e) {
        Log.e(TAG, "Failed to extract session token reflectively", e);
        return null;
    }
}
```

### Failure Policy: Safe Degradation vs Fail-Fast
- **Best-Effort Features (Telemetry suppression, optional UI styling)**: Catch reflection exceptions, log with `android.util.Log.w(TAG, ...)`, and return safe fallback values (`null`, `false`).
- **Core Feature Dependencies (Security gates, storage redirection)**: If a reflection target is indispensable for app stability, log an `ERROR` and degrade gracefully without throwing uncaught exceptions that crash the host app.

---

## 4. Android UI, Threading & Lifecycle Safety

### UI Thread Dispatch
Smali bytecode injections may occur on background worker threads (e.g. OkHttp interceptors, RxJava/Coroutine pools). Any code modifying Android Views, creating `Dialog` instances, or accessing `Toast` **MUST** run on the main looper:

```java
public static void runOnMainThread(Runnable action) {
    if (Looper.myLooper() == Looper.getMainLooper()) {
        action.run();
    } else {
        new Handler(Looper.getMainLooper()).post(action);
    }
}
```

### Activity Lifecycle Invariants
Never attach a dialog or view to a dead or dying activity. Always check lifecycle state before performing UI operations:

```java
if (activity == null || activity.isFinishing()) {
    return;
}
if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.JELLY_BEAN_MR1) {
    if (activity.isDestroyed()) {
        return;
    }
}
```

### Preventing Memory Leaks
- Never store `Activity` or `View` instances in static fields.
- If an asynchronous callback or event listener must retain a reference to a UI component, wrap it in a `WeakReference<Activity>` or `WeakReference<View>`.
- Dismiss any created dialogs when the parent activity finishes or is destroyed.

---

## 5. Safe WebView Configuration

When integrating custom web views (e.g. OpenStreetMap renderers or custom documentation):

```java
WebView webView = new WebView(activity);
WebSettings settings = webView.getSettings();

// Enable JavaScript only if necessary
settings.setJavaScriptEnabled(true);

// Disable insecure local file access
settings.setAllowFileAccess(false);
settings.setAllowContentAccess(false);
if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.JELLY_BEAN) {
    settings.setAllowFileAccessFromFileURLs(false);
    settings.setAllowUniversalAccessFromFileURLs(false);
}

// Add restricted JavaScript interface
webView.addJavascriptInterface(new SafeBridgeInterface(), "AndroidBridge");
```

---

## 6. Verification Protocol

Extension code has no synthetic unit test suite because reflection targets exist only in the real APK. Follow this verification sequence:

1. **Compile Java**:
   ```bash
   ./gradlew :extensions:extension:compileReleaseJavaWithJavac --console=plain
   ```
2. **Build Bundle**:
   ```bash
   ./gradlew :patches:buildAndroid
   ```
3. **Verify DEX Merge & Smali Descriptors**:
   ```bash
   uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
     --mpp patches/build/libs/patches-<version>.mpp \
     --package <pkg> \
     --all
   ```
   `apk-lab check` automatically verifies that all extension classes declared in `extensions/extension.mpe` are properly present in the patched APK's DEX.
4. **Device Smoke & Monitoring**:
   Deploy and monitor the app to confirm reflection targets resolve without runtime `ClassNotFoundException` or `NoSuchMethodError`:
   ```bash
   uv run --project tools/apk-lab apk-lab deploy path/to/patched.apk --launch
   uv run --project tools/apk-lab apk-lab monitor --package <pkg>
   ```
