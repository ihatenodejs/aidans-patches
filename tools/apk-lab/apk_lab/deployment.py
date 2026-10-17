from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Literal

from apk_lab.device import (
    AdbClient,
    get_installed_package_signature,
    install_apk,
    launch_component_intent,
    parse_adb_devices,
    parse_launcher_activity,
    select_device,
)
from apk_lab.inspection import find_build_tools_bin, inspect_artifact
from apk_lab.models import DeployResult, ExitCode
from apk_lab.morphe import (
    apply_patch_set,
    load_patches_list,
    resolve_patch_selections,
)
from apk_lab.tools import ToolManager


def find_repo_root(start_path: Path | None = None) -> Path:
    """Locates the repository root by searching upward for gradlew."""
    curr = (start_path or Path.cwd()).resolve()
    for parent in [curr, *curr.parents]:
        if (parent / "gradlew").is_file():
            return parent
    raise RuntimeError(
        "Could not find repository root (gradlew not found in ancestors)"
    )


def build_patch_bundle(repo_root: Path) -> Path:
    """Runs ./gradlew :patches:buildAndroid and returns the newest .mpp bundle."""
    gradlew = repo_root / "gradlew"
    if not gradlew.is_file():
        raise RuntimeError(f"Gradle wrapper not found at {gradlew}")

    proc = subprocess.run(
        [str(gradlew), ":patches:buildAndroid"],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"Gradle build failed (exit code {proc.returncode}):\n{proc.stderr or proc.stdout}"
        )

    libs_dir = repo_root / "patches" / "build" / "libs"
    if not libs_dir.is_dir():
        raise RuntimeError(f"Output directory does not exist: {libs_dir}")

    mpp_files = list(libs_dir.glob("*.mpp"))
    if not mpp_files:
        raise RuntimeError(f"No .mpp bundle found in {libs_dir}")

    mpp_files.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return mpp_files[0]


def find_app_artifact(target: str, repo_root: Path) -> Path | None:
    """Searches apk/, ~/Downloads/, and .apk-lab/ for matching APK/APKM/XAPK/APKS artifacts."""
    search_dirs = [
        repo_root / "apk",
        Path.home() / "Downloads",
        repo_root / ".apk-lab",
    ]
    valid_extensions = {".apk", ".apkm", ".xapk", ".apks"}
    target_lower = target.lower()

    for base_dir in search_dirs:
        if not base_dir.is_dir():
            continue
        # Scan files in base_dir and its subdirectories (e.g. apk/<App-Name>/)
        candidates: list[Path] = []
        try:
            for entry in base_dir.rglob("*"):
                if entry.is_file() and entry.suffix.lower() in valid_extensions:
                    # Check if target matches filename or any directory component under base_dir
                    rel_parts = entry.relative_to(base_dir).parts
                    if any(target_lower in part.lower() for part in rel_parts):
                        candidates.append(entry)
        except OSError:
            continue

        if candidates:
            # Prefer non-patched artifacts, then sort by mtime descending
            candidates.sort(
                key=lambda p: (
                    not p.name.endswith("-patched.apk"),
                    p.stat().st_mtime,
                ),
                reverse=True,
            )
            return candidates[0]

    return None


def ensure_debug_keystore(keystore_path: Path | None = None) -> Path:
    """Ensures a standard Android debug keystore exists, generating one if absent."""
    ks_path = keystore_path or (Path.home() / ".android" / "debug.keystore")
    ks_path = ks_path.resolve()

    if ks_path.is_file():
        return ks_path

    ks_path.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        "keytool",
        "-genkeypair",
        "-keystore",
        str(ks_path),
        "-storepass",
        "android",
        "-alias",
        "androiddebugkey",
        "-keypass",
        "android",
        "-dname",
        "CN=Android Debug,O=Android,C=US",
        "-keyalg",
        "RSA",
        "-keysize",
        "2048",
        "-validity",
        "10000",
    ]
    res = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if res.returncode != 0:
        raise RuntimeError(
            f"Failed to generate debug keystore via keytool: {res.stderr}"
        )

    ks_path.chmod(0o600)
    return ks_path


