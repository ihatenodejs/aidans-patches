from __future__ import annotations

import subprocess
import zipfile
from pathlib import Path

import pytest
from apk_lab.archives import ArchiveSecurityError, MaterializedApk
from apk_lab.models import ResourceMatch
from apk_lab.resources import (
    derive_split_dir_name,
    extract_matched_resources,
    normalize_resource_query,
    parse_aapt2_resource_dump,
    query_apk_resources,
)


def test_normalize_resource_query_valid():
    # Hex ID
    q_hex = normalize_resource_query("0x7f08028e")
    assert q_hex.kind == "id"
    assert q_hex.resource_id == "0x7f08028e"

    q_hex2 = normalize_resource_query("7f08028e")
    assert q_hex2.kind == "id"
    assert q_hex2.resource_id == "0x7f08028e"

    # Type / Name
    q_tn = normalize_resource_query("@drawable/map_preview")
    assert q_tn.kind == "type_name"
    assert q_tn.resource_type == "drawable"
    assert q_tn.resource_name == "map_preview"

    q_id = normalize_resource_query("@+id/submit_button")
    assert q_id.kind == "type_name"
    assert q_id.resource_type == "id"
    assert q_id.resource_name == "submit_button"

    q_pkg = normalize_resource_query("com.test:string/app_name")
    assert q_pkg.kind == "type_name"
    assert q_pkg.package_name == "com.test"
    assert q_pkg.resource_type == "string"
    assert q_pkg.resource_name == "app_name"

    # Direct paths
    q_res = normalize_resource_query("res/drawable-xxhdpi/icon.png")
    assert q_res.kind == "path"
    assert q_res.path == "res/drawable-xxhdpi/icon.png"

    q_asset = normalize_resource_query("assets/index.android.bundle")
    assert q_asset.kind == "path"
    assert q_asset.path == "assets/index.android.bundle"

    q_lib = normalize_resource_query("lib/arm64-v8a/libsig.so")
    assert q_lib.kind == "path"

    q_manifest = normalize_resource_query("AndroidManifest.xml")
    assert q_manifest.kind == "path"

    # Bare name
    q_bare = normalize_resource_query("map_preview_trails")
    assert q_bare.kind == "bare_name"
    assert q_bare.resource_name == "map_preview_trails"


def test_normalize_resource_query_invalid():
    with pytest.raises(ValueError, match="empty"):
        normalize_resource_query("   ")

    with pytest.raises(ValueError, match="control characters"):
        normalize_resource_query("res/icon\x00.png")

    with pytest.raises(ValueError, match="path traversal"):
        normalize_resource_query("../escaped.png")

    with pytest.raises(ValueError, match="absolute path"):
        normalize_resource_query("/etc/passwd")


def test_parse_aapt2_resource_dump():
    dump_text = """
Package name=com.example.app id=7f
  type string id=01 entryCount=2
    resource 0x7f010001 string/app_name
      () "My App"
      (fr) "Mon App"
  type drawable id=02 entryCount=1
    resource 0x7f08028e drawable/map_preview_trails
      (xxhdpi) (file) res/drawable-xxhdpi/map_preview_trails.webp type=webp
      (file) res/drawable/map_preview_trails.png
"""
    entries = parse_aapt2_resource_dump(
        dump_text=dump_text,
        split_name="base",
        container_member="splits/base.apk",
    )
    assert len(entries) == 4

    # String scalar
    e0 = entries[0]
    assert e0["resource_id"] == "0x7f010001"
    assert e0["resource_type"] == "string"
    assert e0["resource_name"] == "app_name"
    assert e0["qualifier"] is None
    assert e0["value"] == '"My App"'
    assert e0["path"] is None

    # String fr
    e1 = entries[1]
    assert e1["qualifier"] == "fr"
    assert e1["value"] == '"Mon App"'

    # Drawable file xxhdpi
    e2 = entries[2]
    assert e2["resource_id"] == "0x7f08028e"
    assert e2["qualifier"] == "xxhdpi"
    assert e2["path"] == "res/drawable-xxhdpi/map_preview_trails.webp"

    # Drawable file default
    e3 = entries[3]
    assert e3["qualifier"] is None
    assert e3["path"] == "res/drawable/map_preview_trails.png"


