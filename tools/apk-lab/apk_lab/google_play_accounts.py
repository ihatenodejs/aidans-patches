from __future__ import annotations

import json
import os
import re
import stat
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from apk_lab.models import ExitCode


class GooglePlayAccountError(Exception):
    """Raised when parsing, validating, or accessing Google Play accounts fails."""

    def __init__(
        self, message: str, exit_code: ExitCode = ExitCode.USAGE_OR_TOOL_ERROR
    ):
        super().__init__(message)
        self.exit_code = exit_code


EMAIL_REGEX = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")


@dataclass(frozen=True)
class GooglePlayAccount:
    label: str
    email: str
    aas_token: str
    device: str = "px_9a"
    locale: str = "en_US"
    timezone: str = "UTC"
    enabled: bool = True

    @property
    def redacted_email(self) -> str:
        if "@" not in self.email:
            return "***"
        user, domain = self.email.split("@", 1)
        if len(user) <= 2:
            redacted_user = user[:1] + "***"
        else:
            redacted_user = user[:2] + "***"
        return f"{redacted_user}@{domain}"

    def to_dict(self) -> dict[str, Any]:
        return {
            "label": self.label,
            "email": self.email,
            "aasToken": self.aas_token,
            "device": self.device,
            "locale": self.locale,
            "timezone": self.timezone,
            "enabled": self.enabled,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> GooglePlayAccount:
        if not isinstance(data, dict):
            raise GooglePlayAccountError("Account entry must be an object")

        label = data.get("label")
        email = data.get("email")
        aas_token = data.get("aasToken")
        device = data.get("device", "px_9a")
        locale = data.get("locale", "en_US")
        timezone = data.get("timezone", "UTC")
        enabled = data.get("enabled", True)

        if not isinstance(label, str) or not label.strip():
            raise GooglePlayAccountError("Account label must be a non-empty string")
        if not isinstance(email, str) or not email.strip():
            raise GooglePlayAccountError("Account email must be a non-empty string")
        if not EMAIL_REGEX.match(email.strip()):
            raise GooglePlayAccountError(f"Account email '{email}' is malformed")
        if not isinstance(aas_token, str) or not aas_token.strip():
            raise GooglePlayAccountError("Account aasToken must be a non-empty string")
        if not isinstance(device, str) or not device.strip():
            raise GooglePlayAccountError("Account device must be a non-empty string")
        if not isinstance(locale, str) or not locale.strip():
            raise GooglePlayAccountError("Account locale must be a non-empty string")
        if not isinstance(timezone, str) or not timezone.strip():
            raise GooglePlayAccountError("Account timezone must be a non-empty string")
        if not isinstance(enabled, bool):
            raise GooglePlayAccountError("Account enabled must be a boolean")

        return cls(
            label=label.strip(),
            email=email.strip().lower(),
            aas_token=aas_token.strip(),
            device=device.strip(),
            locale=locale.strip(),
            timezone=timezone.strip(),
            enabled=enabled,
        )


@dataclass(frozen=True)
class GooglePlayAccountsFile:
    version: int
    accounts: tuple[GooglePlayAccount, ...]

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": self.version,
            "accounts": [acc.to_dict() for acc in self.accounts],
        }

    def to_json(self, indent: int | None = None) -> str:
        return json.dumps(
            self.to_dict(),
            separators=(",", ":") if indent is None else None,
            indent=indent,
        )

    @classmethod
    def from_dict(
        cls, data: dict[str, Any], require_enabled: bool = True
    ) -> GooglePlayAccountsFile:
        if not isinstance(data, dict):
            raise GooglePlayAccountError(
                "Google Play accounts payload must be an object"
            )

        version = data.get("version")
        if version != 1:
            raise GooglePlayAccountError(
                f"Unsupported schema version: {version}; only version 1 is supported"
            )

        raw_accounts = data.get("accounts")
        if not isinstance(raw_accounts, list):
            raise GooglePlayAccountError("'accounts' must be a list")

        parsed_accounts: list[GooglePlayAccount] = []
        seen_emails: set[str] = set()

        for item in raw_accounts:
            acc = GooglePlayAccount.from_dict(item)
            if acc.email in seen_emails:
                raise GooglePlayAccountError(
                    f"Duplicate email found in accounts: {acc.email}"
                )
            seen_emails.add(acc.email)
            parsed_accounts.append(acc)

        if require_enabled:
            enabled_accounts = [a for a in parsed_accounts if a.enabled]
            if not enabled_accounts:
                raise GooglePlayAccountError("No enabled accounts configured")

        return cls(version=1, accounts=tuple(parsed_accounts))

    @classmethod
    def from_json(
        cls, json_str: str, require_enabled: bool = True
    ) -> GooglePlayAccountsFile:
        try:
            data = json.loads(json_str)
        except json.JSONDecodeError as e:
            raise GooglePlayAccountError(f"Invalid JSON payload: {e}") from e
        return cls.from_dict(data, require_enabled=require_enabled)


