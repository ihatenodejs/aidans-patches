import java.util.Properties
group = "app.aidan"

patches {
    about {
        name = "Aidan's Patches"
        description = "My Morphe patches; mainly focused on financial and productivity/utility apps"
        source = "git@github.com:ihatenodejs/aidans-patches.git"
        author = "Aidan"
        contact = "https://github.com/ihatenodejs/aidans-patches/issues"
        website = "https://github.com/ihatenodejs/aidans-patches"
        license = "WTFPL"
    }
}

// Separate configuration so gson is available at runtime for the
// generatePatchesList task but never bundled into the APK.
val patchListGeneratorClasspath = configurations.create("patchListGeneratorClasspath")

dependencies {
    compileOnly(libs.gson)
    patchListGeneratorClasspath(libs.gson)
}
val emojiFontDir = layout.buildDirectory.dir("generated/resources/emojiFont")
val outputFontFile = emojiFontDir.map { it.file("fonts/AppleColorEmoji.ttf") }

val prepareEmojiFont = tasks.register<Exec>("prepareEmojiFont") {
    description = "Downloads and transforms Apple Color Emoji font for Fizz"
    outputs.file(outputFontFile)

    val fontCacheDir = File(gradle.gradleUserHomeDir, "caches/aidans-patches/fonts")
    val scriptFile = rootProject.file(".github/scripts/prepare_emoji_font.py")

    inputs.file(scriptFile)
    inputs.property("scriptVersion", "1.0.1")

    workingDir = rootDir
    doFirst {
        fontCacheDir.mkdirs()
        outputFontFile.get().asFile.parentFile.mkdirs()
    }

    commandLine(
        "python3",
        scriptFile.absolutePath,
        "--output",
        outputFontFile.get().asFile.absolutePath,
        "--cache-dir",
        fontCacheDir.absolutePath,
    )
}

val nhlAdblockNativeDir = layout.buildDirectory.dir("generated/resources/nhlAdblockNative")
val outputNhlAdblockSo = nhlAdblockNativeDir.map { it.file("nhl/adblock/libnhladblock.so") }
val REQUIRED_NDK_VERSION = "28.2.13676358"

val buildNhlAdblockNative = tasks.register<Exec>("buildNhlAdblockNative") {
    description = "Compiles native Rust adblock engine for arm64-v8a"
    val nativeDir = rootProject.file("extensions/nhl-adblock-native")
    inputs.dir(File(nativeDir, "src"))
    inputs.file(File(nativeDir, "Cargo.toml"))
    if (File(nativeDir, "Cargo.lock").exists()) {
        inputs.file(File(nativeDir, "Cargo.lock"))
    }
    outputs.file(outputNhlAdblockSo)

    workingDir = nativeDir

    doFirst {
        val ndkDir = run {
            val ndkHomeEnv = System.getenv("ANDROID_NDK_HOME")
            if (!ndkHomeEnv.isNullOrBlank()) {
                val f = File(ndkHomeEnv)
                if (f.exists()) return@run f
            }
            val sdkDir = run {
                val envSdk = System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
                if (!envSdk.isNullOrBlank()) {
                    File(envSdk)
                } else {
                    val localProps = rootProject.file("local.properties")
                    if (localProps.exists()) {
                        val props = Properties().apply { localProps.inputStream().use { load(it) } }
                        val dir = props.getProperty("sdk.dir")
                        if (!dir.isNullOrBlank()) File(dir) else null
                    } else null
                }
            } ?: throw GradleException("Could not resolve Android SDK path for NDK compilation. Set ANDROID_HOME or local.properties.")
            val d = File(sdkDir, "ndk/$REQUIRED_NDK_VERSION")
            if (!d.exists()) {
                throw GradleException("Required Android NDK $REQUIRED_NDK_VERSION not found at ${d.absolutePath}. Install NDK $REQUIRED_NDK_VERSION.")
            }
            d
        }

        val osName = System.getProperty("os.name").lowercase()
        val hostTag = when {
            osName.contains("mac") || osName.contains("darwin") -> "darwin-x86_64"
            osName.contains("linux") -> "linux-x86_64"
            osName.contains("windows") -> "windows-x86_64"
            else -> "darwin-x86_64"
        }
        val clangFile = File(ndkDir, "toolchains/llvm/prebuilt/$hostTag/bin/aarch64-linux-android29-clang")
        if (!clangFile.exists()) {
            throw GradleException("NDK clang compiler not found at ${clangFile.absolutePath}")
        }

        environment("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER", clangFile.absolutePath)
        environment("RUSTFLAGS", "-C link-arg=-Wl,-z,max-page-size=16384")
    }

    commandLine(
        "cargo",
        "build",
        "--release",
        "--target",
        "aarch64-linux-android",
        "--manifest-path",
        File(nativeDir, "Cargo.toml").absolutePath,
    )

    doLast {
        val builtSo = File(nativeDir, "target/aarch64-linux-android/release/libnhladblock.so")
        if (!builtSo.exists()) {
            throw GradleException("Cargo succeeded but output binary not found at ${builtSo.absolutePath}")
        }
        val dest = outputNhlAdblockSo.get().asFile
        dest.parentFile.mkdirs()
        builtSo.copyTo(dest, overwrite = true)
    }
}

sourceSets["main"].resources.srcDir(emojiFontDir)
sourceSets["main"].resources.srcDir(nhlAdblockNativeDir)

tasks {
    named("processResources") {
        dependsOn(prepareEmojiFont)
        dependsOn(buildNhlAdblockNative)
    }
    named("sourcesJar") {
        dependsOn(prepareEmojiFont)
        dependsOn(buildNhlAdblockNative)
    }


    named("build") {
        finalizedBy("buildAndroid")
    }

    register<JavaExec>("generatePatchesList") {
        description = "Build patch with patch list"

        dependsOn("buildAndroid")

        classpath = sourceSets["main"].runtimeClasspath + patchListGeneratorClasspath
        mainClass.set("util.PatchListGeneratorKt")
    }

    // Used by gradle-semantic-release-plugin.
    publish {
        dependsOn("generatePatchesList")
    }
}
