// @vitest-environment node

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it, vi } from "vitest";

import type { RuntimeJournalEvent } from "../shared/runtime";
import { RuntimeHost } from "./runtimeHost";

const runtimeRoot = fileURLToPath(new URL("../../../../runtime", import.meta.url));

interface ThreadSummary {
  id: string;
  title: string | null;
  defaultBranchId: string;
  createdAt: string;
  updatedAt: string;
}

describe("RuntimeHost integration", () => {
  it("owns one authenticated Runtime and preserves journal state across restart", async () => {
    const runtimeHome = await mkdtemp(join(tmpdir(), "ikaros-runtime-host-"));
    try {
      const firstHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      const events: RuntimeJournalEvent[] = [];
      const removeNotification = firstHost.onNotification((notification) => {
        if (notification.method === "event") {
          events.push(notification.params as RuntimeJournalEvent);
        }
      });

      try {
        const first = await firstHost.start();
        const same = await firstHost.start();

        expect(same).toEqual(first);
        expect(first).toEqual(
          expect.objectContaining({
            protocolVersion: 1,
            host: "127.0.0.1",
            server: { name: "ikaros-runtime", version: "0.1.0" }
          })
        );
        expect(first.port).toBeGreaterThan(0);
        expect(first.pid).toBeGreaterThan(0);
        expect(firstHost.pid).toBeGreaterThan(0);
        expect(firstHost.isRunning).toBe(true);

        const created = await firstHost.request<{
          thread: ThreadSummary;
          event: { seq: number; type: string };
        }>("thread.create", { title: "Desktop-owned connection" });
        expect(created.thread.title).toBe("Desktop-owned connection");
        expect(created.thread.defaultBranchId).toMatch(/^branch_/);
        expect(created.event).toEqual(
          expect.objectContaining({ seq: 1, type: "thread.created" })
        );

        const firstTurn = await firstHost.request<{ runId: string }>("turn.start", {
          threadId: created.thread.id,
          branchId: created.thread.defaultBranchId,
          content: "desktop alpha",
          providerId: "scripted",
          modelId: "scripted-v1"
        });
        await vi.waitFor(() => {
          expect(
            events.some(
              (event) => event.type === "run.settled" && event.runId === firstTurn.runId
            )
          ).toBe(true);
        });
        const firstAssistant = events.find(
          (event) =>
            event.type === "item.completed" &&
            event.runId === firstTurn.runId &&
            (event.payload.item as { role?: string } | undefined)?.role === "assistant"
        );
        expect(
          (firstAssistant?.payload.item as { content?: string } | undefined)?.content
        ).toBe("Scripted response to: desktop alpha");
      } finally {
        removeNotification();
        await firstHost.stop();
      }

      expect(firstHost.isRunning).toBe(false);

      const secondHost = new RuntimeHost({ runtimeRoot, runtimeHome });
      try {
        const listed = await secondHost.request<{ threads: ThreadSummary[] }>("thread.list");
        expect(listed.threads).toHaveLength(1);
        expect(listed.threads[0]?.title).toBe("Desktop-owned connection");

        const replay = await secondHost.request<{
          events: Array<{ seq: number; type: string }>;
          latestSeq: number;
          nextAfterSeq: number;
          hasMore: boolean;
        }>("event.replay", { afterSeq: 0 });
        expect(replay.latestSeq).toBeGreaterThan(1);
        expect(replay.nextAfterSeq).toBe(replay.latestSeq);
        expect(replay.hasMore).toBe(false);
        expect(replay.events[0]).toEqual(
          expect.objectContaining({ seq: 1, type: "thread.created" })
        );
        expect(replay.events.at(-1)).toEqual(
          expect.objectContaining({ type: "run.settled" })
        );
      } finally {
        await secondHost.stop();
      }
    } finally {
      await rm(runtimeHome, { recursive: true, force: true });
    }
  });
});