def get_default_accounts_file_path() -> Path:
    override = os.environ.get("APKEEP_ACCOUNTS_FILE")
    if override:
        return Path(override).expanduser().resolve()

    xdg_config_home = os.environ.get("XDG_CONFIG_HOME")
    if xdg_config_home:
        base_dir = Path(xdg_config_home).expanduser().resolve()
    else:
        base_dir = Path.home() / ".config"

    return (base_dir / "aidans-patches" / "google-play-accounts.json").resolve()


def check_accounts_file_permissions(file_path: Path) -> None:
    if file_path.is_symlink():
        raise GooglePlayAccountError(
            f"Accounts file cannot be a symlink: {file_path}",
            exit_code=ExitCode.USAGE_OR_TOOL_ERROR,
        )

    mode = file_path.stat().st_mode
    if mode & (stat.S_IRWXG | stat.S_IRWXO):
        octal_mode = oct(stat.S_IMODE(mode))
        raise GooglePlayAccountError(
            f"Accounts file {file_path} has insecure permissions {octal_mode}; must not be group/world readable (expected 0600)",
            exit_code=ExitCode.USAGE_OR_TOOL_ERROR,
        )


def load_google_play_accounts(
    file_path: Path | None = None,
    require_enabled: bool = True,
) -> GooglePlayAccountsFile | None:
    """Loads accounts from APKEEP_ACCOUNTS env, APKEEP_ACCOUNTS_FILE, or default config path.

    Returns None if no configuration source exists.
    """
    env_content = os.environ.get("APKEEP_ACCOUNTS")
    if env_content and env_content.strip():
        return GooglePlayAccountsFile.from_json(
            env_content.strip(), require_enabled=require_enabled
        )

    target_path = file_path or get_default_accounts_file_path()
    if not target_path.exists():
        return None

    check_accounts_file_permissions(target_path)

    try:
        content = target_path.read_text(encoding="utf-8")
    except Exception as e:
        raise GooglePlayAccountError(
            f"Failed to read accounts file {target_path}: {e}"
        ) from e

    return GooglePlayAccountsFile.from_json(content, require_enabled=require_enabled)


def save_google_play_accounts(
    accounts_file: GooglePlayAccountsFile,
    target_path: Path | None = None,
) -> Path:
    """Saves GooglePlayAccountsFile atomically with mode 0600, creating parent dirs with 0700."""
    dest = target_path or get_default_accounts_file_path()

    if dest.is_symlink():
        raise GooglePlayAccountError(
            f"Cannot write to a symlinked accounts file: {dest}",
            exit_code=ExitCode.USAGE_OR_TOOL_ERROR,
        )

    parent = dest.parent
    parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    # Ensure parent permissions
    try:
        parent.chmod(0o700)
    except OSError:
        pass

    data_str = accounts_file.to_json(indent=2) + "\n"

    # Atomic write via temp file in same directory
    with tempfile.NamedTemporaryFile(
        "w", dir=parent, delete=False, encoding="utf-8"
    ) as tf:
        temp_path = Path(tf.name)
        os.chmod(temp_path, 0o600)
        tf.write(data_str)

    os.chmod(temp_path, 0o600)
    temp_path.replace(dest)
    dest.chmod(0o600)

    return dest
