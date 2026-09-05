import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  cloneUiPreferences,
  DEFAULT_UI_PREFERENCES,
  mergeUiPreferences,
  type IkarosDesktopApi,
  type UiPreferences,
  type UiPreferencesPatch
} from "../../shared/platform";
import type {
  RuntimeModelSetEnabledParams,
  RuntimeProviderConfigureParams
} from "../../shared/runtime";
import { themeTransitionCoordinator } from "../applyUiPreferences";
import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { SettingsPage } from "./SettingsPage";

const initialState = useAppStore.getState();

function desktopApiWithPreferences(
  updateImplementation?: (patch: UiPreferencesPatch) => Promise<UiPreferences>
): {
  api: IkarosDesktopApi;
  update: ReturnType<typeof vi.fn>;
  listMemories: ReturnType<typeof vi.fn>;
} {
  let current = cloneUiPreferences(DEFAULT_UI_PREFERENCES);
  const update = vi.fn(
    updateImplementation ??
      (async (patch: UiPreferencesPatch) => {
        current = mergeUiPreferences(current, patch);
        return cloneUiPreferences(current);
      })
  );
  const listMemories = vi.fn(async () => ({
    ok: true as const,
    value: { memories: [], nextCursor: null, hasMore: false }
  }));

  return {
    update,
    listMemories,
    api: {
      runtime: {
        listThreads: async () => ({
          ok: true,
          value: {
            threads: [],
            nextCursor: null,
            hasMore: false,
            snapshotSeq: 0,
          },
        }),
        getThread: async () => {
          throw new Error("not used in settings tests");
        },
        listTurns: async () => {
          throw new Error("not used in settings tests");
        },
        createThread: async () => {
          throw new Error("not used in settings tests");
        },
        renameThread: async () => {
          throw new Error("not used in settings tests");
        },
        archiveThread: async () => {
          throw new Error("not used in settings tests");
        },
        unarchiveThread: async () => {
          throw new Error("not used in settings tests");
        },
        startTurn: async () => {
          throw new Error("not used in settings tests");
        },
        cancelRun: async () => {
          throw new Error("not used in settings tests");
        },
        replayEvents: async () => ({
          ok: true,
          value: {
            events: [],
            latestSeq: 0,
            nextAfterSeq: 0,
            hasMore: false
          }
        }),
        listProviders: async () => ({
          ok: true,
          value: {
            providers: [
              {
                id: "deepseek",
                displayName: "DeepSeek",
                origin: "builtin",
                configured: false,
                credentialConfigured: false,
                health: "unknown"
              }
            ]
          }
        }),
        configureProvider: async () => {
          throw new Error("not used in preference settings tests");
        },
        discoverProviderModels: async () => {
          throw new Error("not used in preference settings tests");
        },
        disconnectProvider: async () => {
          throw new Error("not used in preference settings tests");
        },
        removeProvider: async () => {
          throw new Error("not used in preference settings tests");
        },
        listModels: async () => ({ ok: true, value: { models: [] } }),
        setModelEnabled: async () => {
          throw new Error("not used in preference settings tests");
        },
        listSkills: async () => ({ ok: true, value: { skills: [], diagnostics: [] } }),
        setSkillEnabled: async () => {
          throw new Error("not used in preference settings tests");
        },
        createMemory: async () => {
          throw new Error("not used in preference settings tests");
        },
        correctMemory: async () => {
          throw new Error("not used in preference settings tests");
        },
        forgetMemory: async () => {
          throw new Error("not used in preference settings tests");
        },
        listMemories,
        getMemory: async () => {
          throw new Error("not used in preference settings tests");
        },
        previewFile: async () => {
          throw new Error("not used in preference settings tests");
        },
        getFileChange: async () => {
          throw new Error("not used in preference settings tests");
        },
        readUsage: async () => ({
          ok: true,
          value: {
            summary: {
              lifetimeTokens: null,
              peakDailyTokens: null,
              longestRunningTurnSec: null,
              currentStreakDays: 0,
              longestStreakDays: 0
            },
            dailyUsageBuckets: []
          }
        }),
        onEvent: () => () => undefined
      },
      workspace: {
        chooseDirectory: async () => null
      },
      preferences: {
        get: async () => cloneUiPreferences(current),
        update,
        onChanged: () => () => undefined
      },
      windowControls: {
        usesCustomTitleBar: false,
        close: async () => undefined,
        minimize: async () => undefined,
        toggleMaximize: async () => undefined
      }
    }
  };
}

