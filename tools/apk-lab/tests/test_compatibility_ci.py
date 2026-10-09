from __future__ import annotations

import json
import urllib.error
from unittest.mock import MagicMock

import pytest
from apk_lab.compatibility_ci import (
    build_error_result,
    build_result_item,
    extract_package_from_constants,
    get_changed_patch_files,
    get_package_run_metadata,
    get_target_version_for_package,
    parse_requested_roles,
    post_compatibility_submission,
    post_run_start,
    resolve_matrix_packages,
    run_ci_reconcile_and_test,
)
from apk_lab.fixtures import SlotMetadata
from apk_lab.models import ExitCode, PatchCompatibilityReport


def test_resolve_matrix_packages_workflow_dispatch():
    data = {
        "patches": [
            {
                "compatiblePackages": [
                    {"packageName": "com.app.a"},
                    {"packageName": "com.app.b"},
                ]
            }
        ]
    }

    # All packages default
    assert resolve_matrix_packages(data, "workflow_dispatch") == [
        "com.app.a",
        "com.app.b",
    ]
    assert resolve_matrix_packages(data, "workflow_dispatch", input_pkg="all") == [
        "com.app.a",
        "com.app.b",
    ]

    # Specific valid package
    assert resolve_matrix_packages(
        data, "workflow_dispatch", input_pkg="com.app.a"
    ) == ["com.app.a"]

    # Unknown package must raise ValueError rather than falling back to all
    with pytest.raises(ValueError, match="Unknown package in workflow dispatch"):
        resolve_matrix_packages(data, "workflow_dispatch", input_pkg="com.unknown.app")


def test_resolve_matrix_packages_push_events(tmp_path):
    patches_root = tmp_path / "patches"
    app_base = patches_root / "src" / "main" / "kotlin" / "app" / "aidan" / "patches"

    # Create app A
    app_a_dir = app_base / "appa" / "shared"
    app_a_dir.mkdir(parents=True)
    (app_a_dir / "Constants.kt").write_text(
        'const val APPA_PACKAGE_NAME = "com.app.a"\n'
    )

    # Create app B
    app_b_dir = app_base / "appb" / "shared"
    app_b_dir.mkdir(parents=True)
    (app_b_dir / "Constants.kt").write_text(
        'const val APPB_PACKAGE_NAME = "com.app.b"\n'
    )

    data = {
        "patches": [
            {
                "compatiblePackages": [
                    {
                        "packageName": "com.app.a",
                        "name": "App A",
                        "targets": [{"version": "1.0.0"}],
                    },
                    {
                        "packageName": "com.app.b",
                        "name": "App B",
                        "targets": [{"version": "2.0.0"}],
                    },
                ]
            }
        ]
    }

    # 1. Single app changed -> returns only that package
    changed = [
        "patches/src/main/kotlin/app/aidan/patches/appa/features/FeaturePatch.kt"
    ]
    assert resolve_matrix_packages(
        data, "push", changed_paths=changed, patches_root=patches_root
    ) == ["com.app.a"]

    # 2. Multiple apps changed -> returns sorted deduplicated packages
    changed = [
        "patches/src/main/kotlin/app/aidan/patches/appb/ads/AdPatch.kt",
        "patches/src/main/kotlin/app/aidan/patches/appa/features/FeaturePatch.kt",
    ]
    assert resolve_matrix_packages(
        data, "push", changed_paths=changed, patches_root=patches_root
    ) == ["com.app.a", "com.app.b"]

    # 3. Non-app patch file changed (build.gradle.kts) -> fans out to all
    changed = [
        "patches/build.gradle.kts",
        "patches/src/main/kotlin/app/aidan/patches/appa/features/FeaturePatch.kt",
    ]
    assert resolve_matrix_packages(
        data, "push", changed_paths=changed, patches_root=patches_root
    ) == ["com.app.a", "com.app.b"]

    # 4. Shared utility changed -> fans out to all
    changed = ["patches/src/main/kotlin/util/PatchListGenerator.kt"]
    assert resolve_matrix_packages(
        data, "push", changed_paths=changed, patches_root=patches_root
    ) == ["com.app.a", "com.app.b"]

    # 5. Empty or None changed paths -> fans out to all
    assert resolve_matrix_packages(
        data, "push", changed_paths=[], patches_root=patches_root
    ) == ["com.app.a", "com.app.b"]
    assert resolve_matrix_packages(
        data, "push", changed_paths=None, patches_root=patches_root
    ) == ["com.app.a", "com.app.b"]

    # 6. Unrecognized or deleted app dir -> fans out to all
    changed = ["patches/src/main/kotlin/app/aidan/patches/deleted_app/SomePatch.kt"]
    assert resolve_matrix_packages(
        data, "push", changed_paths=changed, patches_root=patches_root
    ) == ["com.app.a", "com.app.b"]


