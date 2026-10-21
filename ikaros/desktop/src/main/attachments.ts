import { createHash } from "node:crypto";
import { mkdir, stat, writeFile } from "node:fs/promises";
import { extname, isAbsolute, join } from "node:path";

import { app, nativeImage } from "electron";

import type { DesktopAttachmentDescriptor } from "../shared/platform";
import { RUNTIME_ATTACHMENT_LIMITS } from "../shared/runtime";

const IMAGE_MIME_BY_EXTENSION: Record<string, string> = {
  ".gif": "image/gif",
  ".jpeg": "image/jpeg",
  ".jpg": "image/jpeg",
  ".png": "image/png",
  ".webp": "image/webp",
};
const TEXT_MIME_BY_EXTENSION: Record<string, string> = {
  ".c": "text/x-c",
  ".css": "text/css",
  ".csv": "text/csv",
  ".go": "text/x-go",
  ".h": "text/x-c",
  ".html": "text/html",
  ".ini": "text/plain",
  ".java": "text/x-java",
  ".js": "text/javascript",
  ".json": "application/json",
  ".jsx": "text/javascript",
  ".log": "text/plain",
  ".md": "text/markdown",
  ".mjs": "text/javascript",
  ".py": "text/x-python",
  ".rs": "text/x-rust",
  ".sh": "text/x-shellscript",
  ".sql": "text/plain",
  ".svg": "image/svg+xml",
  ".toml": "text/plain",
  ".ts": "text/plain",
  ".tsx": "text/plain",
  ".txt": "text/plain",
  ".xml": "application/xml",
  ".yaml": "application/yaml",
  ".yml": "application/yaml",
};
const UNKNOWN_MIME_TYPE = "application/octet-stream";
const THUMBNAIL_MAX_EDGE = 96;
const IMAGE_SIGNATURES: ReadonlyArray<readonly [string, number[]]> = [
  ["image/png", [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]],
  ["image/jpeg", [0xff, 0xd8, 0xff]],
  ["image/gif", [0x47, 0x49, 0x46, 0x38]],
  ["image/webp", [0x52, 0x49, 0x46, 0x46]],
];

function attachmentMaxBytes(): number {
  return RUNTIME_ATTACHMENT_LIMITS.maxBytes;
}

function imageMimeTypes(): ReadonlySet<string> {
  return new Set(RUNTIME_ATTACHMENT_LIMITS.imageMimeTypes);
}

export function attachmentMimeType(path: string): { kind: "image" | "text"; mimeType: string } {
  const extension = extname(path).toLowerCase();
  const imageMime = IMAGE_MIME_BY_EXTENSION[extension];
  if (imageMime !== undefined) return { kind: "image", mimeType: imageMime };
  return { kind: "text", mimeType: TEXT_MIME_BY_EXTENSION[extension] ?? UNKNOWN_MIME_TYPE };
}

export function sniffImageMimeType(bytes: Uint8Array): string | null {
  for (const [mimeType, signature] of IMAGE_SIGNATURES) {
    if (signature.length > bytes.length) continue;
    if (signature.every((byte, index) => bytes[index] === byte)) {
      if (mimeType === "image/webp") {
        const format = String.fromCharCode(...bytes.slice(8, 12));
        if (format !== "WEBP") continue;
      }
      return mimeType;
    }
  }
  return null;
}

async function describePath(path: string): Promise<DesktopAttachmentDescriptor> {
  if (!isAbsolute(path)) throw new Error("attachment path must be absolute");
  const info = await stat(path).catch(() => null);
  if (info === null || !info.isFile()) throw new Error("attachment is not a file");
  if (info.size < 1) throw new Error("attachment file is empty");
  if (info.size > attachmentMaxBytes()) throw new Error("attachment exceeds the size limit");
  const { kind, mimeType } = attachmentMimeType(path);
  return { path, kind, mimeType, size: info.size };
}

export async function describeAttachmentPaths(paths: unknown): Promise<DesktopAttachmentDescriptor[]> {
  if (!Array.isArray(paths)) throw new Error("attachment paths must be an array");
  const described: DesktopAttachmentDescriptor[] = [];
  for (const path of paths) {
    if (typeof path !== "string") throw new Error("attachment paths must be strings");
    described.push(await describePath(path));
  }
  return described;
}

export async function attachmentThumbnail(path: unknown): Promise<string | null> {
  if (typeof path !== "string") throw new Error("attachment path must be a string");
  const descriptor = await describePath(path);
  if (descriptor.kind !== "image") return null;
  const image = nativeImage.createFromPath(descriptor.path);
  if (image.isEmpty()) return null;
  const size = image.getSize();
  const scale = THUMBNAIL_MAX_EDGE / Math.max(size.width, size.height, 1);
  const thumbnail = scale < 1
    ? image.resize({
        width: Math.max(1, Math.round(size.width * scale)),
        height: Math.max(1, Math.round(size.height * scale)),
        quality: "good",
      })
    : image;
  return thumbnail.toDataURL();
}

export async function saveClipboardImage(
  bytes: unknown,
  mimeType: unknown,
): Promise<DesktopAttachmentDescriptor> {
  if (!(bytes instanceof ArrayBuffer)) throw new Error("pasted image bytes are invalid");
  if (typeof mimeType !== "string" || !imageMimeTypes().has(mimeType)) {
    throw new Error("pasted image type is unsupported");
  }
  const content = Buffer.from(bytes);
  if (content.byteLength < 1) throw new Error("pasted image is empty");
  if (content.byteLength > attachmentMaxBytes()) throw new Error("attachment exceeds the size limit");
  if (sniffImageMimeType(content) !== mimeType) {
    throw new Error("pasted image does not match its declared type");
  }
  const extension = Object.entries(IMAGE_MIME_BY_EXTENSION).find(
    ([, value]) => value === mimeType,
  )?.[0] ?? ".png";
  const digest = createHash("sha256").update(content).digest("hex");
  const directory = join(app.getPath("temp"), "ikaros-attachments");
  await mkdir(directory, { recursive: true });
  const path = join(directory, `${digest.slice(0, 32)}${extension}`);
  await writeFile(path, content);
  return { path, kind: "image", mimeType, size: content.byteLength };
}

