from __future__ import annotations

import sqlite3
import uuid
from collections.abc import Sequence
from pathlib import Path
from typing import Any, Self

from ..domain import (
    ContextItem,
    JournalEvent,
    ModelUsage,
    PreparedTurn,
    RecoveryPlan,
    RunDescriptor,
    ThreadSummary,
    UsageSnapshot,
    WorkspaceSummary,
    utc_now,
)
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from ..run_input import (
    CONTEXT_SELECTION_VERSION,
    CompletedProviderStepV1,
    PreparedModelStepV1,
    RunManifestV1,
    SubmissionFrameTemplateV1,
    SubmissionFrameV1,
    build_context_snapshot,
    build_step_manifest,
    canonical_json,
)
from ..security import response_values_contain_protected_value
from ..tools.core import ToolCall
from .journal import (
    append_event,
    event_from_row,
    latest_sequence,
    replay_events,
)
from .maintenance import (
    ProjectionRepairReport,
    StateBackupReport,
    StateCheckReport,
    check_state,
    create_state_backup,
    rebuild_projection_tables,
    repair_state_projections,
)
from .projections import (
    contains_protected_projection_values,
    context_item_records,
    context_item_records_for_snapshot,
    context_items,
    context_messages,
    find_turn_by_client_request_id,
    get_context_snapshot,
    get_run,
    get_run_manifest,
    get_submission_frame,
    has_active_runs,
    item_location,
    item_row,
    next_item_ordinal,
    run_status,
    workspace_from_json,
    workspace_to_json,
)
from .schema import initialize_schema, validate_existing_schema
from .thread_catalog import ThreadCatalogPage, list_thread_page
from .thread_history import (
    ThreadMetadata,
    TurnHistoryPage,
    get_thread_metadata,
    list_turn_history_page,
)
from .usage import local_activity_date, read_usage

_TERMINAL_RUN_STATUSES = frozenset({"completed", "failed", "cancelled"})
_SQLITE_MAX_INTEGER = (1 << 63) - 1


