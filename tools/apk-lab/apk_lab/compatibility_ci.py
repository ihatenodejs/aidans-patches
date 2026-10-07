from __future__ import annotations

import argparse
import glob
import json
import os
import sys
import tempfile
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path
from typing import Any

from apk_lab.acquisition import AcquisitionError
from apk_lab.fixtures import FixtureError, R2FixtureManager, SlotMetadata
from apk_lab.inspection import InspectionError
from apk_lab.models import ExitCode, PatchCompatibilityReport
from apk_lab.morphe import load_patches_list, run_compatibility_check


def resolve_matrix_packages(
    patches_data: dict[str, Any],
    event_name: str,
    dispatch_pkg: str | None = None,
    input_pkg: str | None = None,
) -> list[str]:
    """Resolves target packages for the CI matrix, rejecting unknown packages."""
    known = sorted(
        {
            cp["packageName"]
            for p in patches_data.get("patches", [])
            for cp in p.get("compatiblePackages", [])
            if cp.get("packageName")
        }
    )

    if event_name == "repository_dispatch":
        pkg = (dispatch_pkg or "").strip()
        if pkg:
            if pkg not in known:
                raise ValueError(f"Unknown package in repository dispatch: {pkg}")
            return [pkg]
        return known

    if event_name == "workflow_dispatch":
        pkg = (input_pkg or "all").strip()
        if pkg and pkg != "all":
            if pkg not in known:
                raise ValueError(f"Unknown package in workflow dispatch: {pkg}")
            return [pkg]
        return known

    return known


def get_target_version_for_package(
    patches_data: dict[str, Any], package_name: str
) -> str:
    """Finds the highest target version for a package in patches-list.json."""
    target_versions: set[str] = set()
    for p in patches_data.get("patches", []):
        for cp in p.get("compatiblePackages", []):
            if cp.get("packageName") == package_name:
                for t in cp.get("targets", []):
                    if t.get("version"):
                        target_versions.add(t["version"])
    return max(target_versions) if target_versions else ""


def build_result_item(role: str, report: PatchCompatibilityReport) -> dict[str, Any]:
    """Builds a single role result item matching CompatibilityResultInput."""
    status = report.overall_status
    if status not in ("compatible", "incompatible", "error"):
        status = "error" if report.failed_cases > 0 else "compatible"

    return {
        "role": role,
        "versionName": report.version_name,
        "versionCode": report.version_code,
        "patchBundleVersion": report.patch_bundle_version,
        "gitRevision": report.git_revision,
        "status": status,
        "passedCount": report.passed_cases,
        "failedCount": report.failed_cases,
        "workflowRunUrl": os.environ.get("GITHUB_SERVER_URL", "https://github.com")
        + f"/{os.environ.get('GITHUB_REPOSITORY', '')}/actions/runs/{os.environ.get('GITHUB_RUN_ID', '')}"
        if os.environ.get("GITHUB_RUN_ID")
        else None,
    }


