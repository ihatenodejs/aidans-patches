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
) -> None:
    """Aligns to 4-byte boundaries, signs with debug keystore, and verifies signature."""
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
        "androiddebugkey",
        "--ks-pass",
        "pass:android",
        "--key-pass",
        "pass:android",
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
    tool_manager: ToolManager | None = None,
    adb_client: AdbClient | None = None,
) -> DeployResult:
    """End-to-end patch application, alignment, signing, installation, and launch."""
    tm = tool_manager or ToolManager()
    client = adb_client or AdbClient()

    # 1. Resolve ADB device
    devices = parse_adb_devices(client.run(["devices"]).stdout)
    serial = select_device(devices, device_serial)

    # 2. Resolve final output path and ensure parent exists
    in_artifact = artifact_path.resolve()
    if out_apk_path:
        final_output = Path(out_apk_path).resolve()
    else:
        final_output = in_artifact.parent / f"{in_artifact.stem}-patched.apk"

    final_parent = final_output.parent
    final_parent.mkdir(parents=True, exist_ok=True)

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

        # Align and sign
        zipalign_and_sign(
            unsigned_apk=staged_unsigned,
            aligned_and_signed_apk=staged_aligned,
            keystore_path=keystore_path,
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
    result = deploy_artifact(
        artifact_path=Path(args.artifact),
        mpp_path=Path(args.mpp),
        package_name=args.package,
        all_defaults=args.all,
        enabled_patches=args.enable,
        raw_options=args.options,
        device_serial=args.device,
        launch=args.launch,
        clean_install=args.clean_install,
        out_apk_path=Path(args.out) if args.out else None,
    )
    print(result.format_human())
    return ExitCode.SUCCESS
