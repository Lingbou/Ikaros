from __future__ import annotations

import base64
import hashlib
from pathlib import Path

import pytest

from ikaros_runtime.attachments import (
    ATTACHMENT_IMAGE_TOKEN_ESTIMATE,
    AttachmentUnavailableError,
    attachment_inputs_from_params,
    attachment_refs_from_wire,
    attachment_refs_to_wire,
    estimate_attachment_tokens,
    image_data_url,
    resolve_attachment_inputs,
)
from ikaros_runtime.protocol.spec import ATTACHMENT_MAX_BYTES, ATTACHMENT_MAX_COUNT

PNG_BYTES = b"\x89PNG\r\n\x1a\n" + b"ikaros-image-body"


def write_image(root: Path, name: str = "pixel.png") -> Path:
    path = root / name
    path.write_bytes(PNG_BYTES)
    return path


def image_input(path: Path) -> dict[str, object]:
    return {"kind": "image", "path": str(path), "mimeType": "image/png"}


def test_attachment_inputs_require_the_exact_wire_shape() -> None:
    assert attachment_inputs_from_params(None) == ()
    with pytest.raises(ValueError, match="attachments must be an array"):
        attachment_inputs_from_params({"kind": "image"})
    with pytest.raises(ValueError, match="each attachment requires"):
        attachment_inputs_from_params([{"kind": "image", "path": "/tmp/a.png"}])
    with pytest.raises(ValueError, match="each attachment requires"):
        attachment_inputs_from_params(
            [
                {
                    "kind": "image",
                    "path": "/tmp/a.png",
                    "mimeType": "image/png",
                    "size": 12,
                }
            ]
        )
    with pytest.raises(ValueError, match="kind must be image or text"):
        attachment_inputs_from_params(
            [{"kind": "video", "path": "/a.mp4", "mimeType": "video/mp4"}]
        )
    with pytest.raises(ValueError, match="mimeType is invalid"):
        attachment_inputs_from_params([{"kind": "text", "path": "/a.txt", "mimeType": "nope"}])
    with pytest.raises(ValueError, match="image attachments must be one of"):
        attachment_inputs_from_params(
            [{"kind": "image", "path": "/a.bmp", "mimeType": "image/bmp"}]
        )
    with pytest.raises(ValueError, match="at most"):
        attachment_inputs_from_params(
            [
                {"kind": "text", "path": f"/a{index}.txt", "mimeType": "text/plain"}
                for index in range(ATTACHMENT_MAX_COUNT + 1)
            ]
        )
    with pytest.raises(ValueError, match="paths must be unique"):
        attachment_inputs_from_params(
            [
                {"kind": "text", "path": "/a.txt", "mimeType": "text/plain"},
                {"kind": "text", "path": "/a.txt", "mimeType": "text/plain"},
            ]
        )


def test_resolve_attachment_records_integrity_facts(tmp_path: Path) -> None:
    image = write_image(tmp_path)
    (reference,) = resolve_attachment_inputs([image_input(image)])

    assert reference.kind == "image"
    assert reference.path == str(image)
    assert reference.mime_type == "image/png"
    assert reference.size == len(PNG_BYTES)
    assert reference.sha256 == hashlib.sha256(PNG_BYTES).hexdigest()
    assert attachment_refs_from_wire(attachment_refs_to_wire([reference])) == (reference,)


def test_resolve_attachment_rejects_missing_empty_and_oversized_files(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="was not found"):
        resolve_attachment_inputs([image_input(tmp_path / "missing.png")])
    with pytest.raises(ValueError, match="must be absolute"):
        resolve_attachment_inputs([image_input(Path("relative.png"))])
    empty = tmp_path / "empty.txt"
    empty.write_bytes(b"")
    with pytest.raises(ValueError, match="is empty"):
        resolve_attachment_inputs([{"kind": "text", "path": str(empty), "mimeType": "text/plain"}])
    oversized = tmp_path / "huge.txt"
    oversized.write_bytes(b"x" * (ATTACHMENT_MAX_BYTES + 1))
    with pytest.raises(ValueError, match="byte limit"):
        resolve_attachment_inputs(
            [{"kind": "text", "path": str(oversized), "mimeType": "text/plain"}]
        )
    directory = tmp_path / "folder"
    directory.mkdir()
    with pytest.raises(ValueError, match="is not a file"):
        resolve_attachment_inputs([image_input(directory)])


def test_resolve_attachment_sniffs_image_content(tmp_path: Path) -> None:
    disguised = tmp_path / "disguised.png"
    disguised.write_bytes(b"not really a png")
    with pytest.raises(ValueError, match="is not a valid image/png image"):
        resolve_attachment_inputs([image_input(disguised)])


def test_image_data_url_inlines_bytes_and_rejects_changed_files(tmp_path: Path) -> None:
    image = write_image(tmp_path)
    (reference,) = resolve_attachment_inputs([image_input(image)])

    encoded = image_data_url(reference)
    assert encoded == f"data:image/png;base64,{base64.b64encode(PNG_BYTES).decode('ascii')}"

    image.write_bytes(b"\x89PNG\r\n\x1a\n" + b"different-body")
    with pytest.raises(AttachmentUnavailableError, match="changed after it was attached"):
        image_data_url(reference)

    image.unlink()
    with pytest.raises(AttachmentUnavailableError, match="is missing"):
        image_data_url(reference)


def test_text_attachment_stays_a_path_reference(tmp_path: Path) -> None:
    notes = tmp_path / "notes.md"
    notes.write_text("# Notes\n", encoding="utf-8")
    (reference,) = resolve_attachment_inputs(
        [{"kind": "text", "path": str(notes), "mimeType": "text/markdown"}]
    )

    assert reference.kind == "text"
    assert reference.size == notes.stat().st_size
    assert estimate_attachment_tokens([reference]) >= len(str(notes)) // 4
    with pytest.raises(ValueError, match="only image attachments can be encoded"):
        image_data_url(reference)


def test_estimate_attachment_tokens_counts_images_conservatively(tmp_path: Path) -> None:
    image = write_image(tmp_path)
    notes = tmp_path / "notes.txt"
    notes.write_text("hello", encoding="utf-8")
    refs = resolve_attachment_inputs(
        [
            image_input(image),
            {"kind": "text", "path": str(notes), "mimeType": "text/plain"},
        ]
    )

    assert estimate_attachment_tokens(refs) >= ATTACHMENT_IMAGE_TOKEN_ESTIMATE


def test_attachment_refs_from_wire_rejects_malformed_records() -> None:
    with pytest.raises(ValueError, match="must be an array"):
        attachment_refs_from_wire({"kind": "image"})
    with pytest.raises(ValueError, match="invalid structure"):
        attachment_refs_from_wire([{"kind": "image", "path": "/a.png", "mimeType": "image/png"}])
    with pytest.raises(ValueError, match="sha256 is invalid"):
        attachment_refs_from_wire(
            [
                {
                    "kind": "image",
                    "path": "/a.png",
                    "mimeType": "image/png",
                    "size": 3,
                    "sha256": "zz",
                }
            ]
        )
