"""Transaction-neutral helpers for Runtime query projections."""

from __future__ import annotations

import sqlite3
from collections.abc import Sequence
from datetime import date
from typing import Any, cast

from ..domain import (
    JOURNAL_EVENT_SCHEMA_VERSION,
    ContextItem,
    JournalEvent,
    PreparedTurn,
    RunDescriptor,
    WorkspaceSummary,
    skill_descriptors_from_wire,
)
from ..file_changes import MAX_CHANGE_EVENT_BYTES, validate_file_change_record
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from ..run_input import (
    ContextItemRecordV1,
    ContextRevision,
    FrozenMemoryContextV1,
    RunConfig,
    StepInput,
    build_step_input,
    canonical_json,
)
from ..security import (
    contains_protected_value,
    json_contains_protected_value,
    json_values_contain_protected_value,
)
from .context_history import (
    load_context_for_revision,
    load_current_run_context,
    select_context_revision,
)
from .file_changes import file_tool_call, resolve_file_tool_path
from .processes import project_process

_SQLITE_MAX_INTEGER = (1 << 63) - 1


def has_active_runs(connection: sqlite3.Connection) -> bool:
    return (
        connection.execute(
            "SELECT 1 FROM runs WHERE status IN ('queued', 'running') LIMIT 1"
        ).fetchone()
        is not None
    )


def contains_protected_projection_values(
    connection: sqlite3.Connection,
    protected_values: Sequence[str],
) -> bool:
    values = tuple(dict.fromkeys(value for value in protected_values if value))
    if not values:
        return False
    thread_rows = connection.execute(
        "SELECT title, client_request_id, workspace_json FROM threads"
    ).fetchall()
    if any(
        json_contains_protected_value(
            (
                row["title"],
                row["client_request_id"],
                (workspace.id, workspace.name, workspace.root_uri)
                if (workspace := workspace_from_json(row["workspace_json"])) is not None
                else None,
            ),
            values,
        )
        for row in thread_rows
    ):
        return True
    for row in connection.execute(
        "SELECT provider_id, model_id, client_request_id FROM runs"
    ).fetchall():
        references = [row["client_request_id"]]
        references.extend(
            _dynamic_provider_model_references(
                str(row["provider_id"]),
                str(row["model_id"]),
            )
        )
        if json_contains_protected_value(references, values):
            return True
    for row in connection.execute("SELECT kind, content, data_json FROM items").fetchall():
        kind = str(row["kind"])
        content = str(row["content"])
        data = json_loads(row["data_json"])
        if kind == "message":
            if contains_protected_value(content, values) or json_values_contain_protected_value(
                data, values
            ):
                return True
        elif kind == "tool_call":
            dynamic = (
                data.get("callId"),
                data.get("toolName"),
                data.get("arguments"),
                data.get("reasoningContent"),
            )
            if json_contains_protected_value(dynamic, values):
                return True
        elif kind == "tool_result":
            if json_contains_protected_value(
                (data.get("callId"), data.get("toolName")), values
            ) or json_values_contain_protected_value(data.get("result"), values):
                return True
        elif contains_protected_value(content, values) or json_contains_protected_value(
            data, values
        ):
            return True
    for row in connection.execute("SELECT config_json FROM run_configs"):
        config = RunConfig.from_wire(json_loads(str(row["config_json"])))
        if json_contains_protected_value(_run_config_dynamic_values(config), values):
            return True
    for row in connection.execute("SELECT record_json FROM context_revisions"):
        ContextRevision.from_wire(json_loads(str(row["record_json"])))
    for row in connection.execute("SELECT record_json FROM process_sessions"):
        if json_contains_protected_value(json_loads(str(row["record_json"])), values):
            return True
    for row in connection.execute(
        """
        SELECT purpose, input_json, response_model_id, request_id, usage_json
        FROM model_calls
        """
    ).fetchall():
        # Step Manifests and usage contain no user/Provider text: only frozen
        # references, controlled provenance, and numeric counters.
        input_value = json_loads(str(row["input_json"]))
        if row["purpose"] == "execution":
            StepInput.from_wire(input_value)
        elif not isinstance(input_value, dict):
            raise RuntimeError("auxiliary model input is invalid")
        usage_json = row["usage_json"]
        if usage_json is not None:
            json_loads(str(usage_json))
        if json_contains_protected_value(
            (row["response_model_id"], row["request_id"]),
            values,
        ):
            return True
    for row in connection.execute("SELECT record_json FROM file_changes"):
        record = validate_file_change_record(json_loads(row["record_json"]))
        if json_contains_protected_value((record["path"], record.get("diff")), values):
            return True
    return False


def _dynamic_provider_model_references(
    provider_id: str,
    model_id: str,
) -> tuple[str, ...]:
    references: list[str] = []
    if provider_id != "scripted":
        references.append(provider_id)
    if provider_id != "scripted" or model_id != "scripted-v1":
        references.append(model_id)
    return tuple(references)


def _run_config_dynamic_values(frame: RunConfig) -> tuple[object, ...]:
    workspace = frame.workspace
    skill_catalog_content = frame.skill_catalog.content if frame.skill_catalog is not None else None
    return (
        *_dynamic_provider_model_references(frame.provider_id, frame.model_id),
        ((workspace.id, workspace.name, workspace.root_uri) if workspace is not None else None),
        tuple((skill.name, skill.description, skill.location) for skill in frame.skills),
        skill_catalog_content,
    )


def find_turn_by_client_request_id(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    branch_id: str,
    content: str,
    provider_id: str,
    model_id: str,
    client_request_id: str,
) -> PreparedTurn | None:
    existing = connection.execute(
        """
        SELECT r.id AS run_id, r.turn_id, r.provider_id, r.model_id,
               t.thread_id, t.branch_id, i.content
        FROM runs r
        JOIN turns t ON t.id = r.turn_id
        JOIN items i ON i.run_id = r.id AND i.ordinal = 1
        WHERE r.client_request_id = ?
        """,
        (client_request_id,),
    ).fetchone()
    if existing is None:
        return None
    expected = (thread_id, branch_id, provider_id, model_id, content)
    actual = tuple(
        str(existing[name])
        for name in ("thread_id", "branch_id", "provider_id", "model_id", "content")
    )
    if actual != expected:
        raise LookupError("clientRequestId was already used with different turn.start parameters")
    return PreparedTurn(
        turn_id=str(existing["turn_id"]),
        run_id=str(existing["run_id"]),
        thread_id=str(existing["thread_id"]),
        branch_id=str(existing["branch_id"]),
        initial_events=(),
        newly_created=False,
    )


def get_run(connection: sqlite3.Connection, run_id: str) -> RunDescriptor:
    row = connection.execute(
        """
        SELECT r.id, r.turn_id, r.provider_id, r.model_id, r.execution_policy,
               t.thread_id, t.branch_id, th.workspace_json
        FROM runs r
        JOIN turns t ON t.id = r.turn_id
        JOIN threads th ON th.id = t.thread_id
        WHERE r.id = ?
        """,
        (run_id,),
    ).fetchone()
    if row is None:
        raise LookupError("run was not found")
    skills = get_run_config(connection, run_id).skills
    return RunDescriptor(
        id=row["id"],
        turn_id=row["turn_id"],
        thread_id=row["thread_id"],
        branch_id=row["branch_id"],
        provider_id=row["provider_id"],
        model_id=row["model_id"],
        execution_policy=row["execution_policy"],
        workspace=workspace_from_json(row["workspace_json"]),
        skills=skills,
    )


def get_run_config(
    connection: sqlite3.Connection,
    run_id: str,
) -> RunConfig:
    row = connection.execute(
        "SELECT config_json FROM run_configs WHERE run_id = ?",
        (run_id,),
    ).fetchone()
    if row is None:
        raise RuntimeError("Run Submission Frame is unavailable")
    try:
        frame = RunConfig.from_wire(json_loads(str(row["config_json"])))
    except (TypeError, ValueError):
        raise RuntimeError("Run Submission Frame is invalid") from None
    if frame.run_id != run_id:
        raise RuntimeError("Run Submission Frame scope is invalid")
    return frame