def test_extract_package_from_constants(tmp_path):
    app_dir = tmp_path / "app"
    app_dir.mkdir()
    shared_dir = app_dir / "shared"
    shared_dir.mkdir()
    constants_file = shared_dir / "Constants.kt"

    # Valid constant
    constants_file.write_text('const val MY_APP_PACKAGE_NAME = "com.my.app"\n')
    assert extract_package_from_constants(app_dir) == "com.my.app"

    # Missing file
    constants_file.unlink()
    assert extract_package_from_constants(app_dir) is None

    # Ambiguous (multiple constants)
    constants_file.write_text(
        'const val A_PACKAGE_NAME = "com.a"\nconst val B_PACKAGE_NAME = "com.b"\n'
    )
    assert extract_package_from_constants(app_dir) is None


def test_get_changed_patch_files_zero_sha():
    assert (
        get_changed_patch_files("0000000000000000000000000000000000000000", "sha123")
        is None
    )
    assert get_changed_patch_files("", "sha123") is None
    assert get_changed_patch_files(None, "sha123") is None


def test_get_package_run_metadata():
    data = {
        "patches": [
            {
                "compatiblePackages": [
                    {
                        "packageName": "com.app.a",
                        "name": "App A",
                        "targets": [{"version": "1.0.0"}, {"version": "1.2.0"}],
                    }
                ]
            }
        ]
    }
    meta = get_package_run_metadata(data, "com.app.a")
    assert meta["packageName"] == "com.app.a"
    assert meta["appName"] == "App A"
    assert meta["targetVersion"] == "1.2.0"
    assert meta["supportedVersions"] == ["1.0.0", "1.2.0"]


def test_get_target_version_for_package():
    data = {
        "patches": [
            {
                "compatiblePackages": [
                    {
                        "packageName": "com.app.a",
                        "name": "App A",
                        "targets": [{"version": "1.0.0"}, {"version": "1.2.0"}],
                    }
                ]
            }
        ]
    }
    assert get_target_version_for_package(data, "com.app.a") == "1.2.0"
    assert get_target_version_for_package(data, "com.app.b") == ""


def test_parse_requested_roles():
    assert parse_requested_roles("target", "repository_dispatch") == ["target"]
    assert parse_requested_roles(None, "workflow_dispatch") is None
    assert parse_requested_roles("target", "workflow_dispatch") == ["target"]
    with pytest.raises(ValueError, match="Invalid requested role"):
        parse_requested_roles("invalid", "workflow_dispatch")


def test_post_compatibility_submission(monkeypatch):
    mock_urlopen = MagicMock()
    mock_resp = MagicMock()
    mock_resp.status = 200
    mock_resp.__enter__.return_value = mock_resp
    mock_urlopen.return_value = mock_resp
    monkeypatch.setattr(urllib.request, "urlopen", mock_urlopen)

    sub = {
        "requestId": "req-1",
        "packageName": "com.test.app",
        "results": [],
    }
    status = post_compatibility_submission("https://worker.test", "sec", sub)
    assert status == 200


def test_post_run_start_success(monkeypatch):
    mock_urlopen = MagicMock()
    mock_resp = MagicMock()
    mock_resp.status = 200
    mock_resp.__enter__.return_value = mock_resp
    mock_urlopen.return_value = mock_resp
    monkeypatch.setattr(urllib.request, "urlopen", mock_urlopen)

    payload = {
        "requestId": "req-1",
        "packageName": "com.test.app",
        "appName": "Test App",
        "targetVersion": "1.0.0",
        "supportedVersions": ["1.0.0"],
        "gitRevision": "sha123",
    }
    code = post_run_start("https://worker.test", "secret-test", payload)
    assert code == 200