class SqliteRuntimeStore:
    def __init__(self, database_path: Path) -> None:
        database_path.parent.mkdir(parents=True, exist_ok=True)
        self.database_path = database_path
        self._connection = sqlite3.connect(database_path)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute("PRAGMA foreign_keys = ON")
        self._connection.execute("PRAGMA journal_mode = WAL")
        self._connection.execute("PRAGMA synchronous = NORMAL")
        try:
            initialize_schema(self._connection)
        except BaseException:
            self._connection.close()
            raise

    @classmethod
    def open_existing(cls, database_path: Path, *, read_only: bool) -> Self:
        resolved_path = database_path.resolve()
        mode = "ro" if read_only else "rw"
        connection = sqlite3.connect(f"{resolved_path.as_uri()}?mode={mode}", uri=True)
        connection.row_factory = sqlite3.Row
        connection.execute("PRAGMA foreign_keys = ON")
        try:
            validate_existing_schema(connection)
        except BaseException:
            connection.close()
            raise
        instance = cls.__new__(cls)
        instance.database_path = resolved_path
        instance._connection = connection
        return instance

    def close(self) -> None:
        self._connection.close()

    def has_active_runs(self) -> bool:
        return has_active_runs(self._connection)

    def journal_contains_protected_values(self, protected_values: Sequence[str]) -> bool:
        values = tuple(dict.fromkeys(value for value in protected_values if value))
        if not values:
            return False
        if contains_protected_projection_values(self._connection, values):
            return True

        after_seq = 0
        while True:
            events, latest_seq = replay_events(self._connection, after_seq, 256)
            if any(
                response_values_contain_protected_value(event.to_wire(), values)
                for event in events
            ):
                return True
            if not events or events[-1].seq >= latest_seq:
                return False
            after_seq = events[-1].seq

    def create_thread(
        self,
        title: str | None,
        *,
        workspace: WorkspaceSummary | None = None,
    ) -> tuple[ThreadSummary, JournalEvent]:
        thread, event, _created = self._create_thread(
            title,
            workspace=workspace,
            client_request_id=None,
        )
        return thread, event

    def create_thread_once(
        self,
        title: str | None,
        client_request_id: str,
        *,
        workspace: WorkspaceSummary | None = None,
    ) -> tuple[ThreadSummary, JournalEvent, bool]:
        return self._create_thread(
            title,
            workspace=workspace,
            client_request_id=client_request_id,
        )

    def _create_thread(
        self,
        title: str | None,
        *,
        workspace: WorkspaceSummary | None,
        client_request_id: str | None,
    ) -> tuple[ThreadSummary, JournalEvent, bool]:
        if client_request_id is not None:
            existing = self._connection.execute(
                """
                SELECT id, title, default_branch_id, workspace_json, created_at, updated_at,
                       archived_at
                FROM threads WHERE client_request_id = ?
                """,
                (client_request_id,),
            ).fetchone()
            if existing is not None:
                existing_workspace = workspace_from_json(existing["workspace_json"])
                if existing["title"] != title or existing_workspace != workspace:
                    raise LookupError(
                        "clientRequestId was already used with different thread.create parameters"
                    )
                event_row = self._connection.execute(
                    """
                    SELECT seq, schema_version, event_type, thread_id, branch_id, turn_id, run_id,
                           item_id,
                           created_at, payload_json
                    FROM events
                    WHERE event_type = 'thread.created' AND thread_id = ?
                    ORDER BY seq
                    LIMIT 1
                    """,
                    (existing["id"],),
                ).fetchone()
                if event_row is None:
                    raise RuntimeError("idempotent thread is missing its creation event")
                return (
                    ThreadSummary(
                        id=str(existing["id"]),
                        title=existing["title"],
                        default_branch_id=str(existing["default_branch_id"]),
                        workspace=existing_workspace,
                        created_at=str(existing["created_at"]),
                        updated_at=str(existing["updated_at"]),
                        archived_at=existing["archived_at"],
                    ),
                    event_from_row(event_row),
                    False,
                )

        thread_id = f"thread_{uuid.uuid4().hex}"
        branch_id = f"branch_{uuid.uuid4().hex}"
        timestamp = utc_now()
        thread = ThreadSummary(
            id=thread_id,
            title=title,
            default_branch_id=branch_id,
            workspace=workspace,
            created_at=timestamp,
            updated_at=timestamp,
            archived_at=None,
        )
        payload: dict[str, Any] = {
            "thread": thread.to_wire(),
            "branch": {
                "id": branch_id,
                "threadId": thread_id,
                "createdAt": timestamp,
                "isDefault": True,
            },
        }
        if client_request_id is not None:
            payload["clientRequestId"] = client_request_id
        with self._connection:
            self._connection.execute(
                """
                INSERT INTO threads(
                    id, title, default_branch_id, workspace_json,
                    created_at, updated_at, client_request_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    thread_id,
                    title,
                    branch_id,
                    workspace_to_json(workspace),
                    timestamp,
                    timestamp,
                    client_request_id,
                ),
            )
            self._connection.execute(
                """
                INSERT INTO branches(id, thread_id, created_at, is_default)
                VALUES (?, ?, ?, 1)
                """,
                (branch_id, thread_id, timestamp),
            )
            event = self._append_event(
                event_type="thread.created",
                thread_id=thread_id,
                branch_id=branch_id,
                payload=payload,
                timestamp=timestamp,
            )
        return thread, event, True

    def list_thread_page(
        self,
        *,
        cursor: str | None,
        limit: int,
        archived: bool = False,
    ) -> ThreadCatalogPage:
        return list_thread_page(
            self._connection,
            cursor=cursor,
            limit=limit,
            archived=archived,
        )

    def get_thread(self, thread_id: str) -> ThreadMetadata:
        return get_thread_metadata(self._connection, thread_id=thread_id)

    def rename_thread(
        self,
        thread_id: str,
        title: str | None,
    ) -> tuple[ThreadSummary, JournalEvent | None]:
        current = self.get_thread(thread_id).thread
        if current.title == title:
            return current, None
        timestamp = utc_now()
        updated = ThreadSummary(
            id=current.id,
            title=title,
            default_branch_id=current.default_branch_id,
            workspace=current.workspace,
            created_at=current.created_at,
            updated_at=timestamp,
            archived_at=current.archived_at,
        )
        with self._connection:
            self._connection.execute(
                "UPDATE threads SET title = ?, updated_at = ? WHERE id = ?",
                (title, timestamp, thread_id),
            )
            event = self._append_event(
                event_type="thread.renamed",
                thread_id=thread_id,
                branch_id=current.default_branch_id,
                timestamp=timestamp,
                payload={"thread": updated.to_wire()},
            )
        return updated, event

    def set_thread_archived(
        self,
        thread_id: str,
        *,
        archived: bool,
    ) -> tuple[ThreadSummary, JournalEvent | None]:
        current = self.get_thread(thread_id).thread
        if (current.archived_at is not None) is archived:
            return current, None
        timestamp = utc_now()
        if archived:
            active = self._connection.execute(
                """
                SELECT 1
                FROM runs r
                JOIN turns t ON t.id = r.turn_id
                WHERE t.thread_id = ? AND r.status IN ('queued', 'running')
                LIMIT 1
                """,
                (thread_id,),
            ).fetchone()
            if active is not None:
                raise LookupError("thread with an active run cannot be archived")
        updated = ThreadSummary(
            id=current.id,
            title=current.title,
            default_branch_id=current.default_branch_id,
            workspace=current.workspace,
            created_at=current.created_at,
            updated_at=timestamp,
            archived_at=timestamp if archived else None,
        )
        event_type = "thread.archived" if archived else "thread.unarchived"
        with self._connection:
            self._connection.execute(
                "UPDATE threads SET archived_at = ?, updated_at = ? WHERE id = ?",
                (updated.archived_at, timestamp, thread_id),
            )
            event = self._append_event(
                event_type=event_type,
                thread_id=thread_id,
                branch_id=current.default_branch_id,
                timestamp=timestamp,
                payload={"thread": updated.to_wire()},
            )
        return updated, event

    def list_turn_page(
        self,
        *,
        thread_id: str,
        branch_id: str,
        cursor: str | None,
        limit: int,
    ) -> TurnHistoryPage:
        return list_turn_history_page(
            self._connection,
            thread_id=thread_id,
            branch_id=branch_id,
            cursor=cursor,
            limit=limit,
        )

    def find_turn_by_client_request_id(
        self,
        *,
        thread_id: str,
        branch_id: str,
        content: str,
        provider_id: str,
        model_id: str,
        client_request_id: str,
    ) -> PreparedTurn | None:
        return find_turn_by_client_request_id(
            self._connection,
            thread_id=thread_id,
            branch_id=branch_id,
            content=content,
            provider_id=provider_id,
            model_id=model_id,
            client_request_id=client_request_id,
        )

    def prepare_turn(
        self,
        *,
        thread_id: str,
        branch_id: str,
        content: str,
        frame_template: SubmissionFrameTemplateV1,
        client_request_id: str | None = None,
    ) -> PreparedTurn:
        provider_id = frame_template.provider.provider_id
        model_id = frame_template.provider.model_id
        if client_request_id is not None:
            existing = self.find_turn_by_client_request_id(
                thread_id=thread_id,
                branch_id=branch_id,
                content=content,
                provider_id=provider_id,
                model_id=model_id,
                client_request_id=client_request_id,
            )
            if existing is not None:
                return existing

        owner = self._connection.execute(
            """
            SELECT t.archived_at, t.workspace_json
            FROM branches b
            JOIN threads t ON t.id = b.thread_id
            WHERE b.id = ? AND b.thread_id = ?
            """,
            (branch_id, thread_id),
        ).fetchone()
        if owner is None:
            raise LookupError("thread or branch was not found")
        if owner["archived_at"] is not None:
            raise LookupError("thread is archived")

        ordinal = int(
            self._connection.execute(
                "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM turns WHERE branch_id = ?",
                (branch_id,),
            ).fetchone()[0]
        )
        turn_id = f"turn_{uuid.uuid4().hex}"
        run_id = f"run_{uuid.uuid4().hex}"
        user_item_id = f"item_{uuid.uuid4().hex}"
        timestamp = utc_now()
        workspace = workspace_from_json(owner["workspace_json"])
        submission_frame = SubmissionFrameV1.from_template(
            frame_template,
            user_item_id=user_item_id,
            thread_id=thread_id,
            branch_id=branch_id,
            turn_id=turn_id,
            run_id=run_id,
            workspace=workspace,
        )
        run_manifest = RunManifestV1.from_frame(
            submission_frame,
            context_selection_version=CONTEXT_SELECTION_VERSION,
        )
        turn_payload = {
            "id": turn_id,
            "threadId": thread_id,
            "branchId": branch_id,
            "ordinal": ordinal,
            "status": "queued",
            "createdAt": timestamp,
            "updatedAt": timestamp,
        }
        run_payload = {
            "id": run_id,
            "turnId": turn_id,
            "providerId": provider_id,
            "modelId": model_id,
            "executionPolicy": submission_frame.execution_policy,
            "status": "queued",
            "createdAt": timestamp,
            "settledAt": None,
            "skills": [skill.to_wire() for skill in submission_frame.skills],
        }
        if client_request_id is not None:
            run_payload["clientRequestId"] = client_request_id
        item_payload = self._item_payload(
            item_id=user_item_id,
            turn_id=turn_id,
            run_id=run_id,
            ordinal=1,
            kind="message",
            role="user",
            status="completed",
            content=content,
            data={},
            created_at=timestamp,
            updated_at=timestamp,
        )

        with self._connection:
            self._connection.execute(
                """
                INSERT INTO turns(id, thread_id, branch_id, ordinal, status, created_at, updated_at)
                VALUES (?, ?, ?, ?, 'queued', ?, ?)
                """,
                (turn_id, thread_id, branch_id, ordinal, timestamp, timestamp),
            )
            self._connection.execute(
                """
                INSERT INTO runs(
                    id, turn_id, provider_id, model_id, execution_policy, status,
                    created_at, client_request_id
                ) VALUES (?, ?, ?, ?, ?, 'queued', ?, ?)
                """,
                (
                    run_id,
                    turn_id,
                    provider_id,
                    model_id,
                    submission_frame.execution_policy,
                    timestamp,
                    client_request_id,
                ),
            )
            self._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at
                ) VALUES (?, ?, ?, 1, 'message', 'user', 'completed', ?, ?, ?)
                """,
                (user_item_id, turn_id, run_id, content, timestamp, timestamp),
            )
            self._connection.execute(
                """
                INSERT INTO run_inputs(
                    run_id, submission_frame_json, run_manifest_json, context_snapshot_json
                ) VALUES (?, ?, ?, NULL)
                """,
                (
                    run_id,
                    canonical_json(submission_frame.to_wire()),
                    canonical_json(run_manifest.to_wire()),
                ),
            )
            self._connection.execute(
                "UPDATE threads SET updated_at = ? WHERE id = ?",
                (timestamp, thread_id),
            )
            user_event = self._append_event(
                event_type="item.completed",
                thread_id=thread_id,
                branch_id=branch_id,
                turn_id=turn_id,
                run_id=run_id,
                item_id=user_item_id,
                timestamp=timestamp,
                payload={
                    "turn": turn_payload,
                    "run": run_payload,
                    "item": item_payload,
                    "submissionFrame": submission_frame.to_wire(),
                    "runManifest": run_manifest.to_wire(),
                    **(
                        {"clientRequestId": client_request_id}
                        if client_request_id is not None
                        else {}
                    ),
                },
            )
            queued_event = self._append_event(
                event_type="run.state_changed",
                thread_id=thread_id,
                branch_id=branch_id,
                turn_id=turn_id,
                run_id=run_id,
                timestamp=timestamp,
                payload={"status": "queued"},
            )
        return PreparedTurn(
            turn_id=turn_id,
            run_id=run_id,
            thread_id=thread_id,
            branch_id=branch_id,
            initial_events=(user_event, queued_event),
            newly_created=True,
        )

    def get_run(self, run_id: str) -> RunDescriptor:
        return get_run(self._connection, run_id)

    def get_submission_frame(self, run_id: str) -> SubmissionFrameV1:
        return get_submission_frame(self._connection, run_id)

    def get_run_manifest(self, run_id: str) -> RunManifestV1:
        return get_run_manifest(self._connection, run_id)

    def run_status(self, run_id: str) -> str:
        return run_status(self._connection, run_id)

    def context_messages(
        self,
        branch_id: str,
        *,
        through_turn_id: str,
    ) -> list[tuple[str, str]]:
        return context_messages(self._connection, branch_id, through_turn_id=through_turn_id)

    def context_items(
        self,
        branch_id: str,
        *,
        through_turn_id: str,
    ) -> list[ContextItem]:
        return context_items(self._connection, branch_id, through_turn_id=through_turn_id)

    def mark_run_running(self, run_id: str) -> JournalEvent:
        run = self.get_run(run_id)
        timestamp = utc_now()
        with self._connection:
            updated = self._connection.execute(
                """
                UPDATE runs SET status = 'running', started_at = ?
                WHERE id = ? AND status = 'queued' AND started_at IS NULL
                """,
                (timestamp, run_id),
            )
            if updated.rowcount != 1:
                raise RuntimeError("run cannot transition to running")
            self._connection.execute(
                "UPDATE turns SET status = 'running', updated_at = ? WHERE id = ?",
                (timestamp, run.turn_id),
            )
            return self._append_event(
                event_type="run.state_changed",
                thread_id=run.thread_id,
                branch_id=run.branch_id,
                turn_id=run.turn_id,
                run_id=run.id,
                timestamp=timestamp,
                payload={"status": "running"},
            )

    def prepare_model_step(
        self,
        run_id: str,
        *,
        step_ordinal: int,
    ) -> PreparedModelStepV1:
        if not isinstance(step_ordinal, int) or isinstance(step_ordinal, bool) or step_ordinal < 1:
            raise ValueError("model Step ordinal must be a positive integer")
        run = self.get_run(run_id)
        frame = get_submission_frame(self._connection, run_id)
        run_manifest = get_run_manifest(self._connection, run_id)
        snapshot = get_context_snapshot(self._connection, run_id)
        if snapshot is None:
            records = context_item_records(
                self._connection,
                run.branch_id,
                through_turn_id=run.turn_id,
            )
            snapshot = build_context_snapshot(
                records,
                current_run_id=run_id,
                frame=frame,
                selection_version=run_manifest.context_selection_version,
            )
        else:
            records = context_item_records_for_snapshot(
                self._connection,
                run_id=run_id,
                snapshot=snapshot,
            )
        step_manifest = build_step_manifest(
            step_ordinal,
            records,
            snapshot,
            current_run_id=run_id,
            frame=frame,
        )
        timestamp = utc_now()
        with self._connection:
            status = self._connection.execute(
                "SELECT status FROM runs WHERE id = ?",
                (run_id,),
            ).fetchone()
            if status is None or status["status"] != "running":
                raise RuntimeError("model input preparation requires a running Run")
            if self._connection.execute(
                "SELECT 1 FROM model_steps WHERE run_id = ? AND outcome IS NULL",
                (run_id,),
            ).fetchone() is not None:
                raise RuntimeError("Run already has an unfinished model Step")
            expected_ordinal = int(
                self._connection.execute(
                    """
                    SELECT COALESCE(MAX(step_ordinal), 0) + 1
                    FROM model_steps WHERE run_id = ?
                    """,
                    (run_id,),
                ).fetchone()[0]
            )
            if step_ordinal != expected_ordinal:
                raise RuntimeError("model Step ordinal is not contiguous")
            stored_snapshot = get_context_snapshot(self._connection, run_id)
            if stored_snapshot is None:
                self._connection.execute(
                    "UPDATE run_inputs SET context_snapshot_json = ? WHERE run_id = ?",
                    (canonical_json(snapshot.to_wire()), run_id),
                )
            elif stored_snapshot != snapshot:
                raise RuntimeError("model Step changes the frozen Context Snapshot")
            self._connection.execute(
                """
                INSERT INTO model_steps(
                    run_id, step_ordinal, step_manifest_json, prepared_at
                ) VALUES (?, ?, ?, ?)
                """,
                (
                    run_id,
                    step_ordinal,
                    canonical_json(step_manifest.to_wire()),
                    timestamp,
                ),
            )
            event = self._append_event(
                event_type="model.input_prepared",
                thread_id=run.thread_id,
                branch_id=run.branch_id,
                turn_id=run.turn_id,
                run_id=run.id,
                timestamp=timestamp,
                payload={
                    "stepOrdinal": step_ordinal,
                    "preparedAt": timestamp,
                    "contextSnapshot": snapshot.to_wire(),
                    "stepManifest": step_manifest.to_wire(),
                },
            )
        return PreparedModelStepV1(
            context_snapshot=snapshot,
            step_manifest=step_manifest,
            items=tuple(record.to_context_item() for record in records),
            event=event,
        )

    def complete_provider_step(
        self,
        run_id: str,
        *,
        step_ordinal: int,
        assistant_item_id: str | None,
        tool_calls: Sequence[ToolCall],
        reasoning_content: str | None,
        usage: ModelUsage | None,
        response_model_id: str | None,
        request_id: str | None,
    ) -> CompletedProviderStepV1:
        _validate_model_step_metadata(
            step_ordinal,
            usage=usage,
            response_model_id=response_model_id,
            request_id=request_id,
        )
        run = self.get_run(run_id)
        step_id = f"step_{uuid.uuid4().hex}" if tool_calls else None
        events: list[JournalEvent] = []
        item_ids: list[str] = []
        with self._connection:
            self._require_open_model_step(run_id, step_ordinal)
            if assistant_item_id is not None:
                events.append(
                    self._terminalize_assistant_item_in_transaction(
                        assistant_item_id,
                        status="completed",
                        timestamp=utc_now(),
                        step_id=step_id,
                    )
                )
            for index, call in enumerate(tool_calls):
                item_id, event = self._create_tool_call_item_in_transaction(
                    run,
                    step_id=step_id,
                    call=call,
                    reasoning_content=reasoning_content if index == 0 else None,
                    timestamp=utc_now(),
                )
                item_ids.append(item_id)
                events.append(event)
            events.append(
                self._finish_model_step_in_transaction(
                    run,
                    step_ordinal=step_ordinal,
                    outcome="completed",
                    reason_code=None,
                    usage=usage,
                    response_model_id=response_model_id,
                    request_id=request_id,
                    timestamp=utc_now(),
                )
            )
        return CompletedProviderStepV1(
            step_id=step_id,
            tool_call_item_ids=tuple(item_ids),
            events=tuple(events),
        )

    def fail_provider_step(
        self,
        run_id: str,
        *,
        step_ordinal: int,
        outcome: str,
        reason_code: str,
        assistant_item_id: str | None,
        response_model_id: str | None = None,
        request_id: str | None = None,
    ) -> tuple[JournalEvent, ...]:
        if outcome not in {"failed", "cancelled"}:
            raise ValueError("failed Provider Step outcome is invalid")
        if not reason_code:
            raise ValueError("failed Provider Step reason is required")
        _validate_model_step_metadata(
            step_ordinal,
            usage=None,
            response_model_id=response_model_id,
            request_id=request_id,
        )
        run = self.get_run(run_id)
        events: list[JournalEvent] = []
        timestamp = utc_now()
        with self._connection:
            self._require_open_model_step(run_id, step_ordinal)
            if assistant_item_id is not None:
                events.append(
                    self._terminalize_assistant_item_in_transaction(
                        assistant_item_id,
                        status=outcome,
                        timestamp=timestamp,
                        step_id=None,
                    )
                )
            events.append(
                self._finish_model_step_in_transaction(
                    run,
                    step_ordinal=step_ordinal,
                    outcome=outcome,
                    reason_code=reason_code,
                    usage=None,
                    response_model_id=response_model_id,
                    request_id=request_id,
                    timestamp=timestamp,
                )
            )
        return tuple(events)

    def _require_open_model_step(self, run_id: str, step_ordinal: int) -> None:
        row = self._connection.execute(
            """
            SELECT ms.outcome, r.status
            FROM model_steps ms JOIN runs r ON r.id = ms.run_id
            WHERE ms.run_id = ? AND ms.step_ordinal = ?
            """,
            (run_id, step_ordinal),
        ).fetchone()
        if row is None:
            raise RuntimeError("model response has no prepared Step")
        if row["outcome"] is not None:
            raise RuntimeError("model response Step is already finished")
        if row["status"] != "running":
            raise RuntimeError("model response completion requires a running Run")

    def _terminalize_assistant_item_in_transaction(
        self,
        item_id: str,
        *,
        status: str,
        timestamp: str,
        step_id: str | None,
    ) -> JournalEvent:
        row = item_row(self._connection, item_id)
        if (
            row["kind"] != "message"
            or row["role"] != "assistant"
            or row["status"] != "streaming"
        ):
            raise RuntimeError("assistant item is not streaming")
        data = json_loads(row["data_json"])
        if step_id is not None:
            data["stepId"] = step_id
        updated = self._connection.execute(
            """
            UPDATE items SET status = ?, updated_at = ?, data_json = ?
            WHERE id = ? AND status = 'streaming'
            """,
            (
                status,
                timestamp,
                json_dumps(data, separators=(",", ":"), ensure_ascii=False),
                item_id,
            ),
        )
        if updated.rowcount != 1:
            raise RuntimeError("assistant item is not streaming")
        item = self._item_payload_from_row(
            row,
            status=status,
            updated_at=timestamp,
            data=data,
        )
        return self._append_event(
            event_type="item.completed",
            thread_id=str(row["thread_id"]),
            branch_id=str(row["branch_id"]),
            turn_id=str(row["turn_id"]),
            run_id=str(row["run_id"]),
            item_id=item_id,
            timestamp=timestamp,
            payload={"item": item},
        )

    def _create_tool_call_item_in_transaction(
        self,
        run: RunDescriptor,
        *,
        step_id: str | None,
        call: ToolCall,
        reasoning_content: str | None,
        timestamp: str,
    ) -> tuple[str, JournalEvent]:
        if step_id is None:
            raise RuntimeError("Tool call response has no Step ID")
        item_id = f"item_{uuid.uuid4().hex}"
        ordinal = next_item_ordinal(self._connection, run.id)
        data: dict[str, Any] = {
            "stepId": step_id,
            "callId": call.id,
            "toolName": call.name,
            "arguments": call.arguments,
        }
        if reasoning_content is not None:
            data["reasoningContent"] = reasoning_content
        item = self._item_payload(
            item_id=item_id,
            turn_id=run.turn_id,
            run_id=run.id,
            ordinal=ordinal,
            kind="tool_call",
            role="assistant",
            status="running",
            content="",
            data=data,
            created_at=timestamp,
            updated_at=timestamp,
        )
        self._connection.execute(
            """
            INSERT INTO items(
                id, turn_id, run_id, ordinal, kind, role, status, content,
                created_at, updated_at, data_json
            ) VALUES (?, ?, ?, ?, 'tool_call', 'assistant', 'running', '', ?, ?, ?)
            """,
            (
                item_id,
                run.turn_id,
                run.id,
                ordinal,
                timestamp,
                timestamp,
                json_dumps(data, separators=(",", ":"), ensure_ascii=False),
            ),
        )
        event = self._append_event(
            event_type="item.started",
            thread_id=run.thread_id,
            branch_id=run.branch_id,
            turn_id=run.turn_id,
            run_id=run.id,
            item_id=item_id,
            timestamp=timestamp,
            payload={"item": item},
        )
        return item_id, event

    def _finish_model_step_in_transaction(
        self,
        run: RunDescriptor,
        *,
        step_ordinal: int,
        outcome: str,
        reason_code: str | None,
        usage: ModelUsage | None,
        response_model_id: str | None,
        request_id: str | None,
        timestamp: str,
    ) -> JournalEvent:
        activity_date = local_activity_date(timestamp) if usage is not None else None
        usage_payload = _model_usage_payload(usage) if usage is not None else None
        updated = self._connection.execute(
            """
            UPDATE model_steps
            SET outcome = ?, reason_code = ?, response_model_id = ?, request_id = ?,
                usage_json = ?, activity_date = ?, finished_at = ?
            WHERE run_id = ? AND step_ordinal = ? AND outcome IS NULL
            """,
            (
                outcome,
                reason_code,
                response_model_id,
                request_id,
                canonical_json(usage_payload) if usage_payload is not None else None,
                activity_date,
                timestamp,
                run.id,
                step_ordinal,
            ),
        )
        if updated.rowcount != 1:
            raise RuntimeError("model response Step is not open")
        if usage is not None:
            assert activity_date is not None
            self._connection.execute(
                """
                INSERT INTO model_usages(
                    thread_id, turn_id, run_id, step_ordinal, provider_id, model_id,
                    input_tokens, cached_input_tokens, output_tokens,
                    reasoning_output_tokens, total_tokens, activity_date, completed_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    run.thread_id,
                    run.turn_id,
                    run.id,
                    step_ordinal,
                    run.provider_id,
                    run.model_id,
                    usage.input_tokens,
                    usage.cached_input_tokens,
                    usage.output_tokens,
                    usage.reasoning_output_tokens,
                    usage.total_tokens,
                    activity_date,
                    timestamp,
                ),
            )
        return self._append_event(
            event_type="model.response_finished",
            thread_id=run.thread_id,
            branch_id=run.branch_id,
            turn_id=run.turn_id,
            run_id=run.id,
            timestamp=timestamp,
            payload={
                "stepOrdinal": step_ordinal,
                "providerId": run.provider_id,
                "modelId": run.model_id,
                "outcome": outcome,
                "reasonCode": reason_code,
                "responseModelId": response_model_id,
                "requestId": request_id,
                "usage": usage_payload,
                "activityDate": activity_date,
                "finishedAt": timestamp,
            },
        )

    def read_usage(self) -> UsageSnapshot:
        return read_usage(self._connection)

    def create_assistant_item(self, run_id: str) -> tuple[str, JournalEvent]:
        run = self.get_run(run_id)
        item_id = f"item_{uuid.uuid4().hex}"
        timestamp = utc_now()
        ordinal = int(
            self._connection.execute(
                "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM items WHERE run_id = ?",
                (run_id,),
            ).fetchone()[0]
        )
        item = self._item_payload(
            item_id=item_id,
            turn_id=run.turn_id,
            run_id=run.id,
            ordinal=ordinal,
            kind="message",
            role="assistant",
            status="streaming",
            content="",
            data={},
            created_at=timestamp,
            updated_at=timestamp,
        )
        with self._connection:
            self._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, 'message', 'assistant', 'streaming', '', ?, ?)
                """,
                (item_id, run.turn_id, run.id, ordinal, timestamp, timestamp),
            )
            event = self._append_event(
                event_type="item.started",
                thread_id=run.thread_id,
                branch_id=run.branch_id,
                turn_id=run.turn_id,
                run_id=run.id,
                item_id=item_id,
                timestamp=timestamp,
                payload={"item": item},
            )
        return item_id, event

    def append_text_delta(self, item_id: str, delta: str) -> JournalEvent:
        location = item_location(self._connection, item_id)
        timestamp = utc_now()
        with self._connection:
            updated = self._connection.execute(
                """
                UPDATE items SET content = content || ?, updated_at = ?
                WHERE id = ? AND status = 'streaming'
                """,
                (delta, timestamp, item_id),
            )
            if updated.rowcount != 1:
                raise RuntimeError("assistant item is not streaming")
            return self._append_event(
                event_type="item.delta",
                thread_id=location["thread_id"],
                branch_id=location["branch_id"],
                turn_id=location["turn_id"],
                run_id=location["run_id"],
                item_id=item_id,
                timestamp=timestamp,
                payload={"delta": delta},
            )

    def complete_assistant_item(
        self,
        item_id: str,
        *,
        step_id: str | None = None,
    ) -> JournalEvent:
        """Complete streamed assistant text before a tool round continues.

        A provider may return both assistant narration and tool calls in one
        response.  The shared ``stepId`` lets provider-context reconstruction
        fold those separately projected Items back into the original assistant
        message without coupling the UI projection to provider wire formats.
        """

        row = item_row(self._connection, item_id)
        if row["kind"] != "message" or row["role"] != "assistant":
            raise RuntimeError("item is not an assistant message")
        if row["status"] != "streaming":
            raise RuntimeError("assistant item is not streaming")
        data = json_loads(row["data_json"])
        if step_id is not None:
            data["stepId"] = step_id
        timestamp = utc_now()
        with self._connection:
            updated = self._connection.execute(
                """
                UPDATE items SET status = 'completed', updated_at = ?, data_json = ?
                WHERE id = ? AND status = 'streaming'
                """,
                (
                    timestamp,
                    json_dumps(data, separators=(",", ":"), ensure_ascii=False),
                    item_id,
                ),
            )
            if updated.rowcount != 1:
                raise RuntimeError("assistant item is not streaming")
            location = item_location(self._connection, item_id)
            item = self._item_payload_from_row(
                row,
                status="completed",
                updated_at=timestamp,
                data=data,
            )
            return self._append_event(
                event_type="item.completed",
                thread_id=location["thread_id"],
                branch_id=location["branch_id"],
                turn_id=location["turn_id"],
                run_id=location["run_id"],
                item_id=item_id,
                timestamp=timestamp,
                payload={"item": item},
            )

    def create_tool_call_item(
        self,
        run_id: str,
        *,
        step_id: str,
        call_id: str,
        tool_name: str,
        arguments: dict[str, Any],
        reasoning_content: str | None = None,
    ) -> tuple[str, JournalEvent]:
        run = self.get_run(run_id)
        item_id = f"item_{uuid.uuid4().hex}"
        timestamp = utc_now()
        ordinal = next_item_ordinal(self._connection, run_id)
        data = {
            "stepId": step_id,
            "callId": call_id,
            "toolName": tool_name,
            "arguments": arguments,
        }
        if reasoning_content is not None:
            data["reasoningContent"] = reasoning_content
        item = self._item_payload(
            item_id=item_id,
            turn_id=run.turn_id,
            run_id=run.id,
            ordinal=ordinal,
            kind="tool_call",
            role="assistant",
            status="running",
            content="",
            data=data,
            created_at=timestamp,
            updated_at=timestamp,
        )
        with self._connection:
            self._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at, data_json
                ) VALUES (?, ?, ?, ?, 'tool_call', 'assistant', 'running', '', ?, ?, ?)
                """,
                (
                    item_id,
                    run.turn_id,
                    run.id,
                    ordinal,
                    timestamp,
                    timestamp,
                    json_dumps(
                        data,
                        separators=(",", ":"),
                        ensure_ascii=False,
                    ),
                ),
            )
            event = self._append_event(
                event_type="item.started",
                thread_id=run.thread_id,
                branch_id=run.branch_id,
                turn_id=run.turn_id,
                run_id=run.id,
                item_id=item_id,
                timestamp=timestamp,
                payload={"item": item},
            )
        return item_id, event

    def complete_tool_call(
        self,
        tool_call_item_id: str,
        *,
        status: str,
        result: dict[str, Any],
        result_content: str,
    ) -> tuple[JournalEvent, JournalEvent]:
        if status not in {"completed", "failed", "cancelled"}:
            raise ValueError("tool status is not terminal")
        row = item_row(self._connection, tool_call_item_id)
        if row["kind"] != "tool_call" or row["status"] != "running":
            raise RuntimeError("tool call item is not running")
        timestamp = utc_now()
        call_data = json_loads(row["data_json"])
        call_data["outcome"] = status
        duration_ms = result.get("durationMs")
        if isinstance(duration_ms, int) and not isinstance(duration_ms, bool):
            call_data["durationMs"] = duration_ms
        result_item_id = f"item_{uuid.uuid4().hex}"
        result_ordinal = next_item_ordinal(self._connection, str(row["run_id"]))
        result_data = {
            "stepId": call_data["stepId"],
            "callId": call_data["callId"],
            "toolCallItemId": tool_call_item_id,
            "toolName": call_data["toolName"],
            "result": result,
        }
        call_item = self._item_payload_from_row(
            row,
            status=status,
            updated_at=timestamp,
            data=call_data,
        )
        result_item = self._item_payload(
            item_id=result_item_id,
            turn_id=str(row["turn_id"]),
            run_id=str(row["run_id"]),
            ordinal=result_ordinal,
            kind="tool_result",
            role="tool",
            status=status,
            content=result_content,
            data=result_data,
            created_at=timestamp,
            updated_at=timestamp,
        )
        with self._connection:
            updated = self._connection.execute(
                """
                UPDATE items SET status = ?, updated_at = ?, data_json = ?
                WHERE id = ? AND status = 'running'
                """,
                (
                    status,
                    timestamp,
                    json_dumps(
                        call_data,
                        separators=(",", ":"),
                        ensure_ascii=False,
                    ),
                    tool_call_item_id,
                ),
            )
            if updated.rowcount != 1:
                raise RuntimeError("tool call item is not running")
            self._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at, data_json
                ) VALUES (?, ?, ?, ?, 'tool_result', 'tool', ?, ?, ?, ?, ?)
                """,
                (
                    result_item_id,
                    row["turn_id"],
                    row["run_id"],
                    result_ordinal,
                    status,
                    result_content,
                    timestamp,
                    timestamp,
                    json_dumps(
                        result_data,
                        separators=(",", ":"),
                        ensure_ascii=False,
                    ),
                ),
            )
            call_event = self._append_event(
                event_type="item.completed",
                thread_id=row["thread_id"],
                branch_id=row["branch_id"],
                turn_id=row["turn_id"],
                run_id=row["run_id"],
                item_id=tool_call_item_id,
                timestamp=timestamp,
                payload={"item": call_item},
            )
            result_event = self._append_event(
                event_type="item.completed",
                thread_id=row["thread_id"],
                branch_id=row["branch_id"],
                turn_id=row["turn_id"],
                run_id=row["run_id"],
                item_id=result_item_id,
                timestamp=timestamp,
                payload={"item": result_item},
            )
        return call_event, result_event

    def terminalize_run(
        self,
        run_id: str,
        status: str,
        *,
        reason_code: str | None = None,
        _finish_open_model_step: bool = False,
    ) -> tuple[JournalEvent, ...]:
        if status not in _TERMINAL_RUN_STATUSES:
            raise ValueError("run status is not terminal")
        run = self.get_run(run_id)
        timestamp = utc_now()
        with self._connection:
            row = self._connection.execute(
                "SELECT status FROM runs WHERE id = ?",
                (run_id,),
            ).fetchone()
            if row["status"] in _TERMINAL_RUN_STATUSES:
                return ()
            open_steps = self._connection.execute(
                """
                SELECT step_ordinal FROM model_steps
                WHERE run_id = ? AND outcome IS NULL
                ORDER BY step_ordinal
                """,
                (run_id,),
            ).fetchall()
            if len(open_steps) > 1:
                raise RuntimeError("Run has multiple unfinished model Steps")
            if open_steps and not _finish_open_model_step:
                raise RuntimeError("Run cannot settle with an unfinished model Step")
            if _finish_open_model_step and (status != "failed" or reason_code is None):
                raise RuntimeError("model Step recovery requires a failed Run reason")
            active_items = self._connection.execute(
                """
                SELECT id, ordinal, kind, role, status, content, created_at, data_json
                FROM items
                WHERE run_id = ? AND status IN ('streaming', 'running')
                ORDER BY ordinal
                """,
                (run_id,),
            ).fetchall()
            if status == "completed" and any(
                item_row["kind"] == "tool_call" for item_row in active_items
            ):
                raise RuntimeError("run cannot complete with an active tool call")
            events: list[JournalEvent] = []
            for item_row in active_items:
                item_id = str(item_row["id"])
                item_data = json_loads(item_row["data_json"])
                if item_row["kind"] == "tool_call":
                    item_data["outcome"] = status
                self._connection.execute(
                    "UPDATE items SET status = ?, updated_at = ?, data_json = ? WHERE id = ?",
                    (
                        status,
                        timestamp,
                        json_dumps(
                            item_data,
                            separators=(",", ":"),
                            ensure_ascii=False,
                        ),
                        item_id,
                    ),
                )
                item = self._item_payload(
                    item_id=item_id,
                    turn_id=run.turn_id,
                    run_id=run.id,
                    ordinal=int(item_row["ordinal"]),
                    kind=str(item_row["kind"]),
                    role=(str(item_row["role"]) if item_row["role"] is not None else None),
                    status=status,
                    content=str(item_row["content"]),
                    data=item_data,
                    created_at=str(item_row["created_at"]),
                    updated_at=timestamp,
                )
                events.append(
                    self._append_event(
                        event_type="item.completed",
                        thread_id=run.thread_id,
                        branch_id=run.branch_id,
                        turn_id=run.turn_id,
                        run_id=run.id,
                        item_id=item_id,
                        timestamp=timestamp,
                        payload={"item": item},
                    )
                )
                if item_row["kind"] == "tool_call":
                    result_item_id = f"item_{uuid.uuid4().hex}"
                    result_ordinal = next_item_ordinal(self._connection, run_id)
                    error_code = reason_code or (
                        "cancelled" if status == "cancelled" else "run_failed"
                    )
                    result = {
                        "toolCallId": item_data["callId"],
                        "toolName": item_data["toolName"],
                        "ok": False,
                        "output": "",
                        "cancelled": status == "cancelled",
                        "stdout": "",
                        "stderr": "",
                        "exitCode": None,
                        "durationMs": 0,
                        "timedOut": False,
                        "truncated": False,
                        "errorCode": error_code,
                    }
                    result_content = json_dumps(
                        result,
                        ensure_ascii=False,
                        separators=(",", ":"),
                    )
                    result_data = {
                        "stepId": item_data["stepId"],
                        "callId": item_data["callId"],
                        "toolCallItemId": item_id,
                        "toolName": item_data["toolName"],
                        "result": result,
                    }
                    result_item = self._item_payload(
                        item_id=result_item_id,
                        turn_id=run.turn_id,
                        run_id=run.id,
                        ordinal=result_ordinal,
                        kind="tool_result",
                        role="tool",
                        status=status,
                        content=result_content,
                        data=result_data,
                        created_at=timestamp,
                        updated_at=timestamp,
                    )
                    self._connection.execute(
                        """
                        INSERT INTO items(
                            id, turn_id, run_id, ordinal, kind, role, status, content,
                            created_at, updated_at, data_json
                        ) VALUES (?, ?, ?, ?, 'tool_result', 'tool', ?, ?, ?, ?, ?)
                        """,
                        (
                            result_item_id,
                            run.turn_id,
                            run.id,
                            result_ordinal,
                            status,
                            result_content,
                            timestamp,
                            timestamp,
                            json_dumps(
                                result_data,
                                separators=(",", ":"),
                                ensure_ascii=False,
                            ),
                        ),
                    )
                    events.append(
                        self._append_event(
                            event_type="item.completed",
                            thread_id=run.thread_id,
                            branch_id=run.branch_id,
                            turn_id=run.turn_id,
                            run_id=run.id,
                            item_id=result_item_id,
                            timestamp=timestamp,
                            payload={"item": result_item},
                        )
                    )
            if open_steps:
                events.append(
                    self._finish_model_step_in_transaction(
                        run,
                        step_ordinal=int(open_steps[0]["step_ordinal"]),
                        outcome="failed",
                        reason_code=reason_code,
                        usage=None,
                        response_model_id=None,
                        request_id=None,
                        timestamp=timestamp,
                    )
                )
            self._connection.execute(
                "UPDATE runs SET status = ?, settled_at = ?, reason_code = ? WHERE id = ?",
                (status, timestamp, reason_code, run_id),
            )
            self._connection.execute(
                "UPDATE turns SET status = ?, updated_at = ? WHERE id = ?",
                (status, timestamp, run.turn_id),
            )
            settled_payload: dict[str, Any] = {
                "status": status,
                "settledAt": timestamp,
            }
            if reason_code is not None:
                settled_payload["reasonCode"] = reason_code
            events.append(
                self._append_event(
                    event_type="run.settled",
                    thread_id=run.thread_id,
                    branch_id=run.branch_id,
                    turn_id=run.turn_id,
                    run_id=run.id,
                    timestamp=timestamp,
                    payload=settled_payload,
                )
            )
        return tuple(events)

    def recover_incomplete_runs(self) -> RecoveryPlan:
        running_rows = self._connection.execute(
            "SELECT id FROM runs WHERE status = 'running' ORDER BY created_at, id"
        ).fetchall()
        for row in running_rows:
            self.terminalize_run(
                str(row["id"]),
                "failed",
                reason_code="runtime_interrupted",
                _finish_open_model_step=True,
            )

        queued_rows = self._connection.execute(
            """
            SELECT r.id, MIN(e.seq) AS first_event_seq
            FROM runs r
            JOIN events e ON e.run_id = r.id
            WHERE r.status = 'queued'
            GROUP BY r.id
            ORDER BY first_event_seq, r.id
            """
        ).fetchall()
        return RecoveryPlan(tuple(str(row["id"]) for row in queued_rows))

    def replay_events(self, after_seq: int, limit: int) -> tuple[list[JournalEvent], int]:
        return replay_events(self._connection, after_seq, limit)

    def latest_sequence(self) -> int:
        return latest_sequence(self._connection)

    def check_state(self) -> StateCheckReport:
        return check_state(self._connection, self.database_path)

    def create_state_backup(self, destination: Path | None = None) -> StateBackupReport:
        return create_state_backup(self._connection, self.database_path, destination)

    def repair_state_projections(
        self,
        backup_destination: Path | None = None,
    ) -> ProjectionRepairReport:
        return repair_state_projections(
            self._connection,
            self.database_path,
            backup_destination,
        )

    def rebuild_projections(self) -> None:
        rebuild_projection_tables(self._connection)

    def _append_event(
        self,
        *,
        event_type: str,
        thread_id: str | None = None,
        branch_id: str | None = None,
        turn_id: str | None = None,
        run_id: str | None = None,
        item_id: str | None = None,
        payload: dict[str, Any],
        timestamp: str,
    ) -> JournalEvent:
        return append_event(
            self._connection,
            event_type=event_type,
            thread_id=thread_id,
            branch_id=branch_id,
            turn_id=turn_id,
            run_id=run_id,
            item_id=item_id,
            payload=payload,
            timestamp=timestamp,
        )

    @staticmethod
    def _item_payload(
        *,
        item_id: str,
        turn_id: str,
        run_id: str,
        ordinal: int,
        kind: str,
        role: str | None,
        status: str,
        content: str,
        data: dict[str, Any],
        created_at: str,
        updated_at: str,
    ) -> dict[str, Any]:
        return {
            "id": item_id,
            "turnId": turn_id,
            "runId": run_id,
            "ordinal": ordinal,
            "kind": kind,
            "role": role,
            "status": status,
            "content": content,
            "data": data,
            "createdAt": created_at,
            "updatedAt": updated_at,
        }

    @classmethod
    def _item_payload_from_row(
        cls,
        row: sqlite3.Row,
        *,
        status: str | None = None,
        updated_at: str | None = None,
        data: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        return cls._item_payload(
            item_id=str(row["id"]),
            turn_id=str(row["turn_id"]),
            run_id=str(row["run_id"]),
            ordinal=int(row["ordinal"]),
            kind=str(row["kind"]),
            role=str(row["role"]) if row["role"] is not None else None,
            status=status if status is not None else str(row["status"]),
            content=str(row["content"]),
            data=data if data is not None else json_loads(row["data_json"]),
            created_at=str(row["created_at"]),
            updated_at=updated_at if updated_at is not None else str(row["updated_at"]),
        )


def _validate_model_usage(step_ordinal: int, usage: ModelUsage) -> None:
    if not isinstance(step_ordinal, int) or isinstance(step_ordinal, bool) or step_ordinal < 1:
        raise ValueError("model usage Step ordinal must be a positive integer")
    for label, required_value in (
        ("input", usage.input_tokens),
        ("output", usage.output_tokens),
        ("total", usage.total_tokens),
    ):
        _validate_token_count(label, required_value)
    for label, optional_value in (
        ("cached input", usage.cached_input_tokens),
        ("reasoning output", usage.reasoning_output_tokens),
    ):
        if optional_value is not None:
            _validate_token_count(label, optional_value)
    if usage.cached_input_tokens is not None and usage.cached_input_tokens > usage.input_tokens:
        raise ValueError("cached input tokens cannot exceed input tokens")
    if (
        usage.reasoning_output_tokens is not None
        and usage.reasoning_output_tokens > usage.output_tokens
    ):
        raise ValueError("reasoning output tokens cannot exceed output tokens")


def _validate_model_step_metadata(
    step_ordinal: int,
    *,
    usage: ModelUsage | None,
    response_model_id: str | None,
    request_id: str | None,
) -> None:
    if usage is not None:
        _validate_model_usage(step_ordinal, usage)
    elif (
        not isinstance(step_ordinal, int)
        or isinstance(step_ordinal, bool)
        or step_ordinal < 1
    ):
        raise ValueError("model Step ordinal must be a positive integer")
    _validate_response_identifier("response model ID", response_model_id)
    _validate_response_identifier("request ID", request_id)


def _validate_response_identifier(label: str, value: str | None) -> None:
    if value is None:
        return
    if (
        not value
        or len(value) > 200
        or any(ord(character) < 32 or ord(character) == 127 for character in value)
    ):
        raise ValueError(f"{label} is invalid")


def _model_usage_payload(usage: ModelUsage) -> dict[str, int | None]:
    return {
        "inputTokens": usage.input_tokens,
        "cachedInputTokens": usage.cached_input_tokens,
        "outputTokens": usage.output_tokens,
        "reasoningOutputTokens": usage.reasoning_output_tokens,
        "totalTokens": usage.total_tokens,
    }


def _validate_token_count(label: str, value: object) -> None:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < 0
        or value > _SQLITE_MAX_INTEGER
    ):
        raise ValueError(f"model usage {label} tokens are invalid")