beforeEach(() => {
  setUiLanguage("en");
  useAppStore.setState({ settingsOpen: true });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn((query: string) => ({
      matches: query === "(prefers-color-scheme: dark)",
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn()
    }))
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  themeTransitionCoordinator.dispose();
  Reflect.deleteProperty(document, "startViewTransition");
  Reflect.deleteProperty(window, "ikarosDesktop");
  Reflect.deleteProperty(window, "matchMedia");
  document.documentElement.removeAttribute("style");
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-color-scheme");
  document.documentElement.removeAttribute("data-translucent-sidebar");
  setUiLanguage("en");
  useAppStore.setState(initialState, true);
});

describe("SettingsPage", () => {
  it.each([
    ["providers", "Providers"],
    ["models", "Models"],
  ] as const)("opens the requested %s section and returns with the conversation intact", (section, heading) => {
    const workspace = { id: "workspace-setup", name: "Setup", rootUri: "/work/setup" };
    useAppStore.setState({
      draft: "Continue this task",
      selectedThreadId: "thread-setup",
      newThreadWorkspace: workspace,
    });
    useAppStore.getState().setSettingsOpen(true, section);

    render(<SettingsPage />);

    expect(screen.getByRole("heading", { level: 1, name: heading })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back to app" }));
    expect(useAppStore.getState()).toMatchObject({
      settingsOpen: false,
      draft: "Continue this task",
      selectedThreadId: "thread-setup",
      newThreadWorkspace: workspace,
    });
  });

  it("manages archived conversations from General settings", async () => {
    const loadArchivedThreads = vi.fn(async () => undefined);
    const unarchiveThread = vi.fn(async () => undefined);
    useAppStore.setState({
      archivedCatalogStatus: "ready",
      archivedThreads: [
        {
          id: "thread-archived",
          title: "Archived research",
          defaultBranchId: "branch-archived",
          workspace: { id: "workspace-research", name: "Research", rootUri: null },
          createdAt: "2026-08-14T00:00:00.000Z",
          updatedAt: "2026-08-14T01:00:00.000Z",
          archivedAt: "2026-08-14T01:00:00.000Z",
        },
      ],
      loadArchivedThreads,
      unarchiveThread,
    });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Manage" }));

    expect(await screen.findByRole("dialog")).toBeTruthy();
    expect(screen.getByText("Archived research")).toBeTruthy();
    expect(screen.getByText("Research")).toBeTruthy();
    expect(loadArchivedThreads).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Unarchive" }));
    await waitFor(() => expect(unarchiveThread).toHaveBeenCalledWith("thread-archived"));
  });

  it("keeps General and adds local Profile and Appearance sections", () => {
    render(<SettingsPage />);

    expect(screen.getByRole("heading", { level: 1, name: "General" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Profile" }));

    expect(screen.getByRole("heading", { level: 1, name: "Profile" })).toBeTruthy();
    expect(screen.getByText("US")).toBeTruthy();
    expect(screen.getByText("User")).toBeTruthy();
    expect(screen.queryByText(/@/)).toBeNull();
    expect(screen.queryByText("Free")).toBeNull();
    expect(screen.queryByText("Share")).toBeNull();
    expect(screen.queryByText("Private")).toBeNull();
    expect(screen.getByRole("button", { name: "Edit" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    expect(screen.getByRole("heading", { level: 1, name: "Appearance" })).toBeTruthy();
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "System" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Light" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Dark" }).getAttribute("aria-checked")).toBe(
      "true"
    );
    expect(screen.getByRole("slider", { name: "Contrast" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Translucent sidebar" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Providers" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Models" })).toBeTruthy();
  });

  it("lazy-loads Memory only after navigation and keeps the stable Workspace ID", async () => {
    const { api, listMemories } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", {
      configurable: true,
      value: api
    });
    useAppStore.setState({
      projects: [
        {
          id: "workspace-stable-id",
          name: "Ikaros",
          color: "#ffffff",
          rootUri: "C:/Workspace/github/Ikaros"
        }
      ],
      selectedThreadId: null,
      newThreadWorkspace: {
        id: "workspace-stable-id",
        name: "Ikaros",
        rootUri: "C:/Workspace/github/Ikaros"
      }
    });

    render(<SettingsPage />);
    expect(listMemories).not.toHaveBeenCalled();

    fireEvent.change(screen.getByPlaceholderText("Search settings..."), {
      target: { value: "memory" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Memory" }));

    expect(await screen.findByRole("heading", { level: 1, name: "Memory" })).toBeTruthy();
    await waitFor(() =>
      expect(listMemories).toHaveBeenCalledWith({ limit: 25, state: "active" })
    );
    fireEvent.click(screen.getByRole("button", { name: "Add memory" }));
    expect((screen.getByLabelText("Scope") as HTMLSelectElement).value).toBe(
      "workspace:workspace-stable-id"
    );
  });

  it("requires an explicit model and configures DeepSeek without retaining its secret", async () => {
    const configureProvider = vi.fn(async (params: RuntimeProviderConfigureParams) => {
      if (params.kind !== "deepseek") throw new Error("expected DeepSeek configuration");
      useAppStore.setState({
        providers: [
          {
            id: "deepseek",
            displayName: "DeepSeek",
            origin: "builtin",
            configured: true,
            credentialConfigured: true,
            health: "unknown"
          }
        ],
        models: params.models.map((model) => ({
          providerId: "deepseek",
          id: model.id,
          displayName: model.displayName,
          enabled: true
        }))
      });
    });
    const setModelEnabled = vi.fn(async (params: RuntimeModelSetEnabledParams) => {
      useAppStore.setState((state) => ({
        models: state.models.map((model) =>
          model.providerId === params.providerId && model.id === params.modelId
            ? { ...model, enabled: params.enabled }
            : model
        )
      }));
    });
    useAppStore.setState({
      providers: [
        {
          id: "deepseek",
          displayName: "DeepSeek",
          origin: "builtin",
          configured: false,
          credentialConfigured: false,
          health: "unknown"
        }
      ],
      models: [],
      configureProvider,
      setModelEnabled
    });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));

    expect(screen.getByRole("heading", { level: 1, name: "Providers" })).toBeTruthy();
    expect(screen.getByText("DeepSeek")).toBeTruthy();
    expect(screen.getByText("Custom provider")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Connect DeepSeek" }));
    const dialog = screen.getByRole("dialog", { name: "Connect DeepSeek" });
    expect(within(dialog).queryByRole("button", { name: "Back" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "Close" })).toBeNull();
    expect(screen.getByText("Providers", { selector: "h1" })).toBeTruthy();
    const apiKey = screen.getByLabelText("DeepSeek API key") as HTMLInputElement;
    const continueButton = screen.getByRole("button", { name: "Continue" }) as HTMLButtonElement;
    expect(apiKey.type).toBe("password");
    expect(continueButton.disabled).toBe(true);

    fireEvent.change(apiKey, { target: { value: "mock-secret" } });
    expect(continueButton.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "deepseek-chat" }
    });
    fireEvent.change(screen.getByLabelText("Model display name"), {
      target: { value: "DeepSeek Chat" }
    });
    expect(continueButton.disabled).toBe(false);
    fireEvent.click(continueButton);

    await waitFor(() =>
      expect(configureProvider).toHaveBeenCalledWith({
        kind: "deepseek",
        apiKey: "mock-secret",
        models: [{ id: "deepseek-chat", displayName: "DeepSeek Chat" }]
      })
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "Disconnect DeepSeek" })).toBeTruthy();
    expect(screen.queryByDisplayValue("mock-secret")).toBeNull();
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("heading", { level: 1, name: "Providers" })
      )
    );

    fireEvent.click(screen.getByRole("button", { name: "Models" }));
    expect(screen.getByRole("heading", { level: 1, name: "Models" })).toBeTruthy();
    expect(screen.getByText("DeepSeek Chat")).toBeTruthy();
    expect(screen.queryByText("DeepSeek V4 Flash")).toBeNull();
    expect(screen.queryByText("DeepSeek V4 Pro")).toBeNull();

    const modelSwitch = screen.getByRole("switch", {
      name: "Toggle DeepSeek Chat"
    });
    expect(modelSwitch.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(modelSwitch);
    await waitFor(() =>
      expect(setModelEnabled).toHaveBeenCalledWith({
        providerId: "deepseek",
        modelId: "deepseek-chat",
        enabled: false
      })
    );
    expect(modelSwitch.getAttribute("aria-checked")).toBe("false");
  });

  it("removes the DeepSeek model group after disconnecting the provider", async () => {
    const disconnectedProvider = {
      id: "deepseek",
      displayName: "DeepSeek",
      origin: "builtin" as const,
      configured: false,
      credentialConfigured: false,
      health: "unknown" as const
    };
    const disconnectProvider = vi.fn(async (providerId: "deepseek") => {
      expect(providerId).toBe("deepseek");
      useAppStore.setState({ providers: [disconnectedProvider], models: [] });
    });
    useAppStore.setState({
      providers: [
        {
          ...disconnectedProvider,
          configured: true,
          credentialConfigured: true
        }
      ],
      models: [
        {
          providerId: "deepseek",
          id: "deepseek-chat",
          displayName: "DeepSeek Chat",
          enabled: true
        }
      ],
      disconnectProvider
    });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    fireEvent.click(screen.getByRole("button", { name: "Disconnect DeepSeek" }));

    await waitFor(() => expect(disconnectProvider).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Connect DeepSeek" })).toBeTruthy()
    );
    fireEvent.click(screen.getByRole("button", { name: "Models" }));

    expect(screen.getByRole("heading", { level: 1, name: "Models" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "DeepSeek" })).toBeNull();
    expect(screen.queryByText("DeepSeek Chat")).toBeNull();
    expect(screen.getByText("Connect a provider to manage its models.")).toBeTruthy();
  });

  it("fetches DeepSeek models without replacing a manual row until one is selected", async () => {
    const discoverDeepSeekModels = vi.fn(async (apiKey: string) => {
      expect(apiKey).toBe("discovery-secret");
      return [
        { id: "deepseek-chat", displayName: "DeepSeek Chat" },
        { id: "deepseek-reasoner", displayName: "DeepSeek Reasoner" }
      ];
    });
    useAppStore.setState({ providers: [], models: [], discoverDeepSeekModels });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect DeepSeek" }));

    const fetchButton = screen.getByRole("button", { name: "Fetch models" });
    const chooseButton = screen.getByRole("button", {
      name: "Choose a fetched model for row 1"
    });
    const displayNameInput = screen.getByLabelText("Model display name");
    const modelIdInput = screen.getByLabelText("Model ID");
    expect(
      displayNameInput.compareDocumentPosition(modelIdInput) &
        Node.DOCUMENT_POSITION_FOLLOWING
    ).not.toBe(0);
    expect((displayNameInput as HTMLInputElement).placeholder).toBe("Display name");
    expect((modelIdInput as HTMLInputElement).placeholder).toBe("model-id");
    expect((fetchButton as HTMLButtonElement).disabled).toBe(true);
    expect((chooseButton as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "manual-model" }
    });
    fireEvent.change(screen.getByLabelText("Model display name"), {
      target: { value: "Manual model" }
    });
    fireEvent.change(screen.getByLabelText("DeepSeek API key"), {
      target: { value: "discovery-secret" }
    });
    expect((fetchButton as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(fetchButton);

    expect(await screen.findByText("Fetched 2 models.")).toBeTruthy();
    expect(discoverDeepSeekModels).toHaveBeenCalledOnce();
    expect((screen.getByLabelText("Model ID") as HTMLInputElement).value).toBe(
      "manual-model"
    );

    fireEvent.pointerDown(chooseButton);
    fireEvent.click(screen.getByRole("menuitem", { name: /DeepSeek Reasoner/ }));
    expect((screen.getByLabelText("Model ID") as HTMLInputElement).value).toBe(
      "deepseek-reasoner"
    );
    expect((screen.getByLabelText("Model display name") as HTMLInputElement).value).toBe(
      "Manual model"
    );

    fireEvent.change(screen.getByLabelText("DeepSeek API key"), {
      target: { value: "different-secret" }
    });
    expect(screen.queryByText("Fetched 2 models.")).toBeNull();
    expect((chooseButton as HTMLButtonElement).disabled).toBe(true);
  });

  it("fills only the model ID from discovery and falls back to it when saving", async () => {
    const configureProvider = vi.fn(async () => undefined);
    const discoverDeepSeekModels = vi.fn(async () => [
      { id: "deepseek-reasoner", displayName: "DeepSeek Reasoner" }
    ]);
    useAppStore.setState({
      providers: [],
      models: [],
      configureProvider,
      discoverDeepSeekModels
    });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect DeepSeek" }));
    fireEvent.change(screen.getByLabelText("DeepSeek API key"), {
      target: { value: "discovery-secret" }
    });

    const chooseButton = screen.getByRole("button", {
      name: "Choose a fetched model for row 1"
    });
    fireEvent.click(screen.getByRole("button", { name: "Fetch models" }));
    await waitFor(() => expect((chooseButton as HTMLButtonElement).disabled).toBe(false));

    fireEvent.pointerDown(chooseButton);
    fireEvent.click(screen.getByRole("menuitem", { name: /DeepSeek Reasoner/ }));

    expect((screen.getByLabelText("Model display name") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Model ID") as HTMLInputElement).value).toBe(
      "deepseek-reasoner"
    );

    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() =>
      expect(configureProvider).toHaveBeenCalledWith({
        kind: "deepseek",
        apiKey: "discovery-secret",
        models: [{ id: "deepseek-reasoner", displayName: "deepseek-reasoner" }]
      })
    );
  });

  it("keeps manual models while discovery fails, retries, or returns no models", async () => {
    const discoverDeepSeekModels = vi
      .fn<() => Promise<never[]>>()
      .mockRejectedValueOnce(new Error("safe discovery failure"))
      .mockResolvedValueOnce([]);
    useAppStore.setState({ providers: [], models: [], discoverDeepSeekModels });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect DeepSeek" }));
    fireEvent.change(screen.getByLabelText("DeepSeek API key"), {
      target: { value: "retry-secret" }
    });
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "manual-model" }
    });

    fireEvent.click(screen.getByRole("button", { name: "Fetch models" }));
    expect(
      await screen.findByText("Could not fetch models. Check the API key and try again.")
    ).toBeTruthy();
    expect((screen.getByLabelText("Model ID") as HTMLInputElement).value).toBe(
      "manual-model"
    );

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(
      await screen.findByText("No models were returned. You can still add one manually.")
    ).toBeTruthy();
    expect(discoverDeepSeekModels).toHaveBeenCalledTimes(2);
  });

  it("submits the complete custom provider configuration and retains only its summary", async () => {
    const configureProvider = vi.fn(async (params: RuntimeProviderConfigureParams) => {
      if (params.kind !== "custom") throw new Error("expected custom configuration");
      useAppStore.setState({
        providers: [
          {
            id: params.providerId,
            displayName: params.displayName,
            origin: "custom",
            configured: true,
            credentialConfigured: Boolean(params.apiKey),
            health: "unknown"
          }
        ],
        models: params.models.map((model) => ({
          providerId: params.providerId,
          id: model.id,
          displayName: model.displayName,
          enabled: true
        }))
      });
    });
    useAppStore.setState({ providers: [], models: [], configureProvider });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect Custom provider" }));
    expect(screen.getByRole("dialog", { name: "Custom provider" })).toBeTruthy();
    expect(screen.getByText("Providers", { selector: "h1" })).toBeTruthy();
    const submitButton = screen.getByRole("button", { name: "Submit" }) as HTMLButtonElement;
    expect(submitButton.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText("Provider ID"), {
      target: { value: "mock-provider" }
    });
    fireEvent.change(screen.getByLabelText("Display name"), {
      target: { value: "   " }
    });
    expect(submitButton.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Display name"), {
      target: { value: "Mock Provider" }
    });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://mock.invalid/v1" }
    });
    expect(submitButton.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("API key"), {
      target: { value: "custom-secret" }
    });
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "mock-model-v1" }
    });
    fireEvent.change(screen.getByLabelText("Model display name"), {
      target: { value: "Mock Model V1" }
    });
    expect(submitButton.disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Add model" }));
    const modelIdInputs = screen.getAllByLabelText("Model ID");
    const modelNameInputs = screen.getAllByLabelText("Model display name");
    fireEvent.change(modelIdInputs[1], { target: { value: "mock-model-v1" } });
    fireEvent.change(modelNameInputs[1], { target: { value: "Duplicate model" } });
    expect(submitButton.disabled).toBe(true);
    fireEvent.change(modelIdInputs[1], { target: { value: "mock-model-v2" } });
    fireEvent.change(modelNameInputs[1], { target: { value: "Mock Model V2" } });
    expect(submitButton.disabled).toBe(false);
    fireEvent.change(screen.getByLabelText("Header name"), {
      target: { value: "X-Mock-Auth" }
    });
    fireEvent.change(screen.getByLabelText("Header value"), {
      target: { value: "header-secret" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() =>
      expect(configureProvider).toHaveBeenCalledWith({
        kind: "custom",
        providerId: "mock-provider",
        displayName: "Mock Provider",
        baseUrl: "https://mock.invalid/v1",
        apiKey: "custom-secret",
        headers: { "X-Mock-Auth": "header-secret" },
        models: [
          { id: "mock-model-v1", displayName: "Mock Model V1" },
          { id: "mock-model-v2", displayName: "Mock Model V2" }
        ]
      })
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByText("Mock Provider")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove Mock Provider" })).toBeTruthy();
    expect(screen.queryByDisplayValue("custom-secret")).toBeNull();
    expect(screen.queryByDisplayValue("header-secret")).toBeNull();
    expect(screen.queryByDisplayValue("https://mock.invalid/v1")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Models" }));
    fireEvent.click(screen.getByRole("button", { name: "Mock Provider" }));
    expect(screen.getByText("Mock Model V1")).toBeTruthy();
    expect(screen.getAllByText("mock-model-v1")).toHaveLength(1);
    expect(screen.getByText("Mock Model V2")).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Toggle Mock Model V1" })).toBeTruthy();
  });

  it("keeps the provider dialog open for a failed configuration retry", async () => {
    const configureProvider = vi.fn(async () => {
      throw new Error("safe configuration failure");
    });
    useAppStore.setState({ providers: [], models: [], configureProvider });

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    fireEvent.click(screen.getByRole("button", { name: "Connect DeepSeek" }));
    fireEvent.change(screen.getByLabelText("DeepSeek API key"), {
      target: { value: "retry-secret" }
    });
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "retry-model" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(await screen.findByText("Could not save provider configuration.")).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "Connect DeepSeek" })).toBeTruthy();
    expect((screen.getByLabelText("DeepSeek API key") as HTMLInputElement).value).toBe(
      "retry-secret"
    );
  });

  it("translates fixed provider UI while preserving product names", () => {
    setUiLanguage("zh-CN");
    render(<SettingsPage />);

    fireEvent.click(screen.getByRole("button", { name: "供应商" }));
    expect(screen.getByRole("heading", { level: 1, name: "供应商" })).toBeTruthy();
    expect(screen.getByText("DeepSeek")).toBeTruthy();
    expect(screen.getByText("自定义供应商")).toBeTruthy();
    expect(screen.getByRole("button", { name: "连接 DeepSeek" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "连接 自定义供应商" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "连接 DeepSeek" }));
    expect(screen.getByRole("button", { name: "获取模型" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "为第 1 行选择已获取的模型" })).toBeTruthy();
  });

  it("dismisses the custom provider dialog without redundant navigation controls", async () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    const trigger = screen.getByRole("button", { name: "Connect Custom provider" });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("true");

    const dialog = screen.getByRole("dialog", { name: "Custom provider" });
    expect(within(dialog).queryByRole("button", { name: "Back" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "Close" })).toBeNull();
    fireEvent.change(screen.getByLabelText("Provider ID"), {
      target: { value: "discarded-draft" }
    });
    fireEvent.click(screen.getByTestId("provider-dialog-overlay"));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(trigger);
    expect((screen.getByLabelText("Provider ID") as HTMLInputElement).value).toBe("");
    fireEvent.keyDown(screen.getByLabelText("Provider ID"), { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(trigger);
  });

  it("dismisses the DeepSeek dialog while preserving the providers page", async () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    const trigger = screen.getByRole("button", { name: "Connect DeepSeek" });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("true");

    const dialog = screen.getByRole("dialog", { name: "Connect DeepSeek" });
    expect(within(dialog).queryByRole("button", { name: "Back" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "Close" })).toBeNull();
    fireEvent.change(screen.getByLabelText("DeepSeek API key"), {
      target: { value: "discarded-secret" }
    });
    fireEvent.keyDown(screen.getByLabelText("DeepSeek API key"), { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(trigger);
    expect((screen.getByLabelText("DeepSeek API key") as HTMLInputElement).value).toBe("");
    fireEvent.click(screen.getByTestId("provider-dialog-overlay"));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(trigger);
  });

  it("preserves the Runtime provider catalog when SettingsPage remounts", () => {
    useAppStore.setState({
      providers: [
        {
          id: "deepseek",
          displayName: "DeepSeek",
          origin: "builtin",
          configured: true,
          credentialConfigured: true,
          health: "unknown"
        }
      ],
      models: [
        {
          providerId: "deepseek",
          id: "persisted-model",
          displayName: "Persisted Model",
          enabled: false
        }
      ]
    });
    const firstRender = render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    expect(screen.getByRole("button", { name: "Disconnect DeepSeek" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Models" }));
    expect(
      screen.getByRole("switch", { name: "Toggle Persisted Model" }).getAttribute("aria-checked")
    ).toBe("false");
    firstRender.unmount();

    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Providers" }));
    expect(screen.getByRole("button", { name: "Disconnect DeepSeek" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Models" }));
    expect(
      screen
        .getByRole("switch", { name: "Toggle Persisted Model" })
        .getAttribute("aria-checked")
    ).toBe("false");
  });

  it("loads Skills only when the Skills section opens and refreshes on re-entry", async () => {
    const loadSkillCatalog = vi.fn(async () => {
      useAppStore.setState({
        skillCatalogStatus: "ready",
        skillCatalogError: null,
        skills: [
          {
            name: "grill-me",
            description: "Resolve one design decision at a time.",
            location: "C:/Users/User/.ikaros/skills/grill-me/SKILL.md",
            enabled: true
          }
        ],
        skillDiagnostics: []
      });
    });
    const setSkillEnabled = vi.fn(async ({ name, enabled }: { name: string; enabled: boolean }) => {
      useAppStore.setState((state) => ({
        skillCatalogStatus: "ready",
        skills: state.skills.map((skill) =>
          skill.name === name ? { ...skill, enabled } : skill
        )
      }));
    });
    useAppStore.setState({
      skillCatalogStatus: "idle",
      skillCatalogError: null,
      skills: [],
      skillDiagnostics: [],
      loadSkillCatalog,
      setSkillEnabled
    });

    render(<SettingsPage />);
    expect(loadSkillCatalog).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Skills" }));
    await waitFor(() => expect(loadSkillCatalog).toHaveBeenCalledOnce());
    expect(screen.getByRole("heading", { level: 1, name: "Skills" })).toBeTruthy();
    expect(screen.getByText("Resolve one design decision at a time.")).toBeTruthy();

    fireEvent.click(screen.getByRole("switch", { name: "Toggle grill-me" }));
    await waitFor(() =>
      expect(setSkillEnabled).toHaveBeenCalledWith({ name: "grill-me", enabled: false })
    );

    fireEvent.click(screen.getByRole("button", { name: "General" }));
    fireEvent.click(screen.getByRole("button", { name: "Skills" }));
    await waitFor(() => expect(loadSkillCatalog).toHaveBeenCalledTimes(2));
  });

  it("finds Profile through settings search without adding fake sections", () => {
    render(<SettingsPage />);

    fireEvent.change(screen.getByRole("searchbox", { name: "Search settings..." }), {
      target: { value: "profile" }
    });

    expect(screen.getByRole("button", { name: "Profile" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "General" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Appearance" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Account" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Usage & billing" })).toBeNull();
  });

  it("uses the normalized settings typography scale", () => {
    render(<SettingsPage />);

    const generalHeading = screen.getByRole("heading", { level: 1, name: "General" });
    expect(generalHeading.className).toContain("text-[20px]");
    expect(generalHeading.className).toContain("leading-[28px]");
    expect(screen.getByText("Language for the app UI").className).toContain("text-[11px]");
    expect(screen.getByRole("button", { name: "Choose UI language" }).className).toContain(
      "leading-[18px]"
    );

    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    const appearanceHeading = screen.getByRole("heading", { level: 1, name: "Appearance" });
    expect(appearanceHeading.className).toContain("text-[20px]");
    expect(appearanceHeading.className).toContain("leading-[28px]");
    expect(screen.getByText("Reduce interface animations and transitions").className).toContain(
      "text-[11px]"
    );

    const codeKeyword = screen.getAllByText("const")[0];
    const codePane = codeKeyword.parentElement?.parentElement?.parentElement;
    expect(codePane?.className).toContain("text-[11px]");
    expect(codePane?.className).toContain("leading-[20px]");

    const renderedClassNames = Array.from(document.querySelectorAll("[class]"), (node) =>
      node.getAttribute("class") ?? ""
    ).join(" ");
    expect(renderedClassNames).not.toMatch(/(?:text|leading)-\[(?:\d+\.\d+|23)px\]/);
  });

  it("applies Simplified Chinese immediately and persists it", async () => {
    const { api, update } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });

    render(<SettingsPage />);

    fireEvent.pointerDown(screen.getByRole("button", { name: "Choose UI language" }), {
      button: 0,
      ctrlKey: false
    });
    const chineseOption = await screen.findByRole("menuitemradio", { name: "简体中文" });
    fireEvent.click(chineseOption);

    expect(await screen.findByRole("heading", { level: 1, name: "常规" })).toBeTruthy();
    expect(screen.getByText("应用 UI 语言")).toBeTruthy();
    expect(document.documentElement.lang).toBe("zh-CN");
    await waitFor(() => expect(update).toHaveBeenCalledWith({ language: "zh-CN" }));
  });

  it("rolls back the UI language when persistence fails", async () => {
    const { api } = desktopApiWithPreferences(async () => {
      throw new Error("disk unavailable");
    });
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });

    render(<SettingsPage />);
    fireEvent.pointerDown(screen.getByRole("button", { name: "Choose UI language" }), {
      button: 0,
      ctrlKey: false
    });
    fireEvent.click(await screen.findByRole("menuitemradio", { name: "简体中文" }));

    expect(await screen.findByText("Could not save settings.")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1, name: "General" })).toBeTruthy();
    expect(document.documentElement.lang).toBe("en");
  });

  it("persists theme selection and applies the light palette", async () => {
    const { api, update } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });
    render(<SettingsPage />);

    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
    fireEvent.click(screen.getByRole("radio", { name: "Light" }));

    await waitFor(() => expect(update).toHaveBeenCalledWith({ colorScheme: "light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.style.getPropertyValue("--canvas")).toBe("#f7f7f7");
  });

  it("starts the light-dark reveal from the clicked theme card", async () => {
    const { api } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });
    const startViewTransition = vi.fn((update: () => void) => {
      update();
      return {
        finished: new Promise(() => undefined),
        ready: Promise.resolve(),
        updateCallbackDone: Promise.resolve(),
        skipTransition: vi.fn()
      };
    });
    Object.defineProperty(document, "startViewTransition", {
      configurable: true,
      value: startViewTransition
    });
    render(<SettingsPage />);
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"));
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    fireEvent.click(screen.getByRole("radio", { name: "Light" }), {
      clientX: 220,
      clientY: 300,
      detail: 1
    });

    expect(startViewTransition).toHaveBeenCalledOnce();
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.dataset.themeTransition).toBe("radial");
    expect(document.documentElement.style.getPropertyValue("--theme-transition-x")).toBe(
      "220px"
    );
    expect(document.documentElement.style.getPropertyValue("--theme-transition-y")).toBe(
      "300px"
    );
  });

  it("previews and persists an edited theme color", async () => {
    const { api, update } = desktopApiWithPreferences();
    Object.defineProperty(window, "ikarosDesktop", { configurable: true, value: api });
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));

    const accent = screen.getByLabelText("Choose Accent color");
    fireEvent.change(accent, { target: { value: "#ff0000" } });
    expect(document.documentElement.style.getPropertyValue("--accent")).toBe("#ff0000");
    fireEvent.blur(accent);

    await waitFor(() =>
      expect(update).toHaveBeenCalledWith(
        expect.objectContaining({ darkTheme: expect.objectContaining({ accent: "#ff0000" }) })
      )
    );
  });

  it("does not claim a preference was saved when the preload API is missing", async () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
    fireEvent.click(screen.getByRole("radio", { name: "Light" }));

    expect(await screen.findByText("Could not save settings.")).toBeTruthy();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("returns to the current app state instead of opening a modal", () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "Back to app" }));
    expect(useAppStore.getState().settingsOpen).toBe(false);
  });
});
