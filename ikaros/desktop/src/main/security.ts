import { shell, type BrowserWindow, type IpcMainInvokeEvent } from "electron";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

export interface RendererTrustPolicy {
  assertTrustedIpc(event: IpcMainInvokeEvent): void;
  isTrustedUrl(url: string): boolean;
}

function normalizeFilePath(path: string): string {
  const normalized = resolve(path);
  return process.platform === "win32" ? normalized.toLocaleLowerCase("en-US") : normalized;
}

export function createRendererTrustPolicy(rendererUrl: string): RendererTrustPolicy {
  const expected = new URL(rendererUrl);

  const isTrustedUrl = (candidateUrl: string): boolean => {
    try {
      const candidate = new URL(candidateUrl);

      if (expected.protocol === "file:") {
        return (
          candidate.protocol === "file:" &&
          normalizeFilePath(fileURLToPath(candidate)) === normalizeFilePath(fileURLToPath(expected))
        );
      }

      return candidate.origin === expected.origin;
    } catch {
      return false;
    }
  };

  return {
    isTrustedUrl,
    assertTrustedIpc(event): void {
      if (event.senderFrame !== event.sender.mainFrame || !isTrustedUrl(event.senderFrame.url)) {
        throw new Error("Blocked IPC request from an untrusted renderer.");
      }
    }
  };
}

function openExternalUrl(url: string): void {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
      return;
    }
    void shell.openExternal(parsed.toString());
  } catch {
    // Invalid and non-web URLs stay inside the trust boundary.
  }
}

export function installWindowSecurity(
  window: BrowserWindow,
  trustPolicy: RendererTrustPolicy
): void {
  const { webContents } = window;

  webContents.setWindowOpenHandler(({ url }) => {
    openExternalUrl(url);
    return { action: "deny" };
  });

  webContents.on("will-navigate", (event, url) => {
    if (trustPolicy.isTrustedUrl(url)) {
      return;
    }

    event.preventDefault();
    openExternalUrl(url);
  });

  webContents.on("will-attach-webview", (event) => {
    event.preventDefault();
  });

  const appSession = webContents.session;
  appSession.setPermissionRequestHandler((requestingWebContents, permission, callback) => {
    const allowClipboardWrite =
      requestingWebContents === webContents &&
      permission === "clipboard-sanitized-write" &&
      trustPolicy.isTrustedUrl(requestingWebContents.getURL());
    callback(allowClipboardWrite);
  });
  appSession.setPermissionCheckHandler((requestingWebContents, permission) => {
    return (
      requestingWebContents === webContents &&
      permission === "clipboard-sanitized-write" &&
      trustPolicy.isTrustedUrl(requestingWebContents.getURL())
    );
  });
}