def get_context_revision(
    connection: sqlite3.Connection,
    run_id: str,
    revision: int | None = None,
) -> ContextRevision | None:
    row = connection.execute(
        "SELECT record_json FROM context_revisions WHERE run_id = ? "
        "AND (? IS NULL OR revision = ?) ORDER BY revision DESC LIMIT 1",
        (run_id, revision, revision),
    ).fetchone()
    if row is None:
        return None
    try:
        return ContextRevision.from_wire(json_loads(str(row["record_json"])))
    except (TypeError, ValueError):
        raise RuntimeError("Run Context Revision is invalid") from None


def run_status(connection: sqlite3.Connection, run_id: str) -> str:
    row = connection.execute("SELECT status FROM runs WHERE id = ?", (run_id,)).fetchone()
    if row is None:
        raise LookupError("run was not found")
    return str(row["status"])


def context_messages(
    connection: sqlite3.Connection,
    branch_id: str,
    *,
    through_turn_id: str,
) -> list[tuple[str, str]]:
    boundary = connection.execute(
        "SELECT ordinal FROM turns WHERE id = ? AND branch_id = ?",
        (through_turn_id, branch_id),
    ).fetchone()
    if boundary is None:
        raise LookupError("context boundary turn was not found")
    rows = connection.execute(
        """
        SELECT i.role, i.content
        FROM items i
        JOIN turns t ON t.id = i.turn_id
        JOIN runs r ON r.id = i.run_id
        WHERE t.branch_id = ? AND t.ordinal <= ? AND i.kind = 'message'
          AND i.status = 'completed' AND i.role IN ('user', 'assistant')
        ORDER BY t.ordinal, r.created_at, r.id, i.ordinal, i.id
        """,
        (branch_id, int(boundary["ordinal"])),
    ).fetchall()
    return [(row["role"], row["content"]) for row in rows]


def context_items(
    connection: sqlite3.Connection,
    branch_id: str,
    *,
    through_turn_id: str,
) -> list[ContextItem]:
    return [
        record.to_context_item()
        for record in context_item_records(
            connection,
            branch_id,
            through_turn_id=through_turn_id,
        )
    ]


def context_item_records(
    connection: sqlite3.Connection,
    branch_id: str,
    *,
    through_turn_id: str,
) -> list[ContextItemRecordV1]:
    boundary = connection.execute(
        "SELECT ordinal FROM turns WHERE id = ? AND branch_id = ?",
        (through_turn_id, branch_id),
    ).fetchone()
    if boundary is None:
        raise LookupError("context boundary turn was not found")
    rows = connection.execute(
        """
        SELECT i.id, i.turn_id, i.run_id, i.kind, i.role, i.content, i.data_json
        FROM items i JOIN turns t ON t.id = i.turn_id JOIN runs r ON r.id = i.run_id
        WHERE t.branch_id = ? AND t.ordinal <= ? AND (
          (i.kind = 'message' AND i.status = 'completed' AND i.role IN ('user', 'assistant'))
          OR (i.kind IN ('tool_call', 'tool_result') AND i.status IN ('completed', 'failed')
              AND (t.id = ? OR r.status = 'completed'))
        ) ORDER BY t.ordinal, r.created_at, r.id, i.ordinal, i.id
        """,
        (branch_id, int(boundary["ordinal"]), through_turn_id),
    ).fetchall()
    return [
        ContextItemRecordV1(
            item_id=str(row["id"]),
            turn_id=str(row["turn_id"]),
            run_id=str(row["run_id"]),
            kind=str(row["kind"]),
            role=str(row["role"]) if row["role"] is not None else None,
            content=str(row["content"]),
            data=json_loads(row["data_json"]),
        )
        for row in rows
    ]


def context_item_records_for_revision(
    connection: sqlite3.Connection,
    *,
    run_id: str,
    snapshot: ContextRevision,
) -> list[ContextItemRecordV1]:
    return list(load_context_for_revision(connection, run_id=run_id, snapshot=snapshot))


def workspace_to_json(workspace: WorkspaceSummary | None) -> str | None:
    if workspace is None:
        return None
    return json_dumps(workspace.to_wire(), separators=(",", ":"), ensure_ascii=False)


def workspace_from_json(value: object) -> WorkspaceSummary | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise RuntimeError("thread workspace projection is invalid")
    try:
        return workspace_from_wire(json_loads(value))
    except (TypeError, ValueError):
        raise RuntimeError("thread workspace projection is invalid") from None


def workspace_from_wire(value: object) -> WorkspaceSummary | None:
    if value is None:
        return None
    if not isinstance(value, dict) or set(value) != {"id", "name", "rootUri"}:
        raise ValueError("thread workspace projection is invalid")
    workspace_id = value["id"]
    name = value["name"]
    root_uri = value["rootUri"]
    if (
        not isinstance(workspace_id, str)
        or not isinstance(name, str)
        or (root_uri is not None and not isinstance(root_uri, str))
    ):
        raise ValueError("thread workspace projection is invalid")
    return WorkspaceSummary(workspace_id, name, root_uri)


