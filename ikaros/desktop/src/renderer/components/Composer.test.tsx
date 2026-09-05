import "@testing-library/jest-dom/vitest";
import * as Tooltip from "@radix-ui/react-tooltip";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../store";
import { Composer } from "./Composer";

const initialState = useAppStore.getState();
let resizeCallback: ResizeObserverCallback | undefined;

beforeEach(() => {
  resizeCallback = undefined;
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
    expect(screen.getByRole("button", { name: "Full access" })).toHaveClass(
      "h-7",
      "text-[10px]",
    );
    expect(screen.getByRole("button", { name: "Add context" })).toHaveClass("size-7");
    expect(screen.getByRole("button", { name: "Ikaros" })).toBeVisible();
  });

  it("does not submit while an IME composition is being confirmed", () => {
    const sendDraft = vi.fn(async () => undefined);
    useAppStore.setState({ draft: "你好", runStatus: "idle", sendDraft });
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

  it("opens the slash command palette and inserts the keyboard-selected command", () => {
    const sendDraft = vi.fn(async () => undefined);
    useAppStore.setState({ draft: "", runStatus: "idle", sendDraft });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.change(textarea, { target: { value: "/" } });

    expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeVisible();
    expect(screen.getAllByRole("option")).toHaveLength(3);
    fireEvent.keyDown(textarea, { key: "ArrowDown" });
    expect(screen.getByRole("option", { name: /Mock feature 2/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(useAppStore.getState().draft).toBe("/mock-2 ");
    expect(screen.queryByRole("listbox", { name: "Slash commands" })).toBeNull();
    expect(sendDraft).not.toHaveBeenCalled();
  });

  it("filters slash commands and lets Escape dismiss the palette", () => {
    useAppStore.setState({ draft: "", runStatus: "idle" });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.change(textarea, { target: { value: "/3" } });
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(screen.getByRole("option", { name: /Mock feature 3/ })).toBeVisible();

    fireEvent.keyDown(textarea, { key: "Escape" });
    expect(screen.queryByRole("listbox", { name: "Slash commands" })).toBeNull();
    expect(useAppStore.getState().draft).toBe("/3");
  });

  it("does not open the slash palette during IME composition", () => {
    useAppStore.setState({ draft: "", runStatus: "idle" });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const textarea = screen.getByRole("textbox", { name: "Message Ikaros" });
    fireEvent.compositionStart(textarea);
    fireEvent.change(textarea, { target: { value: "/" } });
    expect(screen.queryByRole("listbox", { name: "Slash commands" })).toBeNull();

    fireEvent.compositionEnd(textarea);
    expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeVisible();
  });

  it("binds a selected project folder from the context menu", async () => {
    const bindWorkspaceFromFolder = vi.fn(async () => undefined);
    useAppStore.setState({ bindWorkspaceFromFolder } as Partial<typeof initialState>);
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: "Add context" }));
    const projectFolderItem = await screen.findByRole("menuitem", {
      name: "Add project folder",
    });
    expect(projectFolderItem).not.toHaveTextContent("Unavailable");
    fireEvent.click(projectFolderItem);

    expect(bindWorkspaceFromFolder).toHaveBeenCalledTimes(1);
  });

  it("shows a fixed localized error when project folder binding fails", async () => {
    const bindWorkspaceFromFolder = vi.fn(async () => {
      throw new Error("C:\\private\\workspace: directory picker exploded");
    });
    useAppStore.setState({ bindWorkspaceFromFolder } as Partial<typeof initialState>);
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    fireEvent.pointerDown(screen.getByRole("button", { name: "Add context" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Add project folder" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not add project folder.");
    expect(screen.queryByText(/private|directory picker exploded/i)).toBeNull();
    expect(screen.getByRole("textbox", { name: "Message Ikaros" })).toBeVisible();
  });

  it("does not turn Enter into Stop while a run is active", () => {
    const sendDraft = vi.fn(async () => undefined);
    const stopRun = vi.fn();
    useAppStore.setState({
      draft: "Write the next message",
      runStatus: "running",
      runtimeMode: true,
      sendDraft,
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
    expect(stopRun).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(stopRun).toHaveBeenCalledTimes(1);
  });

  it("opens provider configuration without losing the draft, thread, or workspace", () => {
    const sendDraft = vi.fn(async () => undefined);
    const workspace = { id: "workspace-setup", name: "Setup", rootUri: "/workspace/setup" };
    useAppStore.setState({
      runtimeMode: true,
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

    expect(screen.getByRole("button", { name: "Full access" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Configure a model" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Message Ikaros" }), {
      key: "Enter",
    });
    expect(sendDraft).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Configure a model" }));
    expect(useAppStore.getState()).toMatchObject({
      settingsOpen: true,
      settingsInitialSection: "providers",
      draft: "hello",
      selectedThreadId: "thread-setup",
      newThreadWorkspace: workspace,
    });
    useAppStore.getState().setSettingsOpen(false);
    expect(useAppStore.getState().draft).toBe("hello");
  });

  it("opens Models settings when configured models are disabled", () => {
    useAppStore.setState({
      runtimeMode: true,
      providers: [{ id: "custom", displayName: "Custom", origin: "custom", configured: true, credentialConfigured: true, health: "unknown" }],
      models: [{ providerId: "custom", id: "model", displayName: "Model", enabled: false }],
      selectedModel: null,
      draft: "saved draft",
    });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    expect(screen.getByText("The configured models are disabled. Enable one in Models settings.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Enable a model" }));
    expect(useAppStore.getState()).toMatchObject({ settingsOpen: true, settingsInitialSection: "models", draft: "saved draft" });
  });

  it.each(["idle", "loading"] as const)("keeps %s catalogs distinct from missing configuration", (providerCatalogStatus) => {
    useAppStore.setState({ runtimeMode: true, providerCatalogStatus, providers: [], models: [], draft: "saved draft" });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    expect(screen.getByRole("button", { name: "Loading models…" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  });

  it("retries a failed model catalog without opening configuration or discarding the draft", () => {
    const loadProviderCatalog = vi.fn(async () => undefined);
    useAppStore.setState({ runtimeMode: true, providerCatalogStatus: "error", providers: [], models: [], draft: "saved draft", loadProviderCatalog });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    fireEvent.click(screen.getByRole("button", { name: "Reload models" }));
    expect(loadProviderCatalog).toHaveBeenCalledOnce();
    expect(useAppStore.getState()).toMatchObject({ draft: "saved draft", settingsOpen: false });
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
  });

  it("reconnects the Runtime instead of treating an offline catalog as unconfigured", () => {
    const retryRuntimeConnection = vi.fn(async () => undefined);
    useAppStore.setState({ runtimeMode: true, runtimeConnectionStatus: "offline", providerCatalogStatus: "error", draft: "saved draft", retryRuntimeConnection });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(retryRuntimeConnection).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Configure a model" })).toBeNull();
    expect(useAppStore.getState().draft).toBe("saved draft");
  });

  it("makes configuration available after a provider is disconnected", () => {
    useAppStore.setState({
      runtimeMode: true,
      providers: [{ id: "deepseek", displayName: "DeepSeek", origin: "builtin", configured: false, credentialConfigured: false, health: "unknown" }],
      models: [],
      selectedModel: null,
      draft: "saved draft",
    });
    render(<Tooltip.Provider><Composer onClearanceChange={() => undefined} /></Tooltip.Provider>);
    fireEvent.click(screen.getByRole("button", { name: "Configure a model" }));
    expect(useAppStore.getState()).toMatchObject({ settingsInitialSection: "providers", draft: "saved draft" });
  });

  it("sends the preserved draft after configuration supplies a runnable model", () => {
    const sendDraft = vi.fn(async () => undefined);
    useAppStore.setState({
      runtimeMode: true,
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
    fireEvent.click(screen.getByRole("button", { name: "Configure a model" }));

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
        models: [{ providerId: "custom", id: "model", displayName: "Model", enabled: true }],
        selectedModel: { providerId: "custom", modelId: "model" },
      });
      useAppStore.getState().setSettingsOpen(false);
    });

    expect(screen.getByRole("textbox", { name: "Message Ikaros" })).toHaveValue("Resume my original request");
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    expect(sendDraft).toHaveBeenCalledOnce();
  });

  it("lets Runtime users explicitly choose between multiple runnable models", () => {
    useAppStore.setState({
      runtimeMode: true,
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
          enabled: true,
        },
        {
          providerId: "test-provider",
          id: "model-b",
          displayName: "Model B",
          enabled: true,
        },
      ],
      selectedModel: null,
      draft: "hello",
      runStatus: "idle",
    });
    render(
      <Tooltip.Provider>
        <Composer onClearanceChange={() => undefined} />
      </Tooltip.Provider>,
    );

    const modelTrigger = screen.getByRole("button", { name: "Select a model" });
    expect(modelTrigger).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
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
    expect(screen.getByRole("button", { name: "Full access" })).toBeDisabled();
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
