from unittest.mock import MagicMock

import pytest
from apk_lab.compatibility_ci import (
    get_target_version_for_package,
    resolve_matrix_packages,
    run_ci_reconcile_and_test,
)
from apk_lab.fixtures import SlotMetadata
from apk_lab.models import ExitCode, PatchCompatibilityReport


def test_resolve_matrix_packages():
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
    assert resolve_matrix_packages(
        data, "repository_dispatch", dispatch_pkg="com.app.b"
    ) == ["com.app.b"]

    # Unknown package must raise ValueError rather than falling back to all
    with pytest.raises(ValueError, match="Unknown package"):
        resolve_matrix_packages(data, "workflow_dispatch", input_pkg="com.unknown.app")
    with pytest.raises(ValueError, match="Unknown package"):
        resolve_matrix_packages(
            data, "repository_dispatch", dispatch_pkg="com.unknown.app"
        )


def test_get_target_version_for_package():
    data = {
        "patches": [
            {
                "compatiblePackages": [
                    {
                        "packageName": "com.app.a",
                        "targets": [{"version": "1.0.0"}, {"version": "1.2.0"}],
                    }
                ]
            }
        ]
    }
    assert get_target_version_for_package(data, "com.app.a") == "1.2.0"
    assert get_target_version_for_package(data, "com.app.b") == ""


def test_reconcile_and_test_uses_updated_target_and_posts_batched(
    monkeypatch, tmp_path
):
    monkeypatch.setenv("DISPATCH_REQUEST_ID", "req-123")
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "secret-xyz")
    monkeypatch.setenv("WORKER_STATUS_URL", "https://worker.test")
    monkeypatch.setenv("APKEEP_EMAIL", "u@test.com")

    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()

    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    mock_r2 = MagicMock()
    # Initial slots: target is None, latest is old
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: None

    # Acquirer returns new artifact
    def mock_acquirer(pkg, out_dir):
        fake_path = out_dir / f"{pkg}.apkm"
        out_dir.mkdir(parents=True, exist_ok=True)
        fake_path.write_bytes(b"fake_latest")
        source = MagicMock()
        source.value = "apkeep"
        return fake_path, source

    # Rotation produces updated latest and target
    latest_meta = SlotMetadata(
        role="latest",
        package_name="com.test.app",
        version_name="2.0.0",
        version_code=200,
        container_type="APKM",
        sha256="sha_latest",
        signer_sha256="sig",
        size_bytes=100,
        acquisition_source="apkeep",
        timestamp="2026-10-06T00:00:00Z",
    )
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
    mock_r2.rotate_slots_on_new_latest.return_value = (latest_meta, target_meta)

    # Checker records role runs
    checked_roles = []

    def mock_checker(
        apk_path, mpp_path, expected_package=None, all_patches=False, force=False
    ):
        role = "latest" if force else "target"
        checked_roles.append(role)
        ver = "2.0.0" if role == "latest" else "1.0.0"
        status = "compatible"
        report = PatchCompatibilityReport(
            artifact_sha256="sha",
            package_name=expected_package,
            version_name=ver,
            version_code=100,
            patch_bundle_version="1.4.0",
            git_revision="rev1",
            tool_versions={},
            overall_status=status,
            total_cases=5,
            passed_cases=5,
            failed_cases=0,
        )
        return report, 0

    # Poster captures the single batched submission
    posted_submissions = []

    def mock_poster(url, secret, submission):
        posted_submissions.append((url, secret, submission))
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
    # Both target and divergent latest checked
    assert checked_roles == ["target", "latest"]
    assert len(posted_submissions) == 1
    url, secret, sub = posted_submissions[0]
    assert url == "https://worker.test"
    assert secret == "secret-xyz"
    assert sub["requestId"] == "req-123"
    assert sub["packageName"] == "com.test.app"
    assert len(sub["results"]) == 2
    assert {r["role"] for r in sub["results"]} == {"target", "latest"}


def test_reconcile_and_test_no_fixture_fails(monkeypatch, tmp_path):
    monkeypatch.delenv("APKEEP_EMAIL", raising=False)
    monkeypatch.delenv("R2_ACCESS_KEY_ID", raising=False)

    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    mock_r2 = MagicMock()
    mock_r2.get_slot_metadata.return_value = None

    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
    )
    # Must exit nonzero because no fixture exists
    assert exit_code == ExitCode.INVALID_ARTIFACT


def test_reconcile_and_test_compatibility_failure_surfaced(monkeypatch, tmp_path):
    monkeypatch.delenv("APKEEP_EMAIL", raising=False)
    monkeypatch.delenv("R2_ACCESS_KEY_ID", raising=False)
    monkeypatch.setenv("COMPATIBILITY_STATUS_SECRET", "sec")

    runner_temp = tmp_path / "runner_temp"
    runner_temp.mkdir()
    mpp_file = tmp_path / "bundle.mpp"
    mpp_file.write_bytes(b"mpp")

    mock_r2 = MagicMock()
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
    mock_r2.get_slot_metadata.side_effect = lambda pkg, role: (
        target_meta if role == "target" else None
    )

    # Checker returns failed compatibility code
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
            failure_reason="One patch failed",
        )
        return report, 1

    mock_poster = MagicMock(return_value=200)

    exit_code = run_ci_reconcile_and_test(
        pkg="com.test.app",
        mpp_path=mpp_file,
        runner_temp=runner_temp,
        r2_mgr=mock_r2,
        checker=mock_checker,
        poster=mock_poster,
    )
    # Must exit nonzero on compatibility failure even though posting succeeded
    assert exit_code != 0
    mock_poster.assert_called_once()