def apply_event(
    connection: sqlite3.Connection,
    event: JournalEvent,
) -> None:
    event_type = event.type
    payload = event.payload
    if event.schema_version != JOURNAL_EVENT_SCHEMA_VERSION:
        raise RuntimeError("journal event schema version is unsupported")
    if event_type == "run.steered":
        _require_keys(
            payload, {"content", "clientRequestId", "status", "item", "turnId", "runId", "itemId"}
        )
        if not isinstance(payload["content"], str) or not payload["content"].strip():
            raise RuntimeError("run.steered content is invalid")
        if not isinstance(payload["clientRequestId"], str) or not payload["clientRequestId"]:
            raise RuntimeError("run.steered clientRequestId is invalid")
        if payload["status"] != "received":
            raise RuntimeError("run.steered status is invalid")
        item = _record(payload, "item", _ITEM_KEYS)
        _validate_item(item)
        if item["kind"] != "message" or item["role"] != "user" or item["status"] != "streaming":
            raise RuntimeError("run.steered item state is invalid")
        if item["createdAt"] != event.timestamp or item["updatedAt"] != event.timestamp:
            raise RuntimeError("run.steered item timestamp is invalid")
        if item["content"] != payload["content"] or item["data"].get("steer") is not True:
            raise RuntimeError("run.steered item payload is invalid")
        if (
            item["data"].get("clientRequestId") != payload["clientRequestId"]
            or item["data"].get("status") != "received"
        ):
            raise RuntimeError("run.steered item metadata is invalid")
        _require_payload_scope(payload, event)
        _require_existing_run_scope(connection, event)
        _require_item_scope(item, event)
        insert_item(connection, item)
        return
    if event_type == "thread.created":
        _require_keys(payload, {"thread", "branch"}, {"clientRequestId"})
        thread = _record(payload, "thread", _THREAD_KEYS)
        branch = _record(payload, "branch", _BRANCH_KEYS)
        _validate_thread(thread)
        _validate_branch(branch)
        _validate_optional_client_request(payload)
        if thread["archivedAt"] is not None or branch["isDefault"] is not True:
            raise RuntimeError("thread.created payload state is invalid")
        _require_equal("thread creation time", thread["createdAt"], event.timestamp)
        _require_equal("thread update time", thread["updatedAt"], event.timestamp)
        _require_equal("branch creation time", branch["createdAt"], event.timestamp)
        _require_event_scope(
            event,
            thread_id=thread["id"],
            branch_id=branch["id"],
            turn_id=None,
            run_id=None,
            item_id=None,
        )
        _require_equal("thread default branch", thread["defaultBranchId"], branch["id"])
        _require_equal("branch thread", branch["threadId"], thread["id"])
        connection.execute(
            """
            INSERT INTO threads(
                id, title, default_branch_id, workspace_json,
                created_at, updated_at, archived_at, client_request_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                thread["id"],
                thread["title"],
                thread["defaultBranchId"],
                workspace_to_json(workspace_from_wire(thread["workspace"])),
                thread["createdAt"],
                thread["updatedAt"],
                thread["archivedAt"],
                payload.get("clientRequestId"),
            ),
        )
        connection.execute(
            "INSERT INTO branches(id, thread_id, created_at, is_default) VALUES (?, ?, ?, ?)",
            (
                branch["id"],
                branch["threadId"],
                branch["createdAt"],
                int(branch["isDefault"]),
            ),
        )
    elif event_type in {"thread.renamed", "thread.archived", "thread.unarchived"}:
        _require_keys(payload, {"thread"})
        thread = _record(payload, "thread", _THREAD_KEYS)
        _validate_thread(thread)
        _require_equal("thread update time", thread["updatedAt"], event.timestamp)
        if event_type == "thread.archived" and thread["archivedAt"] != event.timestamp:
            raise RuntimeError("thread.archived payload state is invalid")
        if event_type == "thread.unarchived" and thread["archivedAt"] is not None:
            raise RuntimeError("thread.unarchived payload state is invalid")
        _require_thread_lifecycle_transition(connection, event_type, thread)
        _require_event_scope(
            event,
            thread_id=thread["id"],
            branch_id=thread["defaultBranchId"],
            turn_id=None,
            run_id=None,
            item_id=None,
        )
        updated = connection.execute(
            """
            UPDATE threads
            SET title = ?, updated_at = ?, archived_at = ?
            WHERE id = ?
            """,
            (
                thread["title"],
                thread["updatedAt"],
                thread["archivedAt"],
                thread["id"],
            ),
        )
        _require_one_update(updated, event_type)
    elif event_type == "item.completed" and "turn" in payload:
        _require_keys(
            payload,
            {
                "turn",
                "run",
                "item",
                "runConfig",
                "turnId",
                "runId",
                "itemId",
            },
            {"clientRequestId"},
        )
        turn = _record(payload, "turn", _TURN_KEYS)
        run = _record(payload, "run", _RUN_KEYS, {"clientRequestId"})
        item = _record(payload, "item", _ITEM_KEYS)
        _validate_turn(turn)
        _validate_run(run)
        _validate_item(item)
        _validate_optional_client_request(payload)
        try:
            run_config = RunConfig.from_wire(payload["runConfig"])
        except (TypeError, ValueError):
            raise RuntimeError("initial Turn input snapshots are invalid") from None
        if (
            turn["status"] != "queued"
            or run["status"] != "queued"
            or run["settledAt"] is not None
            or run["executionPolicy"] != "full_access"
            or item["kind"] != "message"
            or item["role"] != "user"
            or item["status"] != "completed"
            or item["ordinal"] != 1
        ):
            raise RuntimeError("initial Turn payload state is invalid")
        for label, value in (
            ("turn creation time", turn["createdAt"]),
            ("turn update time", turn["updatedAt"]),
            ("run creation time", run["createdAt"]),
            ("item creation time", item["createdAt"]),
            ("item update time", item["updatedAt"]),
        ):
            _require_equal(label, value, event.timestamp)
        _require_event_scope(
            event,
            thread_id=turn["threadId"],
            branch_id=turn["branchId"],
            turn_id=turn["id"],
            run_id=run["id"],
            item_id=item["id"],
        )
        _require_payload_scope(payload, event)
        _require_equal("run turn", run["turnId"], turn["id"])
        _require_equal("item turn", item["turnId"], turn["id"])
        _require_equal("item run", item["runId"], run["id"])
        _require_equal("Frame User Item", run_config.user_item_id, item["id"])
        _require_equal("Frame Thread", run_config.thread_id, turn["threadId"])
        _require_equal("Frame Branch", run_config.branch_id, turn["branchId"])
        _require_equal("Frame Turn", run_config.turn_id, turn["id"])
        _require_equal("Frame Run", run_config.run_id, run["id"])
        _require_equal("Frame Provider", run_config.provider_id, run["providerId"])
        _require_equal("Frame Model", run_config.model_id, run["modelId"])
        _require_equal(
            "Frame execution policy",
            run_config.execution_policy,
            run["executionPolicy"],
        )
        _require_equal(
            "Frame Skills",
            tuple(skill.to_wire() for skill in run_config.skills),
            tuple(skill.to_wire() for skill in skill_descriptors_from_wire(run["skills"])),
        )
        _require_equal(
            "Frame workspace",
            run_config.workspace,
            workspace_from_json(
                connection.execute(
                    "SELECT workspace_json FROM threads WHERE id = ?",
                    (turn["threadId"],),
                ).fetchone()["workspace_json"]
            ),
        )
        _require_initial_thread_scope(connection, event)
        _require_equal(
            "turn client request",
            payload.get("clientRequestId"),
            run.get("clientRequestId"),
        )
        connection.execute(
            """
            INSERT INTO turns(id, thread_id, branch_id, ordinal, status, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            (
                turn["id"],
                turn["threadId"],
                turn["branchId"],
                turn["ordinal"],
                turn["status"],
                turn["createdAt"],
                turn["updatedAt"],
            ),
        )
        connection.execute(
            """
            INSERT INTO runs(
                id, turn_id, provider_id, model_id, execution_policy, status,
                created_at, settled_at, client_request_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                run["id"],
                run["turnId"],
                run["providerId"],
                run["modelId"],
                run["executionPolicy"],
                run["status"],
                run["createdAt"],
                run["settledAt"],
                run.get("clientRequestId"),
            ),
        )
        insert_item(connection, item)
        connection.execute(
            """
            INSERT INTO run_configs(
                run_id, config_json
            ) VALUES (?, ?)
            """,
            (
                run["id"],
                canonical_json(run_config.to_wire()),
            ),
        )
        updated = connection.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            (turn["updatedAt"], turn["threadId"]),
        )
        _require_one_update(updated, event_type)
    elif event_type == "run.state_changed":
        _require_keys(payload, {"status", "turnId", "runId"})
        _require_choice("Run state", payload["status"], {"queued", "running"})
        _require_payload_scope(payload, event, item_required=False)
        _require_existing_run_scope(connection, event)
        state_event_ordinal = int(
            connection.execute(
                """
                SELECT COUNT(*) FROM events
                WHERE run_id = ? AND event_type = 'run.state_changed' AND seq <= ?
                """,
                (event.run_id, event.seq),
            ).fetchone()[0]
        )
        current = connection.execute(
            """
            SELECT r.status AS run_status, r.started_at, r.created_at,
                   t.status AS turn_status, t.updated_at AS turn_updated_at
            FROM runs r JOIN turns t ON t.id = r.turn_id
            WHERE r.id = ?
            """,
            (event.run_id,),
        ).fetchone()
        if current is None:
            raise RuntimeError("Run state event has no existing Run")
        if payload["status"] == "queued":
            if (
                state_event_ordinal != 1
                or current["run_status"] != "queued"
                or current["turn_status"] != "queued"
                or current["started_at"] is not None
                or current["created_at"] != event.timestamp
                or current["turn_updated_at"] != event.timestamp
            ):
                raise RuntimeError("queued Run notification does not match initial state")
            return
        if (
            state_event_ordinal != 2
            or current["run_status"] != "queued"
            or current["turn_status"] != "queued"
            or current["started_at"] is not None
        ):
            raise RuntimeError("Run can only transition once from queued to running")
        updated_run = connection.execute(
            """
            UPDATE runs SET status = 'running', started_at = ?
            WHERE id = ? AND status = 'queued' AND started_at IS NULL
            """,
            (event.timestamp, payload["runId"]),
        )
        _require_one_update(updated_run, event_type)
        updated_turn = connection.execute(
            """
            UPDATE turns SET status = 'running', updated_at = ?
            WHERE id = ? AND status = 'queued'
            """,
            (event.timestamp, payload["turnId"]),
        )
        _require_one_update(updated_turn, event_type)
    elif event_type == "process.recorded":
        _require_keys(payload, {"process", "turnId", "runId", "itemId"})
        _require_payload_scope(payload, event)
        _require_existing_run_scope(connection, event)
        record = payload["process"]
        if (
            not isinstance(record, dict)
            or record.get("runId") != event.run_id
            or record.get("itemId") != event.item_id
            or record.get("threadId") != event.thread_id
        ):
            raise RuntimeError("process event scope is invalid")
        project_process(connection, record)
    elif event_type == "model.input_prepared":
        if payload.get("purpose") in {"compression", "completion_check"}:
            _require_keys(
                payload,
                {"purpose", "callOrdinal", "preparedAt", "input", "turnId", "runId"},
            )
            if not isinstance(payload["input"], dict):
                raise RuntimeError("auxiliary model input is invalid")
            _require_positive_integer("auxiliary model call ordinal", payload["callOrdinal"])
            _require_string("auxiliary model preparedAt", payload["preparedAt"])
            _require_equal(
                "auxiliary model preparation time", payload["preparedAt"], event.timestamp
            )
            _require_payload_scope(payload, event, item_required=False)
            _require_event_scope(
                event,
                thread_id=event.thread_id,
                branch_id=event.branch_id,
                turn_id=event.turn_id,
                run_id=event.run_id,
                item_id=None,
            )
            _require_existing_run_scope(connection, event)
            running = connection.execute(
                "SELECT status FROM runs WHERE id = ?", (event.run_id,)
            ).fetchone()
            if running is None or running["status"] != "running":
                raise RuntimeError("auxiliary model input requires a running Run")
            if (
                connection.execute(
                    "SELECT 1 FROM model_calls WHERE run_id = ? AND outcome IS NULL",
                    (event.run_id,),
                ).fetchone()
                is not None
            ):
                raise RuntimeError("Run already has an unfinished model call")
            expected = int(
                connection.execute(
                    "SELECT COALESCE(MAX(call_ordinal), 0) + 1 FROM model_calls WHERE run_id = ?",
                    (event.run_id,),
                ).fetchone()[0]
            )
            _require_equal("auxiliary model call ordinal", payload["callOrdinal"], expected)
            connection.execute(
                """INSERT INTO model_calls(
                    run_id, call_ordinal, step_ordinal, input_json, prepared_at, purpose
                ) VALUES (?, ?, NULL, ?, ?, ?)""",
                (
                    event.run_id,
                    expected,
                    canonical_json(payload["input"]),
                    payload["preparedAt"],
                    payload["purpose"],
                ),
            )
            return
        _require_keys(
            payload,
            {
                "stepOrdinal",
                "preparedAt",
                "contextRevision",
                "stepInput",
                "turnId",
                "runId",
            },
        )
        _require_positive_integer("model Step ordinal", payload["stepOrdinal"])
        _require_string("model Step preparedAt", payload["preparedAt"])
        _require_equal("model Step preparation time", payload["preparedAt"], event.timestamp)
        _require_payload_scope(payload, event, item_required=False)
        _require_event_scope(
            event,
            thread_id=event.thread_id,
            branch_id=event.branch_id,
            turn_id=payload["turnId"],
            run_id=payload["runId"],
            item_id=None,
        )
        _require_existing_run_scope(connection, event)
        run = connection.execute(
            "SELECT status FROM runs WHERE id = ?",
            (event.run_id,),
        ).fetchone()
        if run is None or run["status"] != "running":
            raise RuntimeError("model input preparation requires a running Run")
        open_step = connection.execute(
            "SELECT 1 FROM model_calls WHERE run_id = ? AND outcome IS NULL",
            (event.run_id,),
        ).fetchone()
        if open_step is not None:
            raise RuntimeError("Run already has an unfinished model Step")
        expected_ordinal = int(
            connection.execute(
                "SELECT COALESCE(MAX(step_ordinal), 0) + 1 FROM model_calls "
                "WHERE run_id = ? AND purpose = 'execution'",
                (event.run_id,),
            ).fetchone()[0]
        )
        _require_equal("model Step ordinal", payload["stepOrdinal"], expected_ordinal)
        try:
            step_input = StepInput.from_wire(payload["stepInput"])
        except (TypeError, ValueError):
            raise RuntimeError("model Step input is invalid") from None
        _require_equal("Step input ordinal", step_input.step_ordinal, expected_ordinal)
        snapshot = get_context_revision(connection, str(event.run_id), step_input.context_revision)
        if snapshot is None:
            try:
                snapshot = ContextRevision.from_wire(payload["contextRevision"])
            except (TypeError, ValueError):
                raise RuntimeError("initial Context Revision is invalid") from None
            config = get_run_config(connection, str(event.run_id))
            expected_snapshot, _records = select_context_revision(
                connection,
                branch_id=str(event.branch_id),
                turn_id=str(event.turn_id),
                run_id=str(event.run_id),
                config=config,
                memory_context=FrozenMemoryContextV1.from_revision(snapshot),
            )
            if snapshot != expected_snapshot or snapshot.revision != step_input.context_revision:
                raise RuntimeError("initial Context Revision is not canonical")
            connection.execute(
                "INSERT INTO context_revisions(run_id, revision, record_json) VALUES (?, ?, ?)",
                (event.run_id, snapshot.revision, canonical_json(snapshot.to_wire())),
            )
        elif payload["contextRevision"] is not None:
            raise RuntimeError("model Step must reference an existing Context Revision")
        records = context_item_records_for_revision(
            connection,
            run_id=str(event.run_id),
            snapshot=snapshot,
        )
        expected_input = build_step_input(
            expected_ordinal,
            records,
            snapshot,
            current_run_id=str(event.run_id),
            config=get_run_config(connection, str(event.run_id)),
        )
        if step_input != expected_input:
            raise RuntimeError("Step Input is not canonical")
        connection.execute(
            """
            INSERT INTO model_calls(
                run_id, call_ordinal, step_ordinal, input_json, prepared_at
            ) VALUES (?, ?, ?, ?, ?)
            """,
            (
                event.run_id,
                int(
                    connection.execute(
                        "SELECT COALESCE(MAX(call_ordinal), 0) + 1 FROM model_calls "
                        "WHERE run_id = ?",
                        (event.run_id,),
                    ).fetchone()[0]
                ),
                expected_ordinal,
                canonical_json(step_input.to_wire()),
                payload["preparedAt"],
            ),
        )
    elif event_type == "model.response_finished":
        if payload.get("purpose") in {"compression", "completion_check"}:
            _require_keys(
                payload,
                {
                    "purpose",
                    "callOrdinal",
                    "providerId",
                    "modelId",
                    "outcome",
                    "reasonCode",
                    "responseModelId",
                    "requestId",
                    "activityDate",
                    "usage",
                    "finishedAt",
                    "turnId",
                    "runId",
                },
            )
            _require_positive_integer("auxiliary model call ordinal", payload["callOrdinal"])
            _require_string("auxiliary model providerId", payload["providerId"])
            _require_string("auxiliary model modelId", payload["modelId"])
            _require_choice(
                "auxiliary model outcome", payload["outcome"], {"completed", "failed", "cancelled"}
            )
            _require_string("auxiliary model finishedAt", payload["finishedAt"])
            _require_equal("auxiliary model finish time", payload["finishedAt"], event.timestamp)
            _require_payload_scope(payload, event, item_required=False)
            _require_existing_run_scope(connection, event)
            run = connection.execute(
                "SELECT provider_id, model_id, status FROM runs WHERE id = ?", (event.run_id,)
            ).fetchone()
            if run is None or run["status"] != "running":
                raise RuntimeError("auxiliary model completion requires a running Run")
            if (run["provider_id"], run["model_id"]) != (payload["providerId"], payload["modelId"]):
                raise RuntimeError("auxiliary model Provider or Model does not match its Run")
            call = connection.execute(
                "SELECT outcome FROM model_calls "
                "WHERE run_id = ? AND call_ordinal = ? AND purpose = ?",
                (event.run_id, payload["callOrdinal"], payload["purpose"]),
            ).fetchone()
            if call is None or call["outcome"] is not None:
                raise RuntimeError("auxiliary model call is not open")
            reason = payload["reasonCode"]
            if payload["outcome"] == "completed" and reason is not None:
                raise RuntimeError("completed auxiliary model call has a failure reason")
            if payload["outcome"] != "completed":
                _require_string("auxiliary model reasonCode", reason)
            usage_value = payload["usage"]
            activity_date = payload["activityDate"]
            auxiliary_usage = None
            if usage_value is not None:
                if payload["outcome"] != "completed":
                    raise RuntimeError("failed auxiliary model call cannot record usage")
                auxiliary_usage = _record(payload, "usage", _MODEL_USAGE_KEYS)
                _validate_model_usage(1, auxiliary_usage)
                _require_activity_date(activity_date)
            elif activity_date is not None:
                raise RuntimeError("auxiliary model activity date has no usage")
            response_model_id = payload["responseModelId"]
            request_id = payload["requestId"]
            _require_safe_response_identifier("response model ID", response_model_id)
            _require_safe_response_identifier("request ID", request_id)
            connection.execute(
                """UPDATE model_calls SET outcome = ?, reason_code = ?, response_model_id = ?,
                    request_id = ?, usage_json = ?, activity_date = ?, finished_at = ?
                    WHERE run_id = ? AND call_ordinal = ? AND purpose = ? AND outcome IS NULL""",
                (
                    payload["outcome"],
                    reason,
                    response_model_id,
                    request_id,
                    canonical_json(auxiliary_usage) if auxiliary_usage is not None else None,
                    activity_date,
                    payload["finishedAt"],
                    event.run_id,
                    payload["callOrdinal"],
                    payload["purpose"],
                ),
            )
            if auxiliary_usage is not None:
                connection.execute(
                    """INSERT INTO model_usages(
                        thread_id, turn_id, run_id, call_ordinal, step_ordinal,
                        provider_id, model_id,
                        input_tokens, cached_input_tokens, output_tokens, reasoning_output_tokens,
                        total_tokens, activity_date, completed_at
                    ) VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                    (
                        event.thread_id,
                        event.turn_id,
                        event.run_id,
                        payload["callOrdinal"],
                        payload["providerId"],
                        payload["modelId"],
                        auxiliary_usage["inputTokens"],
                        auxiliary_usage["cachedInputTokens"],
                        auxiliary_usage["outputTokens"],
                        auxiliary_usage["reasoningOutputTokens"],
                        auxiliary_usage["totalTokens"],
                        activity_date,
                        payload["finishedAt"],
                    ),
                )
            return
        _require_keys(
            payload,
            {
                "stepOrdinal",
                "providerId",
                "modelId",
                "outcome",
                "reasonCode",
                "responseModelId",
                "requestId",
                "activityDate",
                "usage",
                "finishedAt",
                "turnId",
                "runId",
            },
        )
        _require_positive_integer("model Step ordinal", payload["stepOrdinal"])
        _require_string("model response providerId", payload["providerId"])
        _require_string("model response modelId", payload["modelId"])
        _require_choice(
            "model response outcome",
            payload["outcome"],
            {"completed", "failed", "cancelled"},
        )
        _require_string("model response finishedAt", payload["finishedAt"])
        _require_equal("model response finish time", payload["finishedAt"], event.timestamp)
        _require_payload_scope(payload, event, item_required=False)
        _require_existing_run_scope(connection, event)
        run = connection.execute(
            "SELECT provider_id, model_id, status FROM runs WHERE id = ?",
            (event.run_id,),
        ).fetchone()
        if run is None or run["status"] != "running":
            raise RuntimeError("model response completion requires a running Run")
        if (run["provider_id"], run["model_id"]) != (
            payload["providerId"],
            payload["modelId"],
        ):
            raise RuntimeError("model response Provider or Model does not match its Run")
        step = connection.execute(
            """
            SELECT outcome, call_ordinal FROM model_calls
            WHERE run_id = ? AND step_ordinal = ?
            """,
            (event.run_id, payload["stepOrdinal"]),
        ).fetchone()
        if step is None:
            raise RuntimeError("model response has no prepared Step")
        if step["outcome"] is not None:
            raise RuntimeError("model response Step is already finished")
        outcome = str(payload["outcome"])
        reason_code = payload.get("reasonCode")
        if outcome == "completed":
            if reason_code is not None:
                raise RuntimeError("completed model response has a failure reason")
        else:
            _require_string("model response reasonCode", reason_code)
        response_model_id = payload.get("responseModelId")
        request_id = payload.get("requestId")
        _require_safe_response_identifier("response model ID", response_model_id)
        _require_safe_response_identifier("request ID", request_id)
        usage_value = payload.get("usage")
        activity_date = payload.get("activityDate")
        usage: dict[str, Any] | None = None
        if usage_value is not None:
            if outcome != "completed":
                raise RuntimeError("failed model response cannot record Token usage")
            usage = _record(payload, "usage", _MODEL_USAGE_KEYS)
            _validate_model_usage(payload["stepOrdinal"], usage)
            _require_activity_date(activity_date)
        elif activity_date is not None:
            raise RuntimeError("model response activity date has no Token usage")
        updated = connection.execute(
            """
            UPDATE model_calls
            SET outcome = ?, reason_code = ?, response_model_id = ?, request_id = ?,
                usage_json = ?, activity_date = ?, finished_at = ?
            WHERE run_id = ? AND call_ordinal = ? AND purpose = 'execution' AND outcome IS NULL
            """,
            (
                outcome,
                reason_code,
                response_model_id,
                request_id,
                canonical_json(usage) if usage is not None else None,
                activity_date,
                payload["finishedAt"],
                event.run_id,
                step["call_ordinal"],
            ),
        )
        _require_one_update(updated, event_type)
        if usage is not None:
            connection.execute(
                """
                INSERT INTO model_usages(
                    thread_id, turn_id, run_id, call_ordinal, step_ordinal, provider_id, model_id,
                    input_tokens, cached_input_tokens, output_tokens,
                    reasoning_output_tokens, total_tokens, activity_date, completed_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    event.thread_id,
                    event.turn_id,
                    event.run_id,
                    step["call_ordinal"],
                    payload["stepOrdinal"],
                    payload["providerId"],
                    payload["modelId"],
                    usage["inputTokens"],
                    usage["cachedInputTokens"],
                    usage["outputTokens"],
                    usage["reasoningOutputTokens"],
                    usage["totalTokens"],
                    activity_date,
                    payload["finishedAt"],
                ),
            )
    elif event_type == "item.started":
        _require_keys(payload, {"item", "turnId", "runId", "itemId"})
        item = _record(payload, "item", _ITEM_KEYS)
        _validate_item(item)
        if not (
            (
                item["kind"] == "message"
                and item["role"] == "assistant"
                and item["status"] == "streaming"
            )
            or (
                item["kind"] == "tool_call"
                and item["role"] == "assistant"
                and item["status"] == "running"
            )
        ):
            raise RuntimeError("item.started payload state is invalid")
        if item["content"] != "":
            raise RuntimeError("item.started content must be empty")
        _require_equal("item creation time", item["createdAt"], event.timestamp)
        _require_equal("item update time", item["updatedAt"], event.timestamp)
        _require_payload_scope(payload, event)
        _require_existing_run_scope(connection, event)
        _require_item_scope(item, event)
        insert_item(connection, item)
    elif event_type == "item.delta":
        _require_keys(payload, {"delta", "turnId", "runId", "itemId"})
        _require_string("item delta", payload["delta"])
        _require_payload_scope(payload, event)
        _require_existing_run_scope(connection, event)
        _require_streaming_message(connection, event)
        updated = connection.execute(
            "UPDATE items SET content = content || ?, updated_at = ? WHERE id = ?",
            (payload["delta"], event.timestamp, payload["itemId"]),
        )
        _require_one_update(updated, event_type)
    elif event_type == "item.completed":
        _require_keys(payload, {"item", "turnId", "runId", "itemId"})
        item = _record(payload, "item", _ITEM_KEYS)
        _validate_item(item)
        _require_choice("completed Item state", item["status"], _TERMINAL_STATUSES)
        _require_equal("item update time", item["updatedAt"], event.timestamp)
        _require_payload_scope(payload, event)
        _require_existing_run_scope(connection, event)
        _require_item_scope(item, event)
        existing = connection.execute(
            """
            SELECT turn_id, run_id, ordinal, kind, role, status, content, created_at
            FROM items WHERE id = ?
            """,
            (item["id"],),
        ).fetchone()
        if existing is None:
            if (
                item["kind"] != "tool_result"
                or item["role"] != "tool"
                or item["createdAt"] != event.timestamp
            ):
                raise RuntimeError("completed item is missing its start event")
            insert_item(connection, item)
        else:
            _require_existing_item_completion(existing, item)
            updated = connection.execute(
                """
                UPDATE items
                SET status = ?, content = ?, updated_at = ?, data_json = ?
                WHERE id = ?
                """,
                (
                    item["status"],
                    item["content"],
                    item["updatedAt"],
                    json_dumps(item["data"], separators=(",", ":"), ensure_ascii=False),
                    item["id"],
                ),
            )
            _require_one_update(updated, event_type)
    elif event_type == "run.settled":
        _require_keys(payload, {"status", "settledAt", "turnId", "runId"}, {"reasonCode"})
        _require_choice("settled Run state", payload["status"], _TERMINAL_STATUSES)
        _require_string("Run settledAt", payload["settledAt"])
        _require_equal("Run settlement time", payload["settledAt"], event.timestamp)
        if "reasonCode" in payload:
            _require_string("Run reasonCode", payload["reasonCode"])
        _require_payload_scope(payload, event, item_required=False)
        _require_existing_run_scope(connection, event)
        current = connection.execute(
            """
            SELECT r.status AS run_status, t.status AS turn_status
            FROM runs r JOIN turns t ON t.id = r.turn_id
            WHERE r.id = ?
            """,
            (event.run_id,),
        ).fetchone()
        if (
            current is None
            or current["run_status"] not in {"queued", "running"}
            or current["turn_status"] != current["run_status"]
        ):
            raise RuntimeError("Run settlement requires a matching active Run and Turn")
        if (
            connection.execute(
                "SELECT 1 FROM model_calls WHERE run_id = ? AND outcome IS NULL",
                (event.run_id,),
            ).fetchone()
            is not None
        ):
            raise RuntimeError("Run cannot settle with an unfinished model Step")
        if (
            connection.execute(
                """
            SELECT 1 FROM items
            WHERE run_id = ? AND status IN ('streaming', 'running')
            LIMIT 1
            """,
                (event.run_id,),
            ).fetchone()
            is not None
        ):
            raise RuntimeError("Run cannot settle with an active Item")
        updated_run = connection.execute(
            """
            UPDATE runs SET status = ?, settled_at = ?, reason_code = ?
            WHERE id = ? AND status IN ('queued', 'running')
            """,
            (
                payload["status"],
                payload["settledAt"],
                payload.get("reasonCode"),
                payload["runId"],
            ),
        )
        _require_one_update(updated_run, event_type)
        updated_turn = connection.execute(
            """
            UPDATE turns SET status = ?, updated_at = ?
            WHERE id = ? AND status = ?
            """,
            (
                payload["status"],
                payload["settledAt"],
                payload["turnId"],
                current["turn_status"],
            ),
        )
        _require_one_update(updated_turn, event_type)
    elif event_type == "file.change_recorded":
        _apply_file_change_event(connection, event)
    elif event_type == "context.compacted":
        _require_keys(
            payload,
            {
                "revision",
                "droppedTurns",
                "contextRevision",
                "turnId",
                "runId",
                "omittedItemIds",
                "trigger",
                "targetTokens",
            },
        )
        if event.run_id is None or event.thread_id is None or event.branch_id is None:
            raise RuntimeError("context compaction event has incomplete scope")
        _require_event_scope(
            event,
            thread_id=event.thread_id,
            branch_id=event.branch_id,
            turn_id=event.turn_id,
            run_id=event.run_id,
            item_id=None,
        )
        _require_payload_scope(payload, event, item_required=False)
        revision_value = payload["revision"]
        dropped = payload["droppedTurns"]
        revision_wire = payload["contextRevision"]
        if (
            not isinstance(revision_value, int)
            or isinstance(revision_value, bool)
            or revision_value < 1
            or not isinstance(dropped, list)
            or any(not isinstance(turn_id, str) or not turn_id for turn_id in dropped)
        ):
            raise RuntimeError("context compaction event metadata is invalid")
        omitted_item_ids = payload["omittedItemIds"]
        if (
            not isinstance(omitted_item_ids, list)
            or any(not isinstance(item_id, str) or not item_id for item_id in omitted_item_ids)
            or len(omitted_item_ids) != len(set(omitted_item_ids))
        ):
            raise RuntimeError("context compaction event metadata is invalid")
        trigger = payload["trigger"]
        target_tokens = payload["targetTokens"]
        if trigger not in {"budget_exceeded", "threshold"} or (
            not isinstance(target_tokens, int)
            or isinstance(target_tokens, bool)
            or target_tokens < 1
        ):
            raise RuntimeError("context compaction event metadata is invalid")
        try:
            revision = ContextRevision.from_wire(revision_wire)
        except (TypeError, ValueError):
            raise RuntimeError("context compaction event revision is invalid") from None
        if revision.revision != revision_value:
            raise RuntimeError("context compaction event revision does not match payload")
        run = connection.execute(
            """SELECT r.id, t.branch_id FROM runs r JOIN turns t ON t.id = r.turn_id
               WHERE r.id = ? AND r.turn_id = ?""",
            (event.run_id, event.turn_id),
        ).fetchone()
        if run is None:
            raise RuntimeError("context compaction event Run scope is invalid")
        if len(dropped) != len(set(dropped)):
            raise RuntimeError("context compaction dropped Turns are not unique")
        dropped_rows = connection.execute(
            "SELECT id FROM turns WHERE branch_id = ? AND id IN ({})".format(
                ",".join("?" for _ in dropped) or "NULL"
            ),
            (run["branch_id"], *dropped),
        ).fetchall()
        if {str(row["id"]) for row in dropped_rows} != set(dropped):
            raise RuntimeError("context compaction dropped Turn scope is invalid")
        if revision_value == 1:
            raise RuntimeError("context compaction must advance an existing revision")
        previous_row = connection.execute(
            "SELECT record_json FROM context_revisions WHERE run_id = ? AND revision = ?",
            (event.run_id, revision_value - 1),
        ).fetchone()
        if previous_row is None:
            raise RuntimeError("context compaction revision is not contiguous")
        previous = ContextRevision.from_wire(json_loads(str(previous_row["record_json"])))
        previous_ids = {reference.item_id for reference in previous.history_items}
        visible_pool = previous_ids | {
            record.item_id for record in load_current_run_context(connection, run_id=event.run_id)
        }
        revision_ids = {reference.item_id for reference in revision.history_items}
        if not revision_ids <= visible_pool:
            raise RuntimeError("context compaction revision introduces unknown input")
        expected_omitted = visible_pool - revision_ids
        if not set(omitted_item_ids or ()) <= expected_omitted:
            raise RuntimeError("context compaction omitted items do not match the revision")
        if any(item_id in revision_ids for item_id in (omitted_item_ids or ())):
            raise RuntimeError("context compaction omitted item remains visible")
        tool_rows = connection.execute(
            "SELECT id, kind, data_json FROM items WHERE id IN ({})".format(
                ",".join("?" for _ in visible_pool) or "NULL"
            ),
            tuple(visible_pool),
        ).fetchall()
        paired: dict[str, set[str]] = {}
        for row in tool_rows:
            data = json_loads(str(row["data_json"]))
            call_id = data.get("callId")
            if isinstance(call_id, str) and call_id:
                paired.setdefault(call_id, set()).add(str(row["id"]))
        omitted_set = set(omitted_item_ids or ())
        for pair in paired.values():
            if bool(pair & omitted_set) != (pair <= omitted_set):
                raise RuntimeError("context compaction splits a tool call and result")
        existing = connection.execute(
            "SELECT record_json FROM context_revisions WHERE run_id = ? AND revision = ?",
            (event.run_id, revision_value),
        ).fetchone()
        if existing is not None and str(existing["record_json"]) != canonical_json(
            revision.to_wire()
        ):
            raise RuntimeError("context compaction revision conflicts with stored record")
        if existing is None:
            connection.execute(
                "INSERT INTO context_revisions(run_id, revision, record_json) VALUES (?, ?, ?)",
                (event.run_id, revision_value, canonical_json(revision.to_wire())),
            )
        boundary = revision.current_run_omitted_through_item_id
        if boundary:
            owner = connection.execute(
                "SELECT run_id FROM items WHERE id = ?",
                (boundary,),
            ).fetchone()
            if owner is None or owner["run_id"] != event.run_id:
                raise RuntimeError("context compaction omission boundary is invalid")
    else:
        raise RuntimeError(f"journal event type is unsupported: {event_type}")


def _apply_file_change_event(connection: sqlite3.Connection, event: JournalEvent) -> None:
    if event.thread_id is None or event.item_id is None:
        raise RuntimeError("file change event has no source Tool Call")
    try:
        record = validate_file_change_record(
            {
                key: value
                for key, value in event.payload.items()
                if key not in {"turnId", "runId", "itemId"}
            }
        )
        row, call_data = file_tool_call(
            connection,
            event.thread_id,
            event.item_id,
            allowed=frozenset({"write", "edit"}),
        )
    except (ValueError, LookupError):
        raise RuntimeError("file change event is invalid") from None
    _require_payload_scope(event.payload, event)
    _require_event_scope(
        event,
        thread_id=str(row["thread_id"]),
        branch_id=str(row["branch_id"]),
        turn_id=str(row["turn_id"]),
        run_id=str(row["run_id"]),
        item_id=str(row["id"]),
    )
    if (
        record["threadId"] != event.thread_id
        or record["toolCallItemId"] != event.item_id
        or record["recordedAt"] != event.timestamp
        or record["operation"] != call_data["toolName"]
        or row["status"] not in {"completed", "failed", "cancelled"}
        or row["updated_at"] != event.timestamp
        or len(canonical_json(event.to_wire()).encode("utf-8")) > MAX_CHANGE_EVENT_BYTES
    ):
        raise RuntimeError("file change event does not match its Tool Call")
    if record["path"] is not None and record["path"] != resolve_file_tool_path(
        connection,
        event.thread_id,
        event.item_id,
    ):
        raise RuntimeError("file change event path does not match its Tool Call")
    has_result = any(
        json_loads(result["data_json"]).get("toolCallItemId") == event.item_id
        for result in connection.execute(
            "SELECT data_json FROM items WHERE run_id = ? AND kind = 'tool_result'",
            (event.run_id,),
        )
    )
    if not has_result:
        raise RuntimeError("file change event has no settled Tool Result")
    connection.execute(
        "INSERT INTO file_changes(tool_call_item_id, record_json) VALUES (?, ?)",
        (event.item_id, canonical_json(record)),
    )


def insert_item(connection: sqlite3.Connection, item: dict[str, Any]) -> None:
    connection.execute(
        """
        INSERT INTO items(
            id, turn_id, run_id, ordinal, kind, role, status, content,
            created_at, updated_at, data_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            item["id"],
            item["turnId"],
            item["runId"],
            item["ordinal"],
            item["kind"],
            item["role"],
            item["status"],
            item["content"],
            item["createdAt"],
            item["updatedAt"],
            json_dumps(item["data"], separators=(",", ":"), ensure_ascii=False),
        ),
    )


_THREAD_KEYS = {
    "id",
    "title",
    "defaultBranchId",
    "workspace",
    "createdAt",
    "updatedAt",
    "archivedAt",
}
_BRANCH_KEYS = {"id", "threadId", "createdAt", "isDefault"}
_TURN_KEYS = {"id", "threadId", "branchId", "ordinal", "status", "createdAt", "updatedAt"}
_RUN_KEYS = {
    "id",
    "turnId",
    "providerId",
    "modelId",
    "executionPolicy",
    "status",
    "createdAt",
    "settledAt",
    "skills",
}
_MODEL_USAGE_KEYS = {
    "inputTokens",
    "cachedInputTokens",
    "outputTokens",
    "reasoningOutputTokens",
    "totalTokens",
}
_ITEM_KEYS = {
    "id",
    "turnId",
    "runId",
    "ordinal",
    "kind",
    "role",
    "status",
    "content",
    "data",
    "createdAt",
    "updatedAt",
}
_TERMINAL_STATUSES = {"completed", "failed", "cancelled"}


def _validate_thread(thread: dict[str, Any]) -> None:
    for key in ("id", "defaultBranchId", "createdAt", "updatedAt"):
        _require_string(f"Thread {key}", thread[key])
    _require_optional_string("Thread title", thread["title"])
    _require_optional_string("Thread archivedAt", thread["archivedAt"])
    workspace_from_wire(thread["workspace"])


def _validate_branch(branch: dict[str, Any]) -> None:
    for key in ("id", "threadId", "createdAt"):
        _require_string(f"Branch {key}", branch[key])
    if not isinstance(branch["isDefault"], bool):
        raise RuntimeError("journal event Branch isDefault is invalid")


def _validate_turn(turn: dict[str, Any]) -> None:
    for key in ("id", "threadId", "branchId", "status", "createdAt", "updatedAt"):
        _require_string(f"Turn {key}", turn[key])
    _require_positive_integer("Turn ordinal", turn["ordinal"])


def _validate_run(run: dict[str, Any]) -> None:
    for key in (
        "id",
        "turnId",
        "providerId",
        "modelId",
        "executionPolicy",
        "status",
        "createdAt",
    ):
        _require_string(f"Run {key}", run[key])
    _require_optional_string("Run settledAt", run["settledAt"])
    _validate_optional_client_request(run)
    try:
        skill_descriptors_from_wire(run["skills"])
    except ValueError:
        raise RuntimeError("journal event Run Skills are invalid") from None


def _validate_item(item: dict[str, Any]) -> None:
    for key in (
        "id",
        "turnId",
        "runId",
        "kind",
        "status",
        "createdAt",
        "updatedAt",
    ):
        _require_string(f"Item {key}", item[key])
    _require_text("Item content", item["content"])
    _require_optional_string("Item role", item["role"])
    _require_positive_integer("Item ordinal", item["ordinal"])
    _require_choice("Item kind", item["kind"], {"message", "tool_call", "tool_result"})
    if not isinstance(item["data"], dict):
        raise RuntimeError("journal event Item data is not an object")


def _validate_model_usage(step_ordinal: object, usage: dict[str, Any]) -> None:
    _require_positive_integer("model usage Step ordinal", step_ordinal)
    for label, key in (
        ("input", "inputTokens"),
        ("output", "outputTokens"),
        ("total", "totalTokens"),
    ):
        _require_nonnegative_integer(f"model usage {label} tokens", usage[key])
    for label, key in (
        ("cached input", "cachedInputTokens"),
        ("reasoning output", "reasoningOutputTokens"),
    ):
        value = usage[key]
        if value is not None:
            _require_nonnegative_integer(f"model usage {label} tokens", value)
    if usage["cachedInputTokens"] is not None and usage["cachedInputTokens"] > usage["inputTokens"]:
        raise RuntimeError("journal event cached input tokens exceed input tokens")
    if (
        usage["reasoningOutputTokens"] is not None
        and usage["reasoningOutputTokens"] > usage["outputTokens"]
    ):
        raise RuntimeError("journal event reasoning output tokens exceed output tokens")


def _validate_optional_client_request(value: dict[str, Any]) -> None:
    if "clientRequestId" in value:
        _require_string("clientRequestId", value["clientRequestId"])


def _require_thread_lifecycle_transition(
    connection: sqlite3.Connection,
    event_type: str,
    thread: dict[str, Any],
) -> None:
    row = connection.execute(
        """
        SELECT title, default_branch_id, workspace_json, created_at, archived_at
        FROM threads WHERE id = ?
        """,
        (thread["id"],),
    ).fetchone()
    if row is None:
        raise RuntimeError("journal Thread lifecycle event has no existing Thread")
    immutable_expected = (
        row["default_branch_id"],
        workspace_from_json(row["workspace_json"]),
        row["created_at"],
    )
    immutable_actual = (
        thread["defaultBranchId"],
        workspace_from_wire(thread["workspace"]),
        thread["createdAt"],
    )
    if immutable_actual != immutable_expected:
        raise RuntimeError("journal Thread lifecycle event changes immutable fields")
    if event_type == "thread.renamed":
        if thread["title"] == row["title"] or thread["archivedAt"] != row["archived_at"]:
            raise RuntimeError("thread.renamed payload state is invalid")
    elif event_type == "thread.archived":
        if row["archived_at"] is not None or thread["title"] != row["title"]:
            raise RuntimeError("thread.archived payload state is invalid")
    elif row["archived_at"] is None or thread["title"] != row["title"]:
        raise RuntimeError("thread.unarchived payload state is invalid")


def _require_initial_thread_scope(connection: sqlite3.Connection, event: JournalEvent) -> None:
    row = connection.execute(
        """
        SELECT b.thread_id
        FROM branches b JOIN threads t ON t.id = b.thread_id
        WHERE b.id = ? AND t.id = ?
        """,
        (event.branch_id, event.thread_id),
    ).fetchone()
    if row is None:
        raise RuntimeError("initial Turn scope does not match its Thread and Branch")


def _require_streaming_message(connection: sqlite3.Connection, event: JournalEvent) -> None:
    row = connection.execute(
        "SELECT kind, role, status FROM items WHERE id = ?",
        (event.item_id,),
    ).fetchone()
    if row is None or (row["kind"], row["role"], row["status"]) != (
        "message",
        "assistant",
        "streaming",
    ):
        raise RuntimeError("item.delta target is not a streaming assistant message")


def _require_existing_item_completion(
    existing: sqlite3.Row,
    item: dict[str, Any],
) -> None:
    immutable_existing = tuple(
        existing[key]
        for key in ("turn_id", "run_id", "ordinal", "kind", "role", "content", "created_at")
    )
    immutable_payload = tuple(
        item[key] for key in ("turnId", "runId", "ordinal", "kind", "role", "content", "createdAt")
    )
    if immutable_payload != immutable_existing:
        raise RuntimeError("item.completed changes immutable Item fields")
    expected_active_status = "streaming" if existing["kind"] == "message" else "running"
    if (
        existing["kind"] not in {"message", "tool_call"}
        or existing["status"] != expected_active_status
    ):
        raise RuntimeError("item.completed target is not active")


def _require_string(label: str, value: object) -> None:
    if not isinstance(value, str) or not value:
        raise RuntimeError(f"journal event {label} is invalid")


def _require_text(label: str, value: object) -> None:
    if not isinstance(value, str):
        raise RuntimeError(f"journal event {label} is invalid")


def _require_optional_string(label: str, value: object) -> None:
    if value is not None:
        _require_string(label, value)


def _require_positive_integer(label: str, value: object) -> None:
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise RuntimeError(f"journal event {label} is invalid")


def _require_nonnegative_integer(label: str, value: object) -> None:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < 0
        or value > _SQLITE_MAX_INTEGER
    ):
        raise RuntimeError(f"journal event {label} is invalid")