def zipalign_and_sign(
    unsigned_apk: Path,
    aligned_and_signed_apk: Path,
    keystore_path: Path | None = None,
    ks_alias: str = "androiddebugkey",
    ks_pass: str = "android",
    key_pass: str = "android",
) -> None:
    """Aligns to 4-byte boundaries, signs with keystore, and verifies signature."""
    zipalign = find_build_tools_bin("zipalign")
    apksigner = find_build_tools_bin("apksigner")
    ks = ensure_debug_keystore(keystore_path)

    # 1. zipalign
    align_cmd = [
        str(zipalign),
        "-f",
        "4",
        str(unsigned_apk.resolve()),
        str(aligned_and_signed_apk.resolve()),
    ]
    res_align = subprocess.run(align_cmd, capture_output=True, text=True, check=False)
    if res_align.returncode != 0:
        raise RuntimeError(f"zipalign failed: {res_align.stderr}")

    # 2. apksigner sign (in-place)
    sign_cmd = [
        str(apksigner),
        "sign",
        "--ks",
        str(ks),
        "--ks-key-alias",
        ks_alias,
        "--ks-pass",
        f"pass:{ks_pass}",
        "--key-pass",
        f"pass:{key_pass}",
        str(aligned_and_signed_apk.resolve()),
    ]
    res_sign = subprocess.run(sign_cmd, capture_output=True, text=True, check=False)
    if res_sign.returncode != 0:
        raise RuntimeError(f"apksigner sign failed: {res_sign.stderr}")
    # 3. apksigner verify
    verify_cmd = [
        str(apksigner),
        "verify",
        "--verbose",
        "--print-certs",
        str(aligned_and_signed_apk.resolve()),
    ]
    res_verify = subprocess.run(verify_cmd, capture_output=True, text=True, check=False)
    if res_verify.returncode != 0:
        raise RuntimeError(f"apksigner verify failed: {res_verify.stderr}")


MORPHE_KEYSTORE_SHA256 = (
    "6959a7790bff42cc28fbe41c20d457401e32f4854f414dbc390de9f1529c9295"
)


def resolve_signing_keystore(
    package_name: str | None = None,
    device: str | None = None,
    explicit_keystore: Path | None = None,
    explicit_alias: str | None = None,
    explicit_pass: str | None = None,
    tool_manager: ToolManager | None = None,
) -> tuple[Path, str, str, str]:
    """Resolves signing keystore, alias, store password, and key password.

    Returns a tuple of (keystore_path, ks_alias, ks_pass, key_pass).
    - If explicit_keystore is provided, returns it with explicit or default credentials.
    - If package_name is installed on device, queries signature and matches Morphe keystore.
    - Defaults to standard debug keystore.
    """
    if explicit_keystore is not None:
        alias = explicit_alias or "androiddebugkey"
        password = explicit_pass or "android"
        return (explicit_keystore.resolve(), alias, password, password)

    if package_name:
        sig = get_installed_package_signature(package_name, device=device)
        if sig == MORPHE_KEYSTORE_SHA256:
            tm = tool_manager or ToolManager()
            # Check ToolManager.get_cache_dir("morphe") / "morphe-data" / "morphe.keystore"
            # ToolManager may have get_cache_dir or get_tool_install_dir
            morphe_dir = (
                tm.get_cache_dir("morphe")
                if hasattr(tm, "get_cache_dir")
                else tm.get_tool_install_dir("morphe")
            )
            morphe_ks = morphe_dir / "morphe-data" / "morphe.keystore"
            if morphe_ks.is_file():
                return (
                    morphe_ks.resolve(),
                    explicit_alias or "Morphe",
                    explicit_pass or "Morphe",
                    explicit_pass or "Morphe",
                )

    default_ks = ensure_debug_keystore()
    return (
        default_ks,
        explicit_alias or "androiddebugkey",
        explicit_pass or "android",
        explicit_pass or "android",
    )


