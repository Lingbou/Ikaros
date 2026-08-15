from __future__ import annotations

import uuid
from collections.abc import AsyncIterator
from xml.etree import ElementTree

from ..cancellation import CancellationToken
from ..json_codec import loads as json_loads
from ..tools.core import ToolCall
from .base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)


class ScriptedProvider:
    id = "scripted"
    model_id = "scripted-v1"

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        if request.model_id != self.model_id:
            raise ValueError("unknown scripted model")
        users = [
            (index, message.content)
            for index, message in enumerate(request.messages)
            if message.role == "user"
        ]
        assistants = [
            message.content
            for message in request.messages
            if message.role == "assistant" and not message.tool_calls
        ]
        if not users:
            raise ValueError("scripted provider requires a user message")

        last_user_index, last_user = users[-1]
        command_prefix = "/process.run"
        skill_prefix = "/skill.run"
        tool_results = [
            message for message in request.messages[last_user_index + 1 :] if message.role == "tool"
        ]
        if last_user.startswith(skill_prefix):
            skill_name = last_user.removeprefix(skill_prefix).strip()
            available = {tool.name for tool in request.tools}
            if not skill_name or "read" not in available or "process_run" not in available:
                response = "Usage: /skill.run <skill-name>"
            elif not tool_results:
                location = _scripted_skill_location(request, skill_name)
                if location is None:
                    response = f"Skill is not available: {skill_name}"
                else:
                    yield ToolCallCompleted(
                        ToolCall(
                            id=f"call_{uuid.uuid4().hex}",
                            name="read",
                            arguments={"filePath": location},
                        )
                    )
                    yield ResponseCompleted()
                    return
            elif len(tool_results) == 1:
                command = _scripted_skill_command(tool_results[0].content)
                if command is None:
                    response = "Skill instructions did not declare a runnable test command."
                else:
                    yield ToolCallCompleted(
                        ToolCall(
                            id=f"call_{uuid.uuid4().hex}",
                            name="process_run",
                            arguments={"command": command},
                        )
                    )
                    yield ResponseCompleted()
                    return
            else:
                response = f"Skill completed.\n{_scripted_tool_summary(tool_results[-1].content)}"
        elif last_user.startswith(command_prefix) and not tool_results:
            command = last_user.removeprefix(command_prefix).strip()
            available = {tool.name for tool in request.tools}
            if command and "process_run" in available:
                yield ToolCallCompleted(
                    ToolCall(
                        id=f"call_{uuid.uuid4().hex}",
                        name="process_run",
                        arguments={"command": command},
                    )
                )
                yield ResponseCompleted()
                return
            response = "Usage: /process.run <command>"
        elif tool_results:
            response = _scripted_tool_summary(tool_results[-1].content)
        elif len(users) == 1:
            response = f"Scripted response to: {last_user}"
        else:
            previous_assistant = assistants[-1] if assistants else "(none)"
            response = (
                f"Previous user: {users[-2][1]}\n"
                f"Previous assistant: {previous_assistant}\n"
                f"Current user: {last_user}"
            )

        for start in range(0, len(response), 12):
            await cancellation.sleep(0.005)
            yield TextDelta(response[start : start + 12])
        yield ResponseCompleted()


def _scripted_tool_summary(content: str) -> str:
    try:
        result = json_loads(content)
    except ValueError:
        return f"Command result: {content}"
    if not isinstance(result, dict):
        return f"Command result: {content}"
    output = result.get("output")
    exit_code = result.get("exitCode")
    if not isinstance(output, str):
        output = ""
    if result.get("cancelled") is True:
        return f"Command was cancelled.\n{output}".rstrip()
    if result.get("timedOut") is True:
        return f"Command timed out.\n{output}".rstrip()
    return f"Command exited with code {exit_code}.\n{output}".rstrip()


def _scripted_skill_location(request: ProviderRequest, skill_name: str) -> str | None:
    for message in request.messages:
        if message.role != "system":
            continue
        start = message.content.find("<available_skills>")
        end = message.content.find("</available_skills>")
        if start < 0 or end < start:
            continue
        source = message.content[start : end + len("</available_skills>")]
        try:
            root = ElementTree.fromstring(source)
        except ElementTree.ParseError:
            continue
        for skill in root.findall("skill"):
            if skill.findtext("name") == skill_name:
                location = skill.findtext("location")
                return location if location else None
    return None


def _scripted_skill_command(content: str) -> str | None:
    try:
        result = json_loads(content)
    except ValueError:
        return None
    if (
        not isinstance(result, dict)
        or result.get("toolName") != "read"
        or result.get("ok") is not True
        or not isinstance(result.get("output"), str)
    ):
        return None
    output = result["output"]
    assert isinstance(output, str)
    prefix = "IKAROS_SCRIPT_COMMAND:"
    for line in output.splitlines():
        marker = line.find(prefix)
        if marker < 0:
            continue
        line_prefix = line[:marker].strip()
        if line_prefix and not (
            line_prefix.endswith(":") and line_prefix[:-1].isdigit()
        ):
            continue
        command = line[marker + len(prefix) :].strip()
        return command or None
    return None