def test_reconcile_and_test_uses_existing_r2_target(monkeypatch, tmp_path):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-123")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    target_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_target",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="manual",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )

    mock_acquirer = MagicMock()

    def mock_checker(
        apk_path, mpp_path, expected_package=None, all_patches=False, force=False
    ):
        report = PatchCompatibilityReport(
            artifact_sha256="sha",
            package_name=expected_package,
            version_name="1.0.0",
            version_code=100,
            patch_bundle_version="1.4.0",
            git_revision="rev1",
            tool_versions={},
            overall_status="compatible",
            total_cases=5,
            passed_cases=5,
            failed_cases=0,
        )
        return report, 0

    posted_submissions = []

    def mock_poster(url, secret, submission):
        posted_submissions.append(submission)
        return 200

    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        acquirer=mock_acquirer,
        checker=mock_checker,
        poster=mock_poster,
    )
    assert exit_code == ExitCode.SUCCESS
    # Acquirer was NOT called because fixture was already in R2
    mock_acquirer.assert_not_called()
    mock_r2.download_slot.assert_called_once()
    assert len(posted_submissions) == 1
    assert posted_submissions[0]["results"][0]["status"] == "compatible"
    assert posted_submissions[0]["results"][0]["role"] == "target"


def test_reconcile_and_test_missing_target_acquires_and_uploads(monkeypatch, tmp_path):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-missing-target")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    mock_r2 = MagicMock()
    # No slots initially
    mock_r2.get_slot_metadata.return_value = None

    uploaded_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_acquired",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="apkeep",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2.seed_fixture.return_value = uploaded_meta

    def mock_acquirer(pkg, out_dir, expected_version=None):
        out_dir.mkdir(parents=True, exist_ok=True)
        fake_file = out_dir / f"{pkg}.apkm"
        fake_file.write_bytes(b"downloaded")
        source = MagicMock()
        source.value = "apkeep"
        return fake_file, source

    def mock_checker(
        apk_path, mpp_path, expected_package=None, all_patches=False, force=False
    ):
        return (
            PatchCompatibilityReport(
                artifact_sha256="sha",
                package_name=expected_package,
                version_name="1.0.0",
                version_code=100,
                patch_bundle_version="1.4.0",
                git_revision="rev1",
                tool_versions={},
                overall_status="compatible",
                total_cases=5,
                passed_cases=5,
                failed_cases=0,
            ),
            0,
        )

    posted = []
    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        acquirer=mock_acquirer,
        checker=mock_checker,
        poster=lambda u, s, sub: posted.append(sub) or 200,
    )
    assert exit_code == ExitCode.SUCCESS
    # seed_fixture was called to persist target artifact into R2
    mock_r2.seed_fixture.assert_called_once()
    assert len(posted) == 1
    assert posted[0]["results"][0]["status"] == "compatible"


def test_reconcile_and_test_acquisition_failure_records_acquisition_stage(
    monkeypatch, tmp_path
):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-acq-fail")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.return_value = None

    def failing_acquirer(pkg, out_dir, expected_version=None):
        raise RuntimeError("Version 1.0.0 not found on Play Store or APKMirror")

    posted = []
    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        acquirer=failing_acquirer,
        poster=lambda u, s, sub: posted.append(sub) or 200,
    )
    assert exit_code == ExitCode.INVALID_ARTIFACT
    assert len(posted) == 1
    result = posted[0]["results"][0]
    assert result["status"] == "error"
    assert result["failureStage"] == "acquisition"
    assert "Version 1.0.0 not found" in result["failureReason"]


def test_reconcile_and_test_r2_lookup_failure(monkeypatch, tmp_path):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-r2-lookup-fail")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = RuntimeError("AWS/R2 credentials invalid")

    posted = []
    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        poster=lambda u, s, sub: posted.append(sub) or 200,
    )
    assert exit_code == ExitCode.INFRASTRUCTURE_FAILURE
    assert len(posted) == 1
    result = posted[0]["results"][0]
    assert result["status"] == "error"
    assert result["failureStage"] == "r2-lookup"
    assert "credentials invalid" in result["failureReason"]


