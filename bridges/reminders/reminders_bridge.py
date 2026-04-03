#!/usr/bin/env python3
"""Write iCloud Reminders to a JSON bridge file for VZGLYD."""

from __future__ import annotations

import argparse
import base64
import gzip
import json
import os
import signal
import sys
import time
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Optional
from zoneinfo import ZoneInfo

DEFAULT_OUTPUT = Path(
    os.environ.get("VZGLYD_REMINDERS_PATH", "/tmp/VRX-64-reminders/reminders.json")
)
DEFAULT_SESSION_DIR = Path.home() / ".cache" / "vzglyd" / "icloud_session"
DEFAULT_INTERVAL = 900
LOCAL_TZ = "Australia/Melbourne"


def _decode_title(b64_data: str) -> str:
    if not b64_data:
        return ""
    try:
        raw = base64.b64decode(b64_data)
        decoded = gzip.decompress(raw)
    except Exception:
        return ""

    pos = 0
    while pos < len(decoded) - 2:
        if decoded[pos] == 0x12:
            length = decoded[pos + 1]
            if 1 <= length <= 512 and pos + 2 + length <= len(decoded):
                chunk = decoded[pos + 2 : pos + 2 + length]
                try:
                    text = chunk.decode("utf-8")
                    if all(ord(ch) >= 32 or ch in "\n\r\t" for ch in text):
                        return text
                except UnicodeDecodeError:
                    pass
        pos += 1
    return ""


def _map_priority(raw: object) -> str:
    try:
        priority = int(str(raw))
    except (TypeError, ValueError):
        return "normal"
    if priority == 0:
        return "normal"
    if priority <= 4:
        return "high"
    if priority == 5:
        return "normal"
    return "low"


def _ts_to_date(ts_ms: object, local_tz: str) -> str:
    if ts_ms is None:
        return ""
    try:
        dt = datetime.fromtimestamp(float(ts_ms) / 1000, tz=timezone.utc)
        return dt.astimezone(ZoneInfo(local_tz)).date().isoformat()
    except Exception:
        return ""


def _get_api(email: str, password: str, session_dir: Path):
    from pyicloud import PyiCloudService

    session_dir.mkdir(parents=True, exist_ok=True)
    return PyiCloudService(email, password, cookie_directory=str(session_dir))


def fetch_reminders(
    api,
    target_lists: Optional[list[str]] = None,
    include_completed: bool = False,
    include_past: bool = False,
    local_tz: str = LOCAL_TZ,
) -> list[dict]:
    ck_url = api.data["webservices"]["ckdatabasews"]["url"]
    params = dict(api.params)

    response = api.session.post(
        f"{ck_url}/database/1/com.apple.reminders/production/private/records/changes",
        params=params,
        json={"zoneID": {"zoneName": "Reminders"}},
    )
    response.raise_for_status()
    records = response.json().get("records", [])

    list_names: dict[str, str] = {}
    for record in records:
        if record.get("recordType") != "List" or record.get("deleted"):
            continue
        fields = record.get("fields", {})
        list_names[record["recordName"]] = (
            _decode_title(fields.get("TitleDocument", {}).get("value", "")) or "Reminders"
        )

    rows: list[dict] = []
    today_str = date.today().isoformat()
    for record in records:
        if record.get("recordType") != "Reminder" or record.get("deleted"):
            continue

        fields = record.get("fields", {})
        if fields.get("Deleted", {}).get("value", 0):
            continue

        completed = bool(fields.get("Completed", {}).get("value", 0))
        status = "done" if completed else "pending"
        if status == "done" and not include_completed:
            continue

        title = _decode_title(fields.get("TitleDocument", {}).get("value", ""))
        if not title:
            continue

        due = _ts_to_date(fields.get("DueDate", {}).get("value"), local_tz)
        if not include_past and status == "pending" and due and due < today_str:
            continue

        list_ref = fields.get("List", {}).get("value", {})
        list_record_name = list_ref.get("recordName", "") if isinstance(list_ref, dict) else ""
        list_name = list_names.get(list_record_name, "Reminders")
        if target_lists and list_name not in target_lists:
            continue

        rows.append(
            {
                "title": title,
                "due": due,
                "priority": _map_priority(fields.get("Priority", {}).get("value", 0)),
                "list": list_name,
                "status": status,
            }
        )

    rows.sort(key=lambda row: (0 if row["status"] == "pending" else 1, row["due"] or "9999-99-99", row["title"]))
    return rows