def test_query_apk_resources_cross_split_and_duplicate_status(tmp_path, monkeypatch):
    base_apk = tmp_path / "base.apk"
    with zipfile.ZipFile(base_apk, "w") as zf:
        zf.writestr("AndroidManifest.xml", b"<manifest/>")
        zf.writestr("res/drawable/icon.png", b"icon_v1")
        zf.writestr("assets/bundle.js", b"console.log()")

    split_apk = tmp_path / "split_config.xxhdpi.apk"
    with zipfile.ZipFile(split_apk, "w") as zf:
        zf.writestr("res/drawable-xxhdpi/icon.png", b"icon_xxhdpi")

    mat_base = MaterializedApk(
        path=base_apk,
        container_member="splits/base.apk",
        split_name="base",
        is_base=True,
    )
    mat_split = MaterializedApk(
        path=split_apk,
        container_member="splits/split_config.xxhdpi.apk",
        split_name="split_config.xxhdpi",
        is_base=False,
    )

    # Mock aapt2 dump resources
    def fake_aapt2(cmd, **kwargs):
        apk_arg = cmd[-1]
        if "base" in apk_arg:
            out = """
Package name=com.example.app id=7f
  type drawable id=01
    resource 0x7f080001 drawable/icon
      () (file) res/drawable/icon.png
"""
        else:
            out = """
Package name=com.example.app id=7f
  type drawable id=01
    resource 0x7f080001 drawable/icon
      (xxhdpi) (file) res/drawable-xxhdpi/icon.png
"""
        return subprocess.CompletedProcess(cmd, 0, out, "")

    monkeypatch.setattr(subprocess, "run", fake_aapt2)
    monkeypatch.setattr(
        "apk_lab.resources.find_build_tools_bin", lambda name: Path("/bin/aapt2")
    )

    query = normalize_resource_query("0x7f080001")
    matches = query_apk_resources([mat_base, mat_split], query)

    assert len(matches) == 2
    # Check Morphe modes
    for m in matches:
        assert m.morphe_mode == "resourcePatch"
        assert m.kind == "resource"

    # Query direct asset
    q_asset = normalize_resource_query("assets/bundle.js")
    asset_matches = query_apk_resources([mat_base, mat_split], q_asset)
    assert len(asset_matches) == 1
    assert asset_matches[0].kind == "asset"
    assert asset_matches[0].morphe_mode == "rawResourcePatch"

    # Query manifest
    q_manifest = normalize_resource_query("AndroidManifest.xml")
    man_matches = query_apk_resources([mat_base, mat_split], q_manifest)
    assert len(man_matches) == 1
    assert man_matches[0].kind == "manifest"
    assert man_matches[0].morphe_mode == "resourcePatch"


def test_derive_split_dir_name():
    d1 = derive_split_dir_name("base", None)
    assert d1 == "base-base"

    d2 = derive_split_dir_name("split_config.xxhdpi", "splits/split_config.xxhdpi.apk")
    assert d2.startswith("split_config.xxhdpi-")
    assert len(d2.split("-")[-1]) == 8


def test_extract_matched_resources_preflight_and_safety(tmp_path):
    apk = tmp_path / "test.apk"
    with zipfile.ZipFile(apk, "w") as zf:
        zf.writestr("res/drawable/test.png", b"test_png_bytes")

    mat = MaterializedApk(
        path=apk, container_member="splits/base.apk", split_name="base", is_base=True
    )

    match_file = ResourceMatch(
        container_member="splits/base.apk",
        split_name="base",
        package_name="com.test",
        resource_id="0x7f080001",
        resource_type="drawable",
        resource_name="test",
        qualifier=None,
        value=None,
        path="res/drawable/test.png",
        kind="resource",
        morphe_mode="resourcePatch",
        sha256="abcd",
        duplicate_status="unique",
    )
    match_scalar = ResourceMatch(
        container_member="splits/base.apk",
        split_name="base",
        package_name="com.test",
        resource_id="0x7f010001",
        resource_type="string",
        resource_name="hello",
        qualifier=None,
        value="Hello",
        path=None,
        kind="resource",
        morphe_mode="resourcePatch",
        sha256=None,
        duplicate_status="unique",
    )

    dest_dir = tmp_path / "out_res"
    extracted, warnings = extract_matched_resources(
        extract_dir=dest_dir,
        materialized_apks=[mat],
        matches=[match_file, match_scalar],
    )

    assert len(extracted) == 1
    extracted_path = Path(extracted[0])
    assert extracted_path.is_file()
    assert extracted_path.read_bytes() == b"test_png_bytes"
    assert len(warnings) == 1
    assert "Skipped extracting 1 scalar" in warnings[0]

    # Pre-existing destination fails preflight
    with pytest.raises(ArchiveSecurityError, match="already exists"):
        extract_matched_resources(
            extract_dir=dest_dir,
            materialized_apks=[mat],
            matches=[match_file],
        )
