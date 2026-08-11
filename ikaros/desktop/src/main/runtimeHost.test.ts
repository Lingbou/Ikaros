// @vitest-environment node

import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { RuntimeHost } from "./runtimeHost";

const runtimeRoot = fileURLToPath(new URL("../../../../runtime", import.meta.url));

describe("RuntimeHost integration", () => {
  it("starts one authenticated Runtime, initializes it, and stops it cleanly", async () => {
    const host = new RuntimeHost({ runtimeRoot });

    try {
      const first = await host.start();
      const second = await host.start();

      expect(second).toEqual(first);
      expect(first).toEqual(
        expect.objectContaining({
          protocolVersion: 1,
          host: "127.0.0.1",
          server: { name: "ikaros-runtime", version: "0.1.0" }
        })
      );
      expect(first.port).toBeGreaterThan(0);
      expect(first.pid).toBeGreaterThan(0);
      expect(host.pid).toBeGreaterThan(0);
      expect(host.isRunning).toBe(true);
    } finally {
      await host.stop();
    }

    expect(host.isRunning).toBe(false);
  });
});
