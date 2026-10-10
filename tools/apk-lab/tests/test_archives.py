import io
import warnings
import zipfile

import pytest
from apk_lab.archives import (
    ArchiveSecurityError,
    inspect_safe_zip,
    materialize_artifact_apks,
    safe_extract_all,
)
from apk_lab.models import ArtifactInspection, ContainerType, SplitInfo


def create_in_memory_zip(entries: dict[str, bytes]) -> io.BytesIO:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)
    buf.seek(0)
    return buf


def test_safe_zip_normal():
    buf = create_in_memory_zip(
        {
            "AndroidManifest.xml": b"<manifest></manifest>",
            "classes.dex": b"dex\n035\x00" + b"\x00" * 100,
        }
    )
    report = inspect_safe_zip(buf)
    assert report.container_type == ContainerType.APK
    assert report.entry_count == 2
    assert report.total_uncompressed_bytes > 0


def test_safe_zip_traversal_rejected(tmp_path):
    bad_zip = tmp_path / "bad.zip"
    with zipfile.ZipFile(bad_zip, "w") as zf:
        zf.writestr("../escaped.txt", b"evil")

    with pytest.raises(ArchiveSecurityError, match="Path traversal detected"):
        inspect_safe_zip(bad_zip)


def test_safe_zip_absolute_path_rejected(tmp_path):
    bad_zip = tmp_path / "bad_abs.zip"
    with zipfile.ZipFile(bad_zip, "w") as zf:
        zf.writestr("/absolute/path.txt", b"evil")

    with pytest.raises(ArchiveSecurityError, match="Absolute path in archive"):
        inspect_safe_zip(bad_zip)


def test_safe_zip_duplicate_rejected(tmp_path):
    bad_zip = tmp_path / "dup.zip"
    # Write duplicate entries using lower-level zipfile
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)
        with zipfile.ZipFile(bad_zip, "w") as zf:
            zf.writestr("file.txt", b"one")
            zf.writestr("file.txt", b"two")

    with pytest.raises(ArchiveSecurityError, match="Duplicate entry in archive"):
        inspect_safe_zip(bad_zip)


def test_container_classification_apkm():
    buf = create_in_memory_zip(
        {
            "base.apk": b"PK\x03\x04...",
            "split_config.arm64_v8a.apk": b"PK\x03\x04...",
        }
    )
    report = inspect_safe_zip(buf)
    assert report.container_type == ContainerType.APKM
    assert len(report.apk_members) == 2


def test_container_classification_xapk():
    buf = create_in_memory_zip(
        {
            "manifest.json": b"{}",
            "com.app.apk": b"PK\x03\x04...",
        }
    )
    report = inspect_safe_zip(buf)
    assert report.container_type == ContainerType.XAPK


def test_container_classification_apks():
    buf = create_in_memory_zip(
        {
            "toc.pb": b"\x08\x01",
            "base-master.apk": b"PK\x03\x04...",
        }
    )
    report = inspect_safe_zip(buf)
    assert report.container_type == ContainerType.APKS


def test_safe_extract_all_success(tmp_path):
    zip_path = tmp_path / "test.zip"
    with zipfile.ZipFile(zip_path, "w") as zf:
        zf.writestr("a/b.txt", b"hello world")

    dest_dir = tmp_path / "extracted"
    extracted = safe_extract_all(zip_path, dest_dir)
    assert len(extracted) == 1
    assert (dest_dir / "a" / "b.txt").read_text() == "hello world"


def test_container_classification_apk_with_embedded_apk_asset():
    buf = create_in_memory_zip(
        {
            "AndroidManifest.xml": b"manifest_bytes",
            "classes.dex": b"dex_bytes",
            "assets/helper.apk": b"nested_apk_bytes",
        }
    )
    report = inspect_safe_zip(buf)
    assert report.container_type == ContainerType.APK


def test_safe_extract_all_preexisting_symlink_sibling_rejected(tmp_path):
    dest_dir = tmp_path / "dest"
    dest_dir.mkdir()
    sibling_dir = tmp_path / "dest-sibling"
    sibling_dir.mkdir()
    outside_file = sibling_dir / "target.txt"
    outside_file.write_text("outside safe data")

    # Create symlink inside dest_dir that points to outside_file with prefix-matching dest directory name
    symlink_file = dest_dir / "pwn.txt"
    symlink_file.symlink_to(outside_file)

    zip_path = tmp_path / "malicious.zip"
    with zipfile.ZipFile(zip_path, "w") as zf:
        zf.writestr("pwn.txt", b"evil content")

    with pytest.raises(ArchiveSecurityError, match="escapes target directory"):
        safe_extract_all(zip_path, dest_dir)

    # Outside file must remain untouched
    assert outside_file.read_text() == "outside safe data"


def test_materialize_artifact_apks_plain_apk(tmp_path):
    apk_file = tmp_path / "app.apk"
    apk_file.write_bytes(b"PK\x03\x04fake_apk_content")
    inspection = ArtifactInspection(
        file_path=str(apk_file),
        container_type=ContainerType.APK,
        file_size=apk_file.stat().st_size,
        sha256="fake_sha",
        package_name="com.example.app",
        version_name="1.0.0",
        version_code=100,
    )
    with materialize_artifact_apks(apk_file, inspection) as materialized:
        assert len(materialized) == 1
        mat = materialized[0]
        assert mat.path == apk_file.resolve()
        assert mat.container_member is None
        assert mat.split_name == "base"
        assert mat.is_base is True


def test_materialize_artifact_apks_split_container(tmp_path):
    apkm_file = tmp_path / "app.apkm"
    with zipfile.ZipFile(apkm_file, "w") as zf:
        zf.writestr("splits/base.apk", b"base_apk_bytes")
        zf.writestr("splits/config.xxhdpi.apk", b"config_apk_bytes")

    inspection = ArtifactInspection(
        file_path=str(apkm_file),
        container_type=ContainerType.APKM,
        file_size=apkm_file.stat().st_size,
        sha256="container_sha",
        package_name="com.example.app",
        version_name="1.0.0",
        version_code=100,
        splits=[
            SplitInfo(
                filename="splits/base.apk",
                split_name="base",
                sha256="sha1",
                size=14,
                is_base=True,
            ),
            SplitInfo(
                filename="splits/config.xxhdpi.apk",
                split_name="split_config.xxhdpi",
                sha256="sha2",
                size=16,
                is_base=False,
            ),
        ],
    )
    with materialize_artifact_apks(apkm_file, inspection) as materialized:
        assert len(materialized) == 2
        assert materialized[0].split_name == "base"
        assert materialized[0].is_base is True
        assert materialized[0].container_member == "splits/base.apk"
        assert materialized[0].path.is_file()
        assert materialized[0].path.read_bytes() == b"base_apk_bytes"

        assert materialized[1].split_name == "split_config.xxhdpi"
        assert materialized[1].is_base is False
        assert materialized[1].container_member == "splits/config.xxhdpi.apk"
        assert materialized[1].path.is_file()
        assert materialized[1].path.read_bytes() == b"config_apk_bytes"

        temp_parent = materialized[0].path.parent

    # After context exits, temp directory should be cleaned up
    assert not temp_parent.exists()