def test_reconcile_and_test_r2_upload_failure(monkeypatch, tmp_path):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-r2-upload-fail")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.return_value = None
    mock_r2.seed_fixture.side_effect = RuntimeError("R2 upload timeout")

    def mock_acquirer(pkg, out_dir, expected_version=None):
        out_dir.mkdir(parents=True, exist_ok=True)
        fake_file = out_dir / f"{pkg}.apkm"
        fake_file.write_bytes(b"fake")
        return fake_file, MagicMock()

    posted = []
    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        acquirer=mock_acquirer,
        poster=lambda u, s, sub: posted.append(sub) or 200,
    )
    assert exit_code == ExitCode.INFRASTRUCTURE_FAILURE
    assert len(posted) == 1
    result = posted[0]["results"][0]
    assert result["status"] == "error"
    assert result["failureStage"] == "r2-upload"
    assert "upload timeout" in result["failureReason"]


def test_reconcile_and_test_r2_download_failure(monkeypatch, tmp_path):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-r2-dl-fail")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    target_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_target",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="manual",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )
    mock_r2.download_slot.side_effect = RuntimeError("R2 download broken pipe")

    posted = []
    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        poster=lambda u, s, sub: posted.append(sub) or 200,
    )
    assert exit_code == ExitCode.INFRASTRUCTURE_FAILURE
    assert len(posted) == 1
    result = posted[0]["results"][0]
    assert result["status"] == "error"
    assert result["failureStage"] == "r2-download"
    assert "broken pipe" in result["failureReason"]


def test_reconcile_and_test_compatibility_checker_exception(monkeypatch, tmp_path):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-checker-fail")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    target_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_target",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="manual",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )

    def crashing_checker(apk_path, mpp_path, expected_package=None, all_patches=False):
        raise RuntimeError("Morphe crashed with SIGSEGV in libmorphe")

    posted = []
    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        checker=crashing_checker,
        poster=lambda u, s, sub: posted.append(sub) or 200,
    )
    assert exit_code == ExitCode.USAGE_OR_TOOL_ERROR
    assert len(posted) == 1
    result = posted[0]["results"][0]
    assert result["status"] == "error"
    assert result["failureStage"] == "compatibility-check"
    assert "SIGSEGV" in result["failureReason"]


def test_reconcile_and_test_incompatible_report(monkeypatch, tmp_path):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-incompatible")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    target_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_target",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="manual",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )

    def mock_checker(
        apk_path, mpp_path, expected_package=None, all_patches=False, force=False
    ):
        report = PatchCompatibilityReport(
            artifact_sha256="sha",
            package_name=expected_package,
            version_name="1.0.0",
            version_code=100,
            patch_bundle_version="1.4.0",
            git_revision="rev1",
            tool_versions={},
            overall_status="incompatible",
            total_cases=5,
            passed_cases=4,
            failed_cases=1,
            failure_reason="One patch bytecode failed",
        )
        return report, 1

    posted = []
    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        checker=mock_checker,
        poster=lambda u, s, sub: posted.append(sub) or 200,
    )
    assert exit_code != 0
    assert len(posted) == 1
    result = posted[0]["results"][0]
    assert result["status"] == "incompatible"
    assert result.get("failureStage") is None
    assert result["passedCount"] == 4
    assert result["failedCount"] == 1


def test_build_error_result_with_failure_stage():
    res = build_error_result(
        role="target",
        expected_version="1.54.0",
        reason="Acquisition network timeout",
        failure_stage="acquisition",
        patch_bundle_version="1.4.0",
        git_revision="rev123",
    )
    assert res["role"] == "target"
    assert res["versionName"] == "1.54.0"
    assert res["status"] == "error"
    assert res["failureStage"] == "acquisition"
    assert res["failureReason"] == "Acquisition network timeout"


def test_build_result_item_with_error_status():
    report = PatchCompatibilityReport(
        artifact_sha256="sha",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        patch_bundle_version="1.4.0",
        git_revision="rev1",
        tool_versions={},
        overall_status="error",
        total_cases=2,
        passed_cases=0,
        failed_cases=2,
        failure_reason="Morphe error",
    )
    item = build_result_item("target", report)
    assert item["status"] == "error"
    assert item["failureStage"] == "compatibility-check"
    assert item["failureReason"] == "Morphe error"


