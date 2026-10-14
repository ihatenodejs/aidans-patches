from __future__ import annotations

import stat

import pytest
from apk_lab.google_play_accounts import (
    GooglePlayAccount,
    GooglePlayAccountError,
    GooglePlayAccountsFile,
    check_accounts_file_permissions,
    load_google_play_accounts,
    save_google_play_accounts,
)

VALID_PAYLOAD = {
    "version": 1,
    "accounts": [
        {
            "label": "personal",
            "email": "user@example.com",
            "aasToken": "aas_token_123",
            "device": "px_9a",
            "locale": "en_US",
            "timezone": "UTC",
            "enabled": True,
        },
        {
            "label": "backup",
            "email": "backup@example.com",
            "aasToken": "aas_token_456",
            "device": "px_8",
            "locale": "en_US",
            "timezone": "UTC",
            "enabled": False,
        },
    ],
}


def test_valid_schema_and_ordering():
    accs = GooglePlayAccountsFile.from_dict(VALID_PAYLOAD)
    assert accs.version == 1
    assert len(accs.accounts) == 2
    assert accs.accounts[0].label == "personal"
    assert accs.accounts[0].email == "user@example.com"
    assert accs.accounts[0].aas_token == "aas_token_123"
    assert accs.accounts[0].enabled is True
    assert accs.accounts[1].label == "backup"
    assert accs.accounts[1].enabled is False


def test_reject_unsupported_version():
    payload = {**VALID_PAYLOAD, "version": 2}
    with pytest.raises(GooglePlayAccountError, match="Unsupported schema version"):
        GooglePlayAccountsFile.from_dict(payload)


def test_reject_duplicate_email():
    payload = {
        "version": 1,
        "accounts": [
            {
                "label": "first",
                "email": "user@example.com",
                "aasToken": "tok1",
            },
            {
                "label": "second",
                "email": "USER@example.com",
                "aasToken": "tok2",
            },
        ],
    }
    with pytest.raises(GooglePlayAccountError, match="Duplicate email"):
        GooglePlayAccountsFile.from_dict(payload)


def test_reject_malformed_email():
    payload = {
        "version": 1,
        "accounts": [
            {
                "label": "bad",
                "email": "not-an-email",
                "aasToken": "tok",
            }
        ],
    }
    with pytest.raises(GooglePlayAccountError, match="malformed"):
        GooglePlayAccountsFile.from_dict(payload)


def test_reject_empty_fields():
    for key in ("label", "email", "aasToken"):
        bad = {
            "label": "ok",
            "email": "u@test.com",
            "aasToken": "tok",
        }
        bad[key] = "   "
        with pytest.raises(GooglePlayAccountError):
            GooglePlayAccount.from_dict(bad)


def test_reject_empty_enabled_account_set():
    payload = {
        "version": 1,
        "accounts": [
            {
                "label": "disabled",
                "email": "user@example.com",
                "aasToken": "tok",
                "enabled": False,
            }
        ],
    }
    with pytest.raises(GooglePlayAccountError, match="No enabled accounts"):
        GooglePlayAccountsFile.from_dict(payload, require_enabled=True)


def test_email_redaction():
    acc = GooglePlayAccount(
        label="p",
        email="john.doe@example.com",
        aas_token="secret",
    )
    assert acc.redacted_email == "jo***@example.com"


def test_save_atomic_and_permissions(tmp_path):
    target = tmp_path / "subdir" / "accounts.json"
    accs = GooglePlayAccountsFile.from_dict(VALID_PAYLOAD)
    saved = save_google_play_accounts(accs, target_path=target)

    assert saved.exists()
    assert oct(stat.S_IMODE(saved.stat().st_mode)) == oct(0o600)
    assert oct(stat.S_IMODE(saved.parent.stat().st_mode)) == oct(0o700)

    # Check reading
    loaded = load_google_play_accounts(file_path=saved)
    assert loaded is not None
    assert len(loaded.accounts) == 2


def test_refuse_group_world_readable(tmp_path):
    f = tmp_path / "accounts.json"
    f.write_text("{}", encoding="utf-8")
    f.chmod(0o644)

    with pytest.raises(GooglePlayAccountError, match="insecure permissions"):
        check_accounts_file_permissions(f)


def test_refuse_symlink(tmp_path):
    real_f = tmp_path / "real.json"
    real_f.write_text("{}", encoding="utf-8")
    real_f.chmod(0o600)

    symlink_f = tmp_path / "symlink.json"
    symlink_f.symlink_to(real_f)

    with pytest.raises(GooglePlayAccountError, match="symlink"):
        check_accounts_file_permissions(symlink_f)


def test_load_from_apkeep_accounts_env(monkeypatch):
    import json

    data = json.dumps(VALID_PAYLOAD)
    monkeypatch.setenv("APKEEP_ACCOUNTS", data)

    loaded = load_google_play_accounts()
    assert loaded is not None
    assert loaded.accounts[0].label == "personal"
