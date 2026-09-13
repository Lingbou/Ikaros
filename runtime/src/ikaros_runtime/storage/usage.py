"""Read-only aggregation for exact Provider-reported model usage."""

from __future__ import annotations

import sqlite3
from collections import defaultdict
from datetime import date, datetime, timedelta

from ..domain import DailyUsageBucket, UsageSnapshot, UsageSummary
from ..json_codec import MAX_SAFE_INTEGER


def read_usage(connection: sqlite3.Connection) -> UsageSnapshot:
    rows = connection.execute(
        """
        SELECT activity_date, total_tokens
        FROM model_usages
        ORDER BY completed_at ASC, run_id ASC, call_ordinal ASC
        """
    ).fetchall()

    daily_tokens: defaultdict[str, int] = defaultdict(int)
    lifetime_tokens: int | None = None
    for row in rows:
        start_date = str(row["activity_date"])
        _parse_activity_date(start_date)
        tokens = int(row["total_tokens"])
        daily_tokens[start_date] = _safe_add(daily_tokens[start_date], tokens)
        lifetime_tokens = _safe_add(lifetime_tokens or 0, tokens)

    peak_daily_tokens = max(daily_tokens.values()) if daily_tokens else None
    active_dates = sorted(
        _parse_activity_date(start_date)
        for start_date, tokens in daily_tokens.items()
        if tokens > 0
    )
    current_streak_days, longest_streak_days = _streaks(
        active_dates,
        today=datetime.now().astimezone().date(),
    )

    longest_running_turn_sec: int | None = None
    for row in connection.execute(
        """
        SELECT started_at, settled_at
        FROM runs
        WHERE started_at IS NOT NULL
          AND settled_at IS NOT NULL
          AND (reason_code IS NULL OR reason_code <> 'runtime_interrupted')
        """
    ):
        started = _parse_timestamp(str(row["started_at"]))
        settled = _parse_timestamp(str(row["settled_at"]))
        duration = int((settled - started).total_seconds())
        if duration < 0:
            raise RuntimeError("Run settlement precedes its start time")
        if duration > MAX_SAFE_INTEGER:
            raise RuntimeError("Run duration exceeds the supported numeric range")
        if longest_running_turn_sec is None or duration > longest_running_turn_sec:
            longest_running_turn_sec = duration

    return UsageSnapshot(
        summary=UsageSummary(
            lifetime_tokens=lifetime_tokens,
            peak_daily_tokens=peak_daily_tokens,
            longest_running_turn_sec=longest_running_turn_sec,
            current_streak_days=current_streak_days,
            longest_streak_days=longest_streak_days,
        ),
        daily_usage_buckets=tuple(
            DailyUsageBucket(start_date=start_date, tokens=tokens)
            for start_date, tokens in sorted(daily_tokens.items())
        ),
    )


def _parse_timestamp(value: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise RuntimeError("Runtime state contains an invalid timestamp") from None
    if parsed.tzinfo is None:
        raise RuntimeError("Runtime state contains a timestamp without a timezone")
    return parsed


def local_activity_date(timestamp: str) -> str:
    return _parse_timestamp(timestamp).astimezone().date().isoformat()


def _parse_activity_date(value: str) -> date:
    try:
        parsed = date.fromisoformat(value)
    except ValueError:
        raise RuntimeError("Runtime state contains an invalid activity date") from None
    if parsed.isoformat() != value:
        raise RuntimeError("Runtime state contains a non-canonical activity date")
    return parsed


def _safe_add(left: int, right: int) -> int:
    result = left + right
    if result > MAX_SAFE_INTEGER:
        raise RuntimeError("Token usage exceeds the supported numeric range")
    return result


def _streaks(active_dates: list[date], *, today: date) -> tuple[int, int]:
    if not active_dates:
        return 0, 0

    longest = 1
    current_run = 1
    for previous, current in zip(active_dates, active_dates[1:], strict=False):
        if current == previous + timedelta(days=1):
            current_run += 1
            longest = max(longest, current_run)
        else:
            current_run = 1

    active = set(active_dates)
    current_streak = 0
    cursor = today if today in active else today - timedelta(days=1)
    while cursor in active:
        current_streak += 1
        cursor -= timedelta(days=1)
    return current_streak, longest


__all__ = ["local_activity_date", "read_usage"]