def test_reconcile_and_test_success_writes_submission_file_and_marker(
    monkeypatch, tmp_path
):
    sub_file = tmp_path / "submission.json"
    marker_file = tmp_path / "callback-marker"
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-persist-ok")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-test")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setenv("COMPATIBILITY_SUBMISSION_PATH", str(sub_file))
    monkeypatch.setenv("COMPATIBILITY_CALLBACK_MARKER", str(marker_file))
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    target_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_target",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="apkeep",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )

    def mock_checker(
        apk_path, mpp_path, expected_package=None, all_patches=False, force=False
    ):
        report = PatchCompatibilityReport(
            artifact_sha256="sha",
            package_name=expected_package,
            version_name="1.0.0",
            version_code=100,
            patch_bundle_version="1.4.0",
            git_revision="rev1",
            tool_versions={},
            overall_status="compatible",
            total_cases=5,
            passed_cases=5,
            failed_cases=0,
        )
        return report, 0

    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        checker=mock_checker,
        poster=lambda url, sec, sub: 200,
    )

    assert exit_code == ExitCode.SUCCESS
    assert sub_file.is_file()
    saved_sub = json.loads(sub_file.read_text(encoding="utf-8"))
    assert saved_sub["requestId"] == "req-persist-ok"
    assert saved_sub["packageName"] == "com.test.app"
    assert len(saved_sub["results"]) == 1
    assert saved_sub["results"][0]["status"] == "compatible"
    assert marker_file.is_file()
    assert marker_file.read_text(encoding="utf-8") == "ok"


def test_reconcile_and_test_transport_failure_leaves_submission_without_marker(
    monkeypatch, tmp_path
):
    sub_file = tmp_path / "submission.json"
    marker_file = tmp_path / "callback-marker"
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-persist-fail")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-test")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setenv("COMPATIBILITY_SUBMISSION_PATH", str(sub_file))
    monkeypatch.setenv("COMPATIBILITY_CALLBACK_MARKER", str(marker_file))
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    target_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_target",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="apkeep",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )

    def mock_checker(
        apk_path, mpp_path, expected_package=None, all_patches=False, force=False
    ):
        report = PatchCompatibilityReport(
            artifact_sha256="sha",
            package_name=expected_package,
            version_name="1.0.0",
            version_code=100,
            patch_bundle_version="1.4.0",
            git_revision="rev1",
            tool_versions={},
            overall_status="compatible",
            total_cases=5,
            passed_cases=5,
            failed_cases=0,
        )
        return report, 0

    def failing_poster(url, sec, sub):
        raise urllib.error.URLError("Connection refused")

    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        checker=mock_checker,
        poster=failing_poster,
    )

    assert exit_code == ExitCode.INFRASTRUCTURE_FAILURE
    assert sub_file.is_file()
    saved_sub = json.loads(sub_file.read_text(encoding="utf-8"))
    assert saved_sub["requestId"] == "req-persist-fail"
    assert not marker_file.exists()


def test_reconcile_and_test_non_2xx_poster_leaves_submission_without_marker(
    monkeypatch, tmp_path
):
    sub_file = tmp_path / "submission.json"
    marker_file = tmp_path / "callback-marker"
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-persist-500")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-test")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setenv("COMPATIBILITY_SUBMISSION_PATH", str(sub_file))
    monkeypatch.setenv("COMPATIBILITY_CALLBACK_MARKER", str(marker_file))
    monkeypatch.setattr(
        "apk_lab.compatibility_ci.get_target_version_for_package",
        lambda data, p: "1.0.0",
    )
    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    target_meta = SlotMetadata(
        role="target",
        package_name="com.test.app",
        version_name="1.0.0",
        version_code=100,
        container_type="APKM",
        sha256="sha_target",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="apkeep",
        timestamp="2026-10-06T00:00:00Z",
    )
    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )

    def mock_checker(
        apk_path, mpp_path, expected_package=None, all_patches=False, force=False
    ):
        report = PatchCompatibilityReport(
            artifact_sha256="sha",
            package_name=expected_package,
            version_name="1.0.0",
            version_code=100,
            patch_bundle_version="1.4.0",
            git_revision="rev1",
            tool_versions={},
            overall_status="compatible",
            total_cases=5,
            passed_cases=5,
            failed_cases=0,
        )
        return report, 0

    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        checker=mock_checker,
        poster=lambda url, sec, sub: 500,
    )

    assert exit_code == ExitCode.INFRASTRUCTURE_FAILURE
    assert sub_file.is_file()
    saved_sub = json.loads(sub_file.read_text(encoding="utf-8"))
    assert saved_sub["requestId"] == "req-persist-500"
    assert not marker_file.exists()
