import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../store";
import { OnboardingDialog } from "./OnboardingDialog";

const initialState = useAppStore.getState();

function installDesktopApi(onboardingCompleted: boolean) {
  const update = vi.fn(async () => ({ onboardingCompleted: true }));
  vi.stubGlobal("ikarosDesktop", {
    preferences: {
      get: vi.fn(async () => ({ onboardingCompleted })),
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
  it("shows setup when no provider is configured", async () => {
    installDesktopApi(false);
    render(<OnboardingDialog />);

    expect(await screen.findByText("Welcome to Ikaros")).toBeVisible();
    expect(screen.getByRole("tab", { name: "DeepSeek" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("button", { name: "Set up later" })).toBeVisible();
  });

  it("stays hidden after onboarding completed", async () => {
    installDesktopApi(true);
    render(<OnboardingDialog />);

    await waitFor(() => expect(screen.queryByText("Welcome to Ikaros")).toBeNull());
  });

  it("stays hidden when a provider is already configured and records completion", async () => {
    const { update } = installDesktopApi(false);
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

  it("records completion when the user skips", async () => {
    const { update } = installDesktopApi(false);
    render(<OnboardingDialog />);

    fireEvent.click(await screen.findByRole("button", { name: "Set up later" }));
    expect(update).toHaveBeenCalledWith({ onboardingCompleted: true });
    await waitFor(() => expect(screen.queryByText("Welcome to Ikaros")).toBeNull());
  });
});