def deploy_artifact(
    artifact_path: Path,
    mpp_path: Path,
    package_name: str,
    all_defaults: bool,
    enabled_patches: list[str],
    raw_options: list[str],
    device_serial: str | None = None,
    launch: bool = False,
    clean_install: bool = False,
    out_apk_path: Path | None = None,
    keystore_path: Path | None = None,
    ks_alias: str | None = None,
    ks_pass: str | None = None,
    key_pass: str | None = None,
    tool_manager: ToolManager | None = None,
    adb_client: AdbClient | None = None,
) -> DeployResult:
    """End-to-end patch application, alignment, signing, installation, and launch."""
    tm = tool_manager or ToolManager()
    client = adb_client or AdbClient()

    # 1. Resolve and validate output path (prevent overwriting input artifact)
    in_artifact = artifact_path.resolve()
    if out_apk_path:
        final_output = Path(out_apk_path).resolve()
    else:
        final_output = in_artifact.parent / f"{in_artifact.stem}-patched.apk"

    if final_output == in_artifact:
        raise ValueError("Deployment output must not overwrite the input artifact")

    final_parent = final_output.parent
    final_parent.mkdir(parents=True, exist_ok=True)

    # 2. Resolve ADB device
    devices = parse_adb_devices(client.run(["devices"]).stdout)
    serial = select_device(devices, device_serial)

    # 3. Resolve patch selections
    patches_list_data = load_patches_list()
    selections = resolve_patch_selections(
        package_name=package_name,
        requested_patches=enabled_patches,
        all_defaults=all_defaults,
        raw_options=raw_options,
        patches_list_data=patches_list_data,
    )
    if not selections:
        raise ValueError(
            f"No compatible patches selected for package '{package_name}'. Use --all or specify valid -e patches."
        )

    # 4. Atomic staging inside final_parent
    staging_dir = Path(tempfile.mkdtemp(prefix="apk-lab-stage-", dir=final_parent))
    try:
        staged_unsigned = staging_dir / "unsigned.apk"
        staged_aligned = staging_dir / "aligned.apk"
        scratch_dir = staging_dir / "scratch"

        # Apply Morphe patches
        applied = apply_patch_set(
            tool_manager=tm,
            mpp_path=mpp_path,
            artifact_path=in_artifact,
            package_name=package_name,
            selections=selections,
            output_apk=staged_unsigned,
            scratch_dir=scratch_dir,
        )

        # Resolve keystore and signing credentials
        resolved_ks, resolved_alias, resolved_ks_pass, resolved_key_pass = (
            resolve_signing_keystore(
                package_name=package_name,
                device=serial,
                explicit_keystore=keystore_path,
                explicit_alias=ks_alias,
                explicit_pass=ks_pass,
                tool_manager=tm,
            )
        )
        final_key_pass = key_pass or resolved_key_pass

        # Align and sign
        zipalign_and_sign(
            unsigned_apk=staged_unsigned,
            aligned_and_signed_apk=staged_aligned,
            keystore_path=resolved_ks,
            ks_alias=resolved_alias,
            ks_pass=resolved_ks_pass,
            key_pass=final_key_pass,
        )
        # Re-inspect staged signed APK
        out_inspection = inspect_artifact(staged_aligned)
        if out_inspection.package_name != package_name:
            raise RuntimeError(
                f"Signed output package '{out_inspection.package_name}' does not match expected '{package_name}'"
            )

        # Atomically replace final destination
        os.replace(staged_aligned, final_output)
    finally:
        shutil.rmtree(staging_dir, ignore_errors=True)

    # 5. Install APK
    mode_str: Literal["reinstall", "clean-install"] = (
        "clean-install" if clean_install else "reinstall"
    )
    install_apk(
        client=client,
        apk_path=final_output,
        package_name=package_name,
        serial=serial,
        clean_install=clean_install,
    )

    # 6. Launch if requested
    launch_comp: str | None = None
    if launch:
        aapt2 = find_build_tools_bin("aapt2")
        badging_res = subprocess.run(
            [str(aapt2), "dump", "badging", str(final_output)],
            capture_output=True,
            text=True,
            check=False,
        )
        if badging_res.returncode != 0:
            raise RuntimeError(f"aapt2 dump badging failed: {badging_res.stderr}")

        launch_comp = parse_launcher_activity(badging_res.stdout, package_name)
        launch_component_intent(
            client=client, launch_component=launch_comp, serial=serial
        )

    return DeployResult(
        output_apk=str(final_output),
        package_name=package_name,
        device_serial=serial,
        applied_patches=applied,
        install_mode=mode_str,
        launch_component=launch_comp,
    )


def run_deploy(args: argparse.Namespace) -> int:
    """CLI handler for deploy subcommand."""
    mpp_arg = getattr(args, "mpp", None)
    build_flag = getattr(args, "build", False)

    if not mpp_arg and not build_flag:
        raise ValueError("Either --mpp or --build must be specified")

    if not mpp_arg and build_flag:
        repo_root = find_repo_root()
        mpp_path = build_patch_bundle(repo_root)
    else:
        assert mpp_arg is not None
        mpp_path = Path(mpp_arg)

    result = deploy_artifact(
        artifact_path=Path(args.artifact),
        mpp_path=mpp_path,
        package_name=args.package,
        all_defaults=args.all,
        enabled_patches=args.enable,
        raw_options=args.options,
        device_serial=args.device,
        launch=args.launch,
        clean_install=args.clean_install,
        out_apk_path=Path(args.out) if args.out else None,
        keystore_path=Path(args.keystore) if getattr(args, "keystore", None) else None,
        ks_alias=getattr(args, "ks_alias", None),
        ks_pass=getattr(args, "ks_pass", None),
        key_pass=getattr(args, "key_pass", None),
    )
    print(result.format_human())
    return ExitCode.SUCCESS