def _require_activity_date(value: object) -> None:
    if not isinstance(value, str):
        raise RuntimeError("journal event model usage activityDate is invalid")
    try:
        parsed = date.fromisoformat(value)
    except ValueError:
        raise RuntimeError("journal event model usage activityDate is invalid") from None
    if parsed.isoformat() != value:
        raise RuntimeError("journal event model usage activityDate is invalid")


def _require_safe_response_identifier(label: str, value: object) -> None:
    if value is None:
        return
    if (
        not isinstance(value, str)
        or not value
        or len(value) > 200
        or any(ord(character) < 32 or ord(character) == 127 for character in value)
    ):
        raise RuntimeError(f"journal event {label} is invalid")


def _require_choice(label: str, value: object, choices: set[str]) -> None:
    if not isinstance(value, str) or value not in choices:
        raise RuntimeError(f"journal event {label} is invalid")


def _record(
    payload: dict[str, Any],
    key: str,
    required: set[str],
    optional: set[str] | None = None,
) -> dict[str, Any]:
    value = payload[key]
    if not isinstance(value, dict):
        raise RuntimeError(f"journal event field {key} is not an object")
    record = cast(dict[str, Any], value)
    _require_keys(record, required, optional or set())
    return record


def _require_keys(
    value: dict[str, Any],
    required: set[str],
    optional: set[str] | None = None,
) -> None:
    optional = optional or set()
    actual = set(value)
    if not required <= actual or not actual <= required | optional:
        raise RuntimeError("journal event payload shape is invalid")


