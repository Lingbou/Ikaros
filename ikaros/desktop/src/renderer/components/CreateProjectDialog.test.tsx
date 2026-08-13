import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { RuntimeWorkspaceSummary } from "../../shared/runtime";
import { setUiLanguage } from "../i18n";
import { useAppStore } from "../store";
import { CreateProjectDialog } from "./CreateProjectDialog";

const initialState = useAppStore.getState();
const workspace: RuntimeWorkspaceSummary = {
  id: "workspace-123",
  name: "Ikaros",
  rootUri: "C:\\Workspace\\github\\Ikaros"
};

function installWorkspacePicker(
  implementation: () => Promise<RuntimeWorkspaceSummary | null> = async () => workspace
) {
  const chooseDirectory = vi.fn(implementation);
  Object.defineProperty(window, "ikarosDesktop", {
    configurable: true,
    value: {
      workspace: { chooseDirectory }
    }
  });
  return chooseDirectory;
}

beforeEach(() => {
  setUiLanguage("en");
});

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(window, "ikarosDesktop");
  useAppStore.setState(initialState, true);
  vi.restoreAllMocks();
});

describe("CreateProjectDialog", () => {
  it("stages the chosen workspace with a normalized project name", async () => {
    const chooseDirectory = installWorkspacePicker();
    const stageProjectWorkspace = vi.fn();
    useAppStore.setState({ stageProjectWorkspace });
    const onOpenChange = vi.fn();

    render(<CreateProjectDialog open onOpenChange={onOpenChange} />);

    expect(screen.getByRole("dialog", { name: "Create project" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));

    await waitFor(() => expect(chooseDirectory).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "Project name" })).toHaveValue("Ikaros");
    expect(screen.getByText("C:\\Workspace\\github\\Ikaros")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("textbox", { name: "Project name" }), {
      target: { value: "  Named project  " }
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(stageProjectWorkspace).toHaveBeenCalledWith({
      id: workspace.id,
      name: "Named project",
      rootUri: workspace.rootUri
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("keeps a manually edited name when the folder changes", async () => {
    installWorkspacePicker();
    render(<CreateProjectDialog open onOpenChange={() => undefined} />);

    fireEvent.change(screen.getByRole("textbox", { name: "Project name" }), {
      target: { value: "Personal name" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Change folder" })).toBeInTheDocument()
    );
    expect(screen.getByRole("textbox", { name: "Project name" })).toHaveValue("Personal name");
  });

  it("keeps the dialog open when the native picker is canceled", async () => {
    const chooseDirectory = installWorkspacePicker(async () => null);
    const onOpenChange = vi.fn();
    render(<CreateProjectDialog open onOpenChange={onOpenChange} />);

    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));

    await waitFor(() => expect(chooseDirectory).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("dialog", { name: "Create project" })).toBeInTheDocument();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows a fixed localized picker error without leaking the original failure", async () => {
    installWorkspacePicker(async () => {
      throw new Error("C:\\private\\secret: native picker exploded");
    });
    render(<CreateProjectDialog open onOpenChange={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Could not choose the folder.");
    expect(screen.queryByText(/private|native picker exploded/i)).toBeNull();
    expect(screen.getByRole("dialog", { name: "Create project" })).toBeInTheDocument();
  });

  it("disables controls while choosing and ignores a late picker result after cancel", async () => {
    let resolvePicker: ((value: RuntimeWorkspaceSummary | null) => void) | undefined;
    installWorkspacePicker(
      () =>
        new Promise((resolve) => {
          resolvePicker = resolve;
        })
    );
    const stageProjectWorkspace = vi.fn();
    useAppStore.setState({ stageProjectWorkspace });
    const onOpenChange = vi.fn();
    const { rerender } = render(<CreateProjectDialog open onOpenChange={onOpenChange} />);

    const addFolder = screen.getByRole("button", { name: "Add folder" });
    fireEvent.click(addFolder);
    expect(addFolder).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "Project name" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    rerender(<CreateProjectDialog open={false} onOpenChange={onOpenChange} />);
    resolvePicker?.(workspace);

    await Promise.resolve();
    expect(stageProjectWorkspace).not.toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("resets drafts on close and validates a trimmed 1 to 200 character name", async () => {
    installWorkspacePicker();
    const onOpenChange = vi.fn();
    const { rerender } = render(<CreateProjectDialog open onOpenChange={onOpenChange} />);

    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Change folder" })).toBeInTheDocument()
    );
    const name = screen.getByRole("textbox", { name: "Project name" });
    expect(name).toHaveAttribute("maxlength", "200");
    fireEvent.change(name, { target: { value: "   " } });
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();
    fireEvent.change(name, { target: { value: "x".repeat(200) } });
    expect(screen.getByRole("button", { name: "Create" })).toBeEnabled();
    fireEvent.change(name, { target: { value: "x".repeat(201) } });
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    rerender(<CreateProjectDialog open={false} onOpenChange={onOpenChange} />);
    rerender(<CreateProjectDialog open onOpenChange={onOpenChange} />);

    expect(screen.getByRole("textbox", { name: "Project name" })).toHaveValue("");
    expect(screen.getByRole("button", { name: "Add folder" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();
  });

  it.each([
    [
      "overlay",
      () =>
        fireEvent.pointerDown(screen.getByTestId("create-project-overlay"), {
          button: 0,
          pointerType: "mouse"
        })
    ],
    ["Escape", () => fireEvent.keyDown(document, { key: "Escape" })],
    ["close button", () => fireEvent.click(screen.getByRole("button", { name: "Close" }))],
    ["Cancel", () => fireEvent.click(screen.getByRole("button", { name: "Cancel" }))]
  ])("resets without staging when dismissed by %s", async (_method, dismiss) => {
    installWorkspacePicker();
    const stageProjectWorkspace = vi.fn();
    useAppStore.setState({ stageProjectWorkspace });
    const onOpenChange = vi.fn();
    const { rerender } = render(<CreateProjectDialog open onOpenChange={onOpenChange} />);

    fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Change folder" })).toBeInTheDocument()
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Project name" }), {
      target: { value: "Discard me" }
    });
    dismiss();

    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(stageProjectWorkspace).not.toHaveBeenCalled();
    rerender(<CreateProjectDialog open={false} onOpenChange={onOpenChange} />);
    rerender(<CreateProjectDialog open onOpenChange={onOpenChange} />);
    expect(screen.getByRole("textbox", { name: "Project name" })).toHaveValue("");
    expect(screen.getByRole("button", { name: "Add folder" })).toBeInTheDocument();
  });
});