def post_compatibility_submission(
    worker_url: str,
    status_secret: str,
    submission: dict[str, Any],
) -> int:
    """Posts a batched submission to the Worker, returning the HTTP response status."""
    req = urllib.request.Request(
        f"{worker_url.rstrip('/')}/api/compatibility-results",
        data=json.dumps(submission).encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {status_secret}",
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.status


def run_ci_reconcile_and_test(
    pkg: str,
    mpp_path: Path,
    runner_temp: Path,
    r2_mgr: Any = None,
    acquirer: Callable[..., Any] | None = None,
    checker: Callable[..., Any] | None = None,
    poster: Callable[..., Any] | None = None,
) -> int:
    """Executes CI acquisition, rotation, target check, and divergent latest check for a package."""
    patches_data = load_patches_list()
    target_version = get_target_version_for_package(patches_data, pkg)
    print(f"Checking {pkg} with target version {target_version}")

    if r2_mgr is None:
        r2_mgr = R2FixtureManager()
    if checker is None:
        checker = run_compatibility_check
    if poster is None:
        poster = post_compatibility_submission

    request_id = (
        os.environ.get("DISPATCH_REQUEST_ID")
        or f"ci-{os.environ.get('GITHUB_RUN_ID', 'manual')}"
    )
    status_secret = os.environ.get("COMPATIBILITY_STATUS_SECRET")
    worker_url = os.environ.get("WORKER_STATUS_URL", "https://worker.patch.p0ntus.com")

    latest_slot: SlotMetadata | None = r2_mgr.get_slot_metadata(pkg, "latest")
    target_slot: SlotMetadata | None = r2_mgr.get_slot_metadata(pkg, "target")

    # 1. Acquire new latest if credentials present
    has_creds = bool(os.environ.get("APKEEP_EMAIL")) or bool(
        os.environ.get("R2_ACCESS_KEY_ID")
    )
    if has_creds:
        try:
            if acquirer is None:
                from apk_lab.acquisition import acquire_artifact

                acquirer = acquire_artifact

            dl_dir = runner_temp / f"{pkg}-acquire"
            new_latest_path, src = acquirer(pkg, dl_dir)
            print(f"Acquired {pkg} via {src}: {new_latest_path}")
            new_latest_meta, updated_target_meta = r2_mgr.rotate_slots_on_new_latest(
                pkg,
                new_latest_path,
                target_version,
                source=getattr(src, "value", str(src)),
            )
            latest_slot = new_latest_meta
            if updated_target_meta:
                target_slot = updated_target_meta
        except (
            AcquisitionError,
            FixtureError,
            InspectionError,
            OSError,
            ValueError,
            RuntimeError,
        ) as e:
            print(f"Acquisition or rotation warning: {e}", file=sys.stderr)

    # 2. Select and download target fixture
    target_file = runner_temp / f"{pkg}-target.apk"
    if target_slot:
        r2_mgr.download_slot(pkg, "target", target_file)
    elif latest_slot:
        r2_mgr.download_slot(pkg, "latest", target_file)
    else:
        print(f"Error: No usable fixture slot available for {pkg}", file=sys.stderr)
        return ExitCode.INVALID_ARTIFACT

    results: list[dict[str, Any]] = []
    overall_exit = ExitCode.SUCCESS

    # Run target check
    t_report, t_code = checker(
        target_file, mpp_path, expected_package=pkg, all_patches=True
    )
    print(
        f"Target compatibility: {t_report.overall_status} ({t_report.passed_cases}/{t_report.total_cases})"
    )
    results.append(build_result_item("target", t_report))
    if t_code != 0 or t_report.overall_status != "compatible":
        overall_exit = t_code or ExitCode.USAGE_OR_TOOL_ERROR

    # 3. Test latest slot if divergent
    if latest_slot and target_slot and latest_slot.sha256 != target_slot.sha256:
        latest_file = runner_temp / f"{pkg}-latest.apk"
        r2_mgr.download_slot(pkg, "latest", latest_file)
        l_report, l_code = checker(
            latest_file, mpp_path, expected_package=pkg, all_patches=True, force=True
        )
        print(
            f"Latest compatibility: {l_report.overall_status} ({l_report.passed_cases}/{l_report.total_cases})"
        )
        results.append(build_result_item("latest", l_report))
        if l_code != 0 or l_report.overall_status != "compatible":
            overall_exit = l_code or ExitCode.USAGE_OR_TOOL_ERROR

    # 4. Post batched results if configured
    if status_secret and worker_url:
        submission = {
            "requestId": request_id,
            "packageName": pkg,
            "results": results,
        }
        try:
            status_code = poster(worker_url, status_secret, submission)
            print(f"Posted {len(results)} results to worker: HTTP {status_code}")
        except (urllib.error.URLError, TimeoutError, OSError, ValueError) as err:
            print(f"Failed to post results to worker: {err}", file=sys.stderr)
            # If secret was explicitly supplied, posting failure is treated as error
            if overall_exit == ExitCode.SUCCESS:
                overall_exit = ExitCode.INFRASTRUCTURE_FAILURE

    return overall_exit


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="APK Lab CI Orchestration Helper")
    subparsers = parser.add_subparsers(dest="subcommand", required=True)

    p_matrix = subparsers.add_parser("matrix", help="Compute package matrix")
    p_matrix.add_argument("--output-file", help="File to write GITHUB_OUTPUT to")

    p_run = subparsers.add_parser("run", help="Run compatibility check for package")
    p_run.add_argument("--package", required=True, help="Package name")
    p_run.add_argument("--mpp", help="Path to .mpp patch bundle")

    args = parser.parse_args(argv)

    if args.subcommand == "matrix":
        data = load_patches_list()
        event_name = os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch")
        dispatch_pkg = os.environ.get("DISPATCH_PACKAGE_NAME")
        input_pkg = os.environ.get("INPUT_PACKAGE_NAME")

        try:
            selected = resolve_matrix_packages(
                data, event_name, dispatch_pkg=dispatch_pkg, input_pkg=input_pkg
            )
        except ValueError as e:
            print(f"Matrix resolution error: {e}", file=sys.stderr)
            return ExitCode.USAGE_OR_TOOL_ERROR

        matrix_json = json.dumps(selected)
        print(f"packages={matrix_json}")
        if args.output_file:
            with open(args.output_file, "a", encoding="utf-8") as f:
                f.write(f"packages={matrix_json}\n")
        return ExitCode.SUCCESS

    if args.subcommand == "run":
        mpp_path = None
        if args.mpp:
            mpp_path = Path(args.mpp).resolve()
        else:
            mpp_files = sorted(glob.glob("patches/build/libs/patches-*.mpp"))
            if mpp_files:
                mpp_path = Path(mpp_files[-1]).resolve()

        if not mpp_path or not mpp_path.is_file():
            print("Error: No MPP bundle found in patches/build/libs/", file=sys.stderr)
            return ExitCode.USAGE_OR_TOOL_ERROR

        runner_temp = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir()))
        return run_ci_reconcile_and_test(args.package, mpp_path, runner_temp)

    return ExitCode.SUCCESS


if __name__ == "__main__":
    sys.exit(main())