def _require_event_scope(
    event: JournalEvent,
    *,
    thread_id: object,
    branch_id: object,
    turn_id: object,
    run_id: object,
    item_id: object,
) -> None:
    expected = (thread_id, branch_id, turn_id, run_id, item_id)
    actual = (event.thread_id, event.branch_id, event.turn_id, event.run_id, event.item_id)
    if actual != expected:
        raise RuntimeError("journal event envelope does not match its payload")


def _require_payload_scope(
    payload: dict[str, Any],
    event: JournalEvent,
    *,
    item_required: bool = True,
) -> None:
    _require_equal("payload turn", payload["turnId"], event.turn_id)
    _require_equal("payload run", payload["runId"], event.run_id)
    if item_required:
        _require_equal("payload item", payload["itemId"], event.item_id)


def _require_item_scope(item: dict[str, Any], event: JournalEvent) -> None:
    _require_equal("item identity", item["id"], event.item_id)
    _require_equal("item turn", item["turnId"], event.turn_id)
    _require_equal("item run", item["runId"], event.run_id)


def _require_existing_run_scope(connection: sqlite3.Connection, event: JournalEvent) -> None:
    row = connection.execute(
        """
        SELECT t.thread_id, t.branch_id, r.turn_id
        FROM runs r JOIN turns t ON t.id = r.turn_id
        WHERE r.id = ?
        """,
        (event.run_id,),
    ).fetchone()
    if row is None or (
        row["thread_id"],
        row["branch_id"],
        row["turn_id"],
        event.run_id,
    ) != (event.thread_id, event.branch_id, event.turn_id, event.run_id):
        raise RuntimeError("journal event scope does not match its Run")


