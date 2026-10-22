import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../store";
import { Composer } from "./Composer";

const initialState = useAppStore.getState();
let resizeCallback: ResizeObserverCallback | undefined;

beforeEach(() => {
  resizeCallback = undefined;
  useAppStore.setState({
        runtimeReady: true,
    runtimeConnectionStatus: "connected",
    providerCatalogStatus: "ready",
  });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: ResizeObserverCallback) {
        resizeCallback = callback;
      }

      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  vi.unstubAllGlobals();
});

describe("Composer input and clearance", () => {
  it("uses one unified conversation surface without a mode selector", () => {
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    expect(screen.queryByRole("button", { name: "Agent" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Chat" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Research" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Full access" })).toBeNull();
    expect(screen.getByRole("button", { name: "Add" })).toHaveClass("size-7");
    expect(screen.getByRole("textbox", { name: "Message Ikaros" })).toBeVisible();
  });

  it("does not submit while an IME composition is being confirmed", () => {
    const sendDraft = vi.fn(async () => undefined);
    useAppStore.setState({
      draft: "你好",
      runStatus: "idle",
      sendDraft,
      providers: [
        {
          id: "deepseek",
          displayName: "DeepSeek",
          origin: "builtin",
          configured: true,
          credentialConfigured: true,
          health: "ready",
        },
      ],
      models: [
        {
          contextWindow: 32768,
          providerId: "deepseek",
          id: "deepseek-chat",
          displayName: "DeepSeek Chat",
          enabled: true,
          reasoningEffort: null,
          reasoningEfforts: [],
        },
      ],
      selectedModel: { providerId: "deepseek", modelId: "deepseek-chat" },
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });
    expect(sendDraft).not.toHaveBeenCalled();

    fireEvent.keyDown(textarea, { key: "Enter", isComposing: false });
    expect(sendDraft).toHaveBeenCalledTimes(1);
  });

  it("adds files and folders from the grouped context menu", async () => {
    const attachComposerContext = vi.fn(async () => undefined);
    useAppStore.setState({ attachComposerContext } as Partial<typeof initialState>);
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: "Add" }));
    expect(await screen.findByText("Add")).toBeInTheDocument();
    const entry = await screen.findByRole("menuitem", { name: "Files and folders" });
    fireEvent.click(entry);

    expect(attachComposerContext).toHaveBeenCalledTimes(1);
  });

  it("shows a fixed localized error when the picker fails", async () => {
    const attachComposerContext = vi.fn(async () => {
      throw new Error("C:\\private\\workspace: directory picker exploded");
    });
    useAppStore.setState({ attachComposerContext } as Partial<typeof initialState>);
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: "Add" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Files and folders" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not add the selection.");
    expect(screen.queryByText(/private|directory picker exploded/i)).toBeNull();
    expect(screen.getByRole("textbox", { name: "Message Ikaros" })).toBeVisible();
  });

  it("lists attached files above the input and removes them", async () => {
    useAppStore.setState({
      composerAttachments: [
        {
          path: "/tmp/notes.txt",
          kind: "text",
          mimeType: "text/plain",
          name: "notes.txt",
        },
        {
          path: "/tmp/spec.md",
          kind: "text",
          mimeType: "text/markdown",
          name: "spec.md",
        },
      ],
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    expect(screen.getByText("notes.txt")).toBeVisible();
    expect(screen.getByText("spec.md")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Remove notes.txt" }));

    expect(useAppStore.getState().composerAttachments).toEqual([
      {
        path: "/tmp/spec.md",
        kind: "text",
        mimeType: "text/markdown",
        name: "spec.md",
      },
    ]);
    expect(screen.queryByText("notes.txt")).toBeNull();
  });

  it("renders image thumbnails and removes them", () => {
    useAppStore.setState({
      composerAttachments: [
        {
          path: "/tmp/pixel.png",
          kind: "image",
          mimeType: "image/png",
          name: "pixel.png",
          previewUrl: "data:image/png;base64,AAAA",
        },
      ],
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    expect(screen.getByAltText("Attached image pixel.png")).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA",
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove pixel.png" }));
    expect(useAppStore.getState().composerAttachments).toEqual([]);
  });

  it("attaches images pasted into the composer", () => {
    const addComposerImageFiles = vi.fn(async () => undefined);
    useAppStore.setState({ addComposerImageFiles } as Partial<typeof initialState>);
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );
    const pasted = new File([new Uint8Array([1, 2, 3])], "pasted.png", { type: "image/png" });

    fireEvent.paste(screen.getByRole("textbox", { name: "Message Ikaros" }), {
      clipboardData: {
        items: [{ kind: "file", type: "image/png", getAsFile: () => pasted }],
      },
    });

    expect(addComposerImageFiles).toHaveBeenCalledWith([pasted]);
  });

  it("attaches files dropped onto the composer", () => {
    const addComposerImageFiles = vi.fn(async () => undefined);
    useAppStore.setState({ addComposerImageFiles } as Partial<typeof initialState>);
    const { container } = render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );
    const surface = container.querySelector<HTMLElement>("[data-composer-surface]");
    if (!surface) throw new Error("composer surface was not rendered");
    const dropped = new File([new Uint8Array([1])], "dropped.png", { type: "image/png" });

    fireEvent.dragOver(surface, { dataTransfer: { types: ["Files"], files: [dropped] } });
    expect(screen.getByText("Drop files to attach")).toBeVisible();
    fireEvent.drop(surface, { dataTransfer: { types: ["Files"], files: [dropped] } });

    expect(addComposerImageFiles).toHaveBeenCalledWith([dropped]);
    expect(screen.queryByText("Drop files to attach")).toBeNull();
  });

  it("uses one primary button for steer and Stop while a run is active", () => {
    const sendDraft = vi.fn(async () => undefined);
    const steerRun = vi.fn(async () => undefined);
    const stopRun = vi.fn();
    useAppStore.setState({
      draft: "Write the next message",
      runStatus: "running",
            sendDraft,
      steerRun,
      stopRun,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.keyDown(textarea, { key: "Enter" });
    expect(sendDraft).not.toHaveBeenCalled();
    expect(steerRun).toHaveBeenCalledTimes(1);
    expect(stopRun).not.toHaveBeenCalled();

    act(() => useAppStore.setState({ draft: "" }));
    expect(screen.queryByRole("button", { name: "Send requirement" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(stopRun).toHaveBeenCalledTimes(1);
  });

  it("lists models under a header with a trailing check for the selected one", async () => {
    useAppStore.setState({
      providers: [
        {
          id: "deepseek",
          displayName: "DeepSeek",
          origin: "builtin",
          configured: true,
          credentialConfigured: true,
          health: "ready",
        },
      ],
      models: [
        {
          contextWindow: 32768,
          providerId: "deepseek",
          id: "deepseek-flash",
          displayName: "DeepSeek Flash",
          enabled: true,
          reasoningEffort: null,
          reasoningEfforts: [],
        },
        {
          contextWindow: 32768,
          providerId: "deepseek",
          id: "deepseek-v4",
          displayName: "DeepSeek V4.1 Flash",
          enabled: true,
          reasoningEffort: null,
          reasoningEfforts: [],
        },
      ],
      selectedModel: { providerId: "deepseek", modelId: "deepseek-flash" },
      runtimeConnectionStatus: "connected",
      providerCatalogStatus: "ready",
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: "DeepSeek Flash" }));

    expect(await screen.findByText("Select a model")).toBeInTheDocument();
    const selectedItem = await screen.findByRole("menuitem", { name: "DeepSeek Flash" });
    const otherItem = await screen.findByRole("menuitem", { name: "DeepSeek V4.1 Flash" });
    expect(selectedItem.querySelector("svg")).not.toBeNull();
    expect(otherItem.querySelector("svg")).toBeNull();
  });

  it("shows the staged project above the input for a new project chat", () => {
    useAppStore.setState({
      newThreadWorkspace: {
        id: "project-research",
        name: "Research",
        kind: "project",
        rootUri: "/workspace/research",
      },
      selectedThreadId: null,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const header = document.querySelector("[data-composer-project]");
    expect(header).not.toBeNull();
    expect(header?.textContent).toContain("Research");

    act(() => useAppStore.setState({ newThreadWorkspace: null }));
    expect(document.querySelector("[data-composer-project]")).toBeNull();
  });

  it("hides the staged project while the first message is pending", () => {
    useAppStore.setState({
      newThreadWorkspace: {
        id: "project-research",
        name: "Research",
        kind: "project",
        rootUri: "/workspace/research",
      },
      selectedThreadId: null,
      pendingRuntimeNewThread: { epoch: 1 } as never,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    expect(document.querySelector("[data-composer-project]")).toBeNull();
  });

  it("changes the reasoning level from the model submenu", async () => {
    const setModelLimits = vi.fn(async () => undefined);
    useAppStore.setState({
      providers: [
        {
          id: "deepseek",
          displayName: "DeepSeek",
          origin: "builtin",
          configured: true,
          credentialConfigured: true,
          health: "ready",
        },
      ],
      models: [
        {
          contextWindow: 32768,
          providerId: "deepseek",
          id: "deepseek-flash",
          displayName: "DeepSeek Flash",
          enabled: true,
          reasoningEffort: "high",
          reasoningEfforts: ["low", "high", "max"],
        },
      ],
      selectedModel: { providerId: "deepseek", modelId: "deepseek-flash" },
      runtimeConnectionStatus: "connected",
      providerCatalogStatus: "ready",
      setModelLimits,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: /DeepSeek Flash/ }));
    const subTrigger = await screen.findByRole("menuitem", { name: /DeepSeek Flash/ });
    fireEvent.keyDown(subTrigger, { key: "ArrowRight" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "max" }));

    await waitFor(() =>
      expect(setModelLimits).toHaveBeenCalledWith({
        providerId: "deepseek",
        modelId: "deepseek-flash",
        contextWindow: 32768,
        reasoningEfforts: ["low", "high", "max"],
        reasoningEffort: "max",
      }),
    );
  });

  it("clears the level when the active entry is picked again and hides Auto", async () => {
    const setModelLimits = vi.fn(async () => undefined);
    useAppStore.setState({
      providers: [
        {
          id: "deepseek",
          displayName: "DeepSeek",
          origin: "builtin",
          configured: true,
          credentialConfigured: true,
          health: "ready",
        },
      ],
      models: [
        {
          contextWindow: 32768,
          providerId: "deepseek",
          id: "deepseek-flash",
          displayName: "DeepSeek Flash",
          enabled: true,
          reasoningEffort: "max",
          reasoningEfforts: ["low", "high", "max"],
        },
      ],
      selectedModel: { providerId: "deepseek", modelId: "deepseek-flash" },
      runtimeConnectionStatus: "connected",
      providerCatalogStatus: "ready",
      setModelLimits,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: /DeepSeek Flash/ }));
    const subTrigger = await screen.findByRole("menuitem", { name: /DeepSeek Flash/ });
    fireEvent.keyDown(subTrigger, { key: "ArrowRight" });

    expect(screen.queryByRole("menuitem", { name: "Auto" })).toBeNull();
    fireEvent.click(await screen.findByRole("menuitem", { name: "max" }));

    await waitFor(() =>
      expect(setModelLimits).toHaveBeenCalledWith({
        providerId: "deepseek",
        modelId: "deepseek-flash",
        contextWindow: 32768,
        reasoningEfforts: ["low", "high", "max"],
        reasoningEffort: null,
      }),
    );
  });

  it("does not offer a reasoning submenu when the model has no levels", async () => {
    useAppStore.setState({
      providers: [
        {
          id: "deepseek",
          displayName: "DeepSeek",
          origin: "builtin",
          configured: true,
          credentialConfigured: true,
          health: "ready",
        },
      ],
      models: [
        {
          contextWindow: 32768,
          providerId: "deepseek",
          id: "deepseek-flash",
          displayName: "DeepSeek Flash",
          enabled: true,
          reasoningEffort: null,
          reasoningEfforts: [],
        },
      ],
      selectedModel: { providerId: "deepseek", modelId: "deepseek-flash" },
      runtimeConnectionStatus: "connected",
      providerCatalogStatus: "ready",
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: "DeepSeek Flash" }));
    const item = await screen.findByRole("menuitem", { name: "DeepSeek Flash" });

    expect(item.getAttribute("aria-haspopup")).toBeNull();
  });

  it("stays quiet without a configured provider and keeps the draft, thread, and workspace", () => {
    const sendDraft = vi.fn(async () => undefined);
    const workspace = {
      id: "workspace-setup",
      name: "Setup",
      kind: "project" as const,
      rootUri: "/workspace/setup",
    };
    useAppStore.setState({
            providers: [],
      models: [],
      selectedModel: null,
      draft: "hello",
      selectedThreadId: "thread-setup",
      newThreadWorkspace: workspace,
      runStatus: "idle",
      sendDraft,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    // First-run setup lives in the onboarding dialog: the composer shows no hint.
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
    expect(
      screen.queryByText("Connect a provider and configure a model to send messages."),
    ).toBeNull();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Message Ikaros" }), {
      key: "Enter",
    });
    expect(sendDraft).not.toHaveBeenCalled();
    expect(useAppStore.getState()).toMatchObject({
      settingsOpen: false,
      draft: "hello",
      selectedThreadId: "thread-setup",
      newThreadWorkspace: workspace,
    });
  });

  it("opens Models settings when configured models are disabled", () => {
    useAppStore.setState({
            providers: [{ id: "custom", displayName: "Custom", origin: "custom", configured: true, credentialConfigured: true, health: "unknown" }],
      models: [
        {
          providerId: "custom",
          id: "model",
          displayName: "Model",
          enabled: false,
          contextWindow: 32768,
          reasoningEffort: null,
          reasoningEfforts: [],
        },
      ],
      selectedModel: null,
      draft: "saved draft",
    });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    expect(screen.getByText("The configured models are disabled. Enable one in Models settings.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Enable a model" }));
    expect(useAppStore.getState()).toMatchObject({ settingsOpen: true, settingsInitialSection: "models", draft: "saved draft" });
  });

  it.each(["idle", "loading"] as const)("keeps %s catalogs distinct from missing configuration", (providerCatalogStatus) => {
    useAppStore.setState({ providerCatalogStatus, providers: [], models: [], draft: "saved draft" });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    expect(screen.getByRole("button", { name: "Loading models…" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  });

  it("retries a failed model catalog without opening configuration or discarding the draft", () => {
    const loadProviderCatalog = vi.fn(async () => undefined);
    useAppStore.setState({ providerCatalogStatus: "error", providers: [], models: [], draft: "saved draft", loadProviderCatalog });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    fireEvent.click(screen.getByRole("button", { name: "Reload models" }));
    expect(loadProviderCatalog).toHaveBeenCalledOnce();
    expect(useAppStore.getState()).toMatchObject({ draft: "saved draft", settingsOpen: false });
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
  });

  it("stays quiet while the offline dialog owns the reconnect affordance", () => {
    useAppStore.setState({ runtimeConnectionStatus: "offline", providerCatalogStatus: "error", draft: "saved draft" });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);

    expect(screen.queryByRole("button", { name: "Reconnect" })).toBeNull();
    expect(screen.queryByText(/The local runtime is offline/)).toBeNull();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
    expect(useAppStore.getState().draft).toBe("saved draft");
  });

  it("keeps the composer quiet when the only provider is disconnected", () => {
    useAppStore.setState({
            providers: [{ id: "deepseek", displayName: "DeepSeek", origin: "builtin", configured: false, credentialConfigured: false, health: "unknown" }],
      models: [],
      selectedModel: null,
      draft: "saved draft",
    });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(useAppStore.getState().draft).toBe("saved draft");
  });

  it("sends the preserved draft after configuration supplies a runnable model", () => {
    const sendDraft = vi.fn(async () => undefined);
    useAppStore.setState({
            providers: [],
      models: [],
      selectedModel: null,
      draft: "Resume my original request",
      runStatus: "idle",
      sendDraft,
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();

    act(() => {
      useAppStore.setState({
        providers: [{
          id: "custom",
          displayName: "Custom",
          origin: "custom",
          configured: true,
          credentialConfigured: true,
          health: "unknown",
        }],
        models: [
          {
            providerId: "custom",
            id: "model",
            displayName: "Model",
            enabled: true,
            contextWindow: 32768,
            reasoningEffort: null,
            reasoningEfforts: [],
          },
        ],
        selectedModel: { providerId: "custom", modelId: "model" },
      });
      useAppStore.getState().setSettingsOpen(false);
    });

    expect(screen.getByRole("textbox", { name: "Message Ikaros" })).toHaveValue("Resume my original request");
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(sendDraft).toHaveBeenCalledOnce();
  });

  it("lets Runtime users change the selected model without showing a selection instruction", () => {
    useAppStore.setState({
            providers: [
        {
          id: "test-provider",
          displayName: "Test Provider",
          origin: "custom",
          configured: true,
          credentialConfigured: false,
          health: "unknown",
        },
      ],
      models: [
        {
          providerId: "test-provider",
          id: "model-a",
          displayName: "Model A",
          contextWindow: 32768,
          enabled: true,
          reasoningEffort: null,
          reasoningEfforts: [],
        },
        {
          providerId: "test-provider",
          id: "model-b",
          displayName: "Model B",
          contextWindow: 32768,
          enabled: true,
          reasoningEffort: null,
          reasoningEfforts: [],
        },
      ],
      selectedModel: { providerId: "test-provider", modelId: "model-a" },
      draft: "hello",
      runStatus: "idle",
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const modelTrigger = screen.getByRole("button", { name: "Model A" });
    expect(modelTrigger).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
    expect(screen.queryByRole("status")).toBeNull();
    fireEvent.pointerDown(modelTrigger);
    expect(screen.queryByText(/Test Provider/)).not.toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Model A" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitem", { name: "Model B" }));

    expect(useAppStore.getState().selectedModel).toEqual({
      providerId: "test-provider",
      modelId: "model-b",
    });
    expect(screen.getByRole("button", { name: "Model B" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
  });

  it("reports clearance from the measured composer surface instead of a fixed height", () => {
    const onClearanceChange = vi.fn();
    const { container } = render(
      <Tooltip.Provider>
        <Composer onClearanceChange={onClearanceChange} />
      </Tooltip.Provider>,
    );
    const overlay = container.querySelector<HTMLElement>("[data-composer-overlay]");
    const surface = container.querySelector<HTMLElement>("[data-composer-surface]");
    if (!overlay || !surface || !resizeCallback) throw new Error("Composer measurement was not initialized");

    vi.spyOn(overlay, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 337, 960, 303));
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 389, 738, 247));
    act(() => resizeCallback?.([], {} as ResizeObserver));

    expect(onClearanceChange).toHaveBeenLastCalledWith(275);
  });
});