def write_json(path: Path, reminders: list[dict], fetched_at: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {"fetched_at": fetched_at, "reminders": reminders}
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    os.replace(tmp, path)


def setup_2fa(email: str, password: str, session_dir: Path) -> None:
    api = _get_api(email, password, session_dir)
    if not getattr(api, "requires_2fa", False) and not getattr(api, "requires_2sa", False):
        print("Session already valid.")
        return

    print("2FA required. Check your trusted Apple device for the verification code.", flush=True)
    code = input("Enter the 6-digit code: ").strip()
    if api.validate_2fa_code(code):
        print(f"2FA verified. Session saved to {session_dir}", flush=True)
        return

    print("2FA verification failed.", flush=True)
    sys.exit(1)


def run(
    email: str,
    password: str,
    session_dir: Path,
    output: Path,
    target_lists: Optional[list[str]],
    include_completed: bool,
    include_past: bool,
    daemon: bool,
    interval: int,
    local_tz: str,
) -> None:
    from pyicloud.exceptions import PyiCloudFailedLoginException

    stopping = False

    def _handle_signal(_signum, _frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, _handle_signal)
    signal.signal(signal.SIGINT, _handle_signal)

    try:
        api = _get_api(email, password, session_dir)
    except PyiCloudFailedLoginException as exc:
        print(f"reminders-bridge: login failed: {exc}", flush=True)
        print("Run with --setup to complete the one-time 2FA flow.", flush=True)
        sys.exit(1)

    if getattr(api, "requires_2fa", False) or getattr(api, "requires_2sa", False):
        print("reminders-bridge: 2FA is still required. Run with --setup first.", flush=True)
        sys.exit(1)

    while not stopping:
        fetched_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        print(f"reminders-bridge: fetching at {fetched_at} ...", flush=True)
        rows = fetch_reminders(
            api,
            target_lists=target_lists,
            include_completed=include_completed,
            include_past=include_past,
            local_tz=local_tz,
        )
        write_json(output, rows, fetched_at)
        print(f"reminders-bridge: wrote {len(rows)} reminders to {output}", flush=True)

        if not daemon:
            return
        for _ in range(interval):
            if stopping:
                return
            time.sleep(1)


def main() -> None:
    parser = argparse.ArgumentParser(description="Write iCloud Reminders to a VZGLYD JSON bridge file")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--session-dir", type=Path, default=DEFAULT_SESSION_DIR)
    parser.add_argument("--daemon", action="store_true")
    parser.add_argument("--interval", type=int, default=DEFAULT_INTERVAL)
    parser.add_argument("--setup", action="store_true")
    parser.add_argument("--all", dest="include_completed", action="store_true")
    parser.add_argument("--include-past", action="store_true")
    parser.add_argument("--list", dest="target_lists", action="append")
    parser.add_argument("--local-tz", default=LOCAL_TZ)
    args = parser.parse_args()

    email = os.environ.get("ICLOUD_EMAIL", "").strip()
    password = os.environ.get("ICLOUD_PASSWORD", "").strip()
    if not email or not password:
        print("Set ICLOUD_EMAIL and ICLOUD_PASSWORD before running the bridge.", file=sys.stderr)
        sys.exit(1)

    if args.setup:
        setup_2fa(email, password, args.session_dir)
        return

    run(
        email=email,
        password=password,
        session_dir=args.session_dir,
        output=args.output,
        target_lists=args.target_lists,
        include_completed=args.include_completed,
        include_past=args.include_past,
        daemon=args.daemon,
        interval=args.interval,
        local_tz=args.local_tz,
    )


if __name__ == "__main__":
    main()