def _require_equal(label: str, actual: object, expected: object) -> None:
    if actual != expected:
        raise RuntimeError(f"journal event {label} is inconsistent")


def _require_one_update(cursor: sqlite3.Cursor, event_type: str) -> None:
    if cursor.rowcount != 1:
        raise RuntimeError(f"journal event {event_type} did not update exactly one projection row")


def item_location(connection: sqlite3.Connection, item_id: str) -> sqlite3.Row:
    row = connection.execute(
        """
        SELECT i.turn_id, i.run_id, t.thread_id, t.branch_id
        FROM items i JOIN turns t ON t.id = i.turn_id
        WHERE i.id = ?
        """,
        (item_id,),
    ).fetchone()
    if row is None:
        raise LookupError("item was not found")
    return cast(sqlite3.Row, row)


def item_row(connection: sqlite3.Connection, item_id: str) -> sqlite3.Row:
    row = connection.execute(
        """
        SELECT i.id, i.turn_id, i.run_id, i.ordinal, i.kind, i.role, i.status,
               i.content, i.created_at, i.updated_at, i.data_json,
               t.thread_id, t.branch_id
        FROM items i JOIN turns t ON t.id = i.turn_id
        WHERE i.id = ?
        """,
        (item_id,),
    ).fetchone()
    if row is None:
        raise LookupError("item was not found")
    return cast(sqlite3.Row, row)


def next_item_ordinal(connection: sqlite3.Connection, run_id: str) -> int:
    return int(
        connection.execute(
            "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM items WHERE run_id = ?",
            (run_id,),
        ).fetchone()[0]
    )
