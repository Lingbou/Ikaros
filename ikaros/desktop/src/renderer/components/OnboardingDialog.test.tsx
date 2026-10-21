import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../store";
import { OnboardingDialog } from "./OnboardingDialog";

const initialState = useAppStore.getState();

function installDesktopApi(
  overrides: Partial<{
    onboardingCompleted: boolean;
    username: string;
    language: "en" | "zh-CN";
    colorScheme: "system" | "light" | "dark";
  }> = {},
) {
  const update = vi.fn(async () => ({ onboardingCompleted: true }));
  vi.stubGlobal("ikarosDesktop", {
    preferences: {
      get: vi.fn(async () => ({
        onboardingCompleted: false,
        username: "User",
        language: "en",
        colorScheme: "dark",
        ...overrides,
      })),
      update,
      onChanged: vi.fn(() => () => undefined),
    },
  });
  return { update };
}

beforeEach(() => {
  useAppStore.setState({
    runtimeConnectionStatus: "connected",
    providerCatalogStatus: "ready",
    providers: [],
    models: [],
    selectedModel: null,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState, true);
  vi.unstubAllGlobals();
});

describe("first-run onboarding", () => {
  it("starts with the profile step", async () => {
    installDesktopApi();
    render(<OnboardingDialog />);

    expect(await screen.findByText("Welcome to Ikaros")).toBeVisible();
    expect(screen.getByText("Step 1 of 3")).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Nickname" })).toHaveValue("User");
    expect(screen.getByRole("button", { name: "中文" })).toBeVisible();
    expect(screen.getByRole("button", { name: "English" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
  });

  it("validates the nickname before continuing and saves it", async () => {
    const { update } = installDesktopApi();
    render(<OnboardingDialog />);
    const input = await screen.findByRole("textbox", { name: "Nickname" });

    fireEvent.change(input, { target: { value: "   " } });
    expect(screen.getByRole("alert")).toHaveTextContent("Use 1 to 32 characters.");
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();

    fireEvent.change(input, { target: { value: "  Nova  " } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(update).toHaveBeenCalledWith({ username: "Nova" });
    expect(screen.getByText("Step 2 of 3")).toBeVisible();
    expect(screen.getByText("Connect a model")).toBeVisible();
  });

  it("applies language and theme immediately", async () => {
    const { update } = installDesktopApi();
    render(<OnboardingDialog />);
    await screen.findByText("Welcome to Ikaros");

    fireEvent.click(screen.getByRole("button", { name: "中文" }));
    expect(update).toHaveBeenCalledWith({ language: "zh-CN" });
    fireEvent.click(screen.getByRole("button", { name: "Light" }));
    expect(update).toHaveBeenCalledWith({ colorScheme: "light" });
  });

  it("connects a provider and finishes from the summary", async () => {
    const { update } = installDesktopApi();
    const configureProvider = vi.fn(async () => {
      useAppStore.setState({
        providers: [
          {
            id: "deepseek",
            displayName: "DeepSeek",
            origin: "builtin",
            configured: true,
            credentialConfigured: true,
            health: "unknown",
          },
        ],
        models: [
          {
            providerId: "deepseek",
            id: "deepseek-chat",
            displayName: "deepseek-chat",
            enabled: true,
            contextWindow: 32768,
            reasoningEffort: null,
            reasoningEfforts: [],
          },
        ],
      });
    });
    useAppStore.setState({
      configureProvider,
      loadProviderCatalog: vi.fn(async () => undefined),
      discoverDeepSeekModels: vi.fn(async () => []),
    });

    render(<OnboardingDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "Continue" }));

    fireEvent.change(screen.getByLabelText("DeepSeek API key"), {
      target: { value: "sk-test" },
    });
    fireEvent.change(screen.getByLabelText("Model ID"), { target: { value: "deepseek-chat" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() => expect(configureProvider).toHaveBeenCalledOnce());
    expect(await screen.findByText("You are all set")).toBeVisible();
    expect(screen.getByText("Step 3 of 3")).toBeVisible();
    expect(screen.getByText("DeepSeek")).toBeVisible();
    expect(screen.getByText("Enabled models")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Start using Ikaros" }));
    expect(update).toHaveBeenCalledWith({ onboardingCompleted: true });
    await waitFor(() => expect(screen.queryByText("You are all set")).toBeNull());
  });

  it("records completion when skipped", async () => {
    const { update } = installDesktopApi();
    render(<OnboardingDialog />);

    fireEvent.click(await screen.findByRole("button", { name: "Set up later" }));
    expect(update).toHaveBeenCalledWith({ onboardingCompleted: true });
    await waitFor(() => expect(screen.queryByText("Welcome to Ikaros")).toBeNull());
  });

  it("stays hidden after onboarding completed", async () => {
    installDesktopApi({ onboardingCompleted: true });
    render(<OnboardingDialog />);

    await waitFor(() => expect(screen.queryByText("Welcome to Ikaros")).toBeNull());
  });

  it("stays hidden when a provider is already configured and records completion", async () => {
    const { update } = installDesktopApi();
    useAppStore.setState({
      providers: [
        {
          id: "custom",
          displayName: "Custom",
          origin: "custom",
          configured: true,
          credentialConfigured: true,
          health: "unknown",
        },
      ],
    });
    render(<OnboardingDialog />);

    await waitFor(() => expect(update).toHaveBeenCalledWith({ onboardingCompleted: true }));
    expect(screen.queryByText("Welcome to Ikaros")).toBeNull();
  });
});
