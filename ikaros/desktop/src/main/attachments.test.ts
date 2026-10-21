import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const electron = vi.hoisted(() => ({
  app: { getPath: vi.fn(() => "") },
  nativeImage: {
    createFromPath: vi.fn(),
  },
}));

vi.mock("electron", () => ({
  app: electron.app,
  nativeImage: electron.nativeImage,
}));

import {
  attachmentMimeType,
  attachmentThumbnail,
  describeAttachmentPaths,
  saveClipboardImage,
  sniffImageMimeType,
} from "./attachments";

const PNG_BYTES = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);
let root: string;

beforeEach(async () => {
  root = await mkdtemp(join(tmpdir(), "ikaros-attachments-"));
  electron.app.getPath.mockReturnValue(root);
  electron.nativeImage.createFromPath.mockReset();
});

afterEach(async () => {
  await rm(root, { recursive: true, force: true });
});

describe("desktop attachment helpers", () => {
  it("maps file extensions onto Runtime attachment kinds", () => {
    expect(attachmentMimeType("C:/tmp/pixel.png")).toEqual({
      kind: "image",
      mimeType: "image/png",
    });
    expect(attachmentMimeType("C:/tmp/notes.md")).toEqual({
      kind: "text",
      mimeType: "text/markdown",
    });
    expect(attachmentMimeType("C:/tmp/archive.zip")).toEqual({
      kind: "text",
      mimeType: "application/octet-stream",
    });
  });

  it("sniffs inline image signatures", () => {
    expect(sniffImageMimeType(PNG_BYTES)).toBe("image/png");
    const webp = Buffer.concat([
      Buffer.from("RIFF"),
      Buffer.from([0, 0, 0, 0]),
      Buffer.from("WEBP"),
      Buffer.from([1, 2]),
    ]);
    expect(sniffImageMimeType(webp)).toBe("image/webp");
    expect(sniffImageMimeType(Buffer.from("plain text"))).toBeNull();
  });

  it("describes selected files and rejects unusable paths", async () => {
    const image = join(root, "pixel.png");
    await writeFile(image, PNG_BYTES);
    const notes = join(root, "notes.md");
    await writeFile(notes, "# Notes\n");

    await expect(describeAttachmentPaths([image, notes])).resolves.toEqual([
      { path: image, kind: "image", mimeType: "image/png", size: PNG_BYTES.byteLength },
      { path: notes, kind: "text", mimeType: "text/markdown", size: 8 },
    ]);
    await expect(describeAttachmentPaths(["relative.png"])).rejects.toThrow(
      "attachment path must be absolute",
    );
    await expect(describeAttachmentPaths([join(root, "missing.png")])).rejects.toThrow(
      "attachment is not a file",
    );
    await expect(describeAttachmentPaths(root)).rejects.toThrow(
      "attachment paths must be an array",
    );
  });

  it("returns bounded thumbnails for image attachments only", async () => {
    const image = join(root, "pixel.png");
    await writeFile(image, PNG_BYTES);
    const notes = join(root, "notes.md");
    await writeFile(notes, "# Notes\n");
    const resized = { toDataURL: () => "data:image/png;base64,THUMB" };
    electron.nativeImage.createFromPath.mockReturnValue({
      isEmpty: () => false,
      getSize: () => ({ width: 512, height: 256 }),
      resize: vi.fn(() => resized),
      toDataURL: () => "data:image/png;base64,FULL",
    });

    await expect(attachmentThumbnail(image)).resolves.toBe("data:image/png;base64,THUMB");
    await expect(attachmentThumbnail(notes)).resolves.toBeNull();
    electron.nativeImage.createFromPath.mockReturnValue({ isEmpty: () => true });
    await expect(attachmentThumbnail(image)).resolves.toBeNull();
  });

  it("stores pasted images in the attachment temp directory", async () => {
    const bytes = new Uint8Array(PNG_BYTES).buffer;

    const descriptor = await saveClipboardImage(bytes, "image/png");

    expect(descriptor.kind).toBe("image");
    expect(descriptor.mimeType).toBe("image/png");
    expect(descriptor.size).toBe(PNG_BYTES.byteLength);
    expect(descriptor.path.startsWith(join(root, "ikaros-attachments"))).toBe(true);
    await expect(describeAttachmentPaths([descriptor.path])).resolves.toEqual([descriptor]);
    await expect(
      saveClipboardImage(bytes, "image/jpeg"),
    ).rejects.toThrow("does not match its declared type");
    await expect(saveClipboardImage(bytes, "image/bmp")).rejects.toThrow(
      "pasted image type is unsupported",
    );
    await expect(saveClipboardImage("not bytes", "image/png")).rejects.toThrow(
      "pasted image bytes are invalid",
    );
  });
});
