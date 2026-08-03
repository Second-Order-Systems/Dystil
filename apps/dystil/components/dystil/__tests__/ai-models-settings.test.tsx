import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AiModelsSettings } from "../ai-models-settings";

Object.defineProperties(HTMLElement.prototype, {
  hasPointerCapture: { value: () => false },
  setPointerCapture: { value: () => {} },
  releasePointerCapture: { value: () => {} },
  scrollIntoView: { value: () => {} },
});

const mocks = vi.hoisted(() => ({
  aiPresetList: vi.fn(),
  aiProviderStatus: vi.fn(),
  aiPresetDiscoverModels: vi.fn(),
  aiPresetActivateManaged: vi.fn(),
  toast: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@/components/ui/use-toast", () => ({ toast: (...args: unknown[]) => mocks.toast(...args) }));
vi.mock("@/lib/utils/tauri", () => ({
  commands: {
    aiPresetList: (...args: unknown[]) => mocks.aiPresetList(...args),
    aiProviderStatus: (...args: unknown[]) => mocks.aiProviderStatus(...args),
    aiPresetDiscoverModels: (...args: unknown[]) => mocks.aiPresetDiscoverModels(...args),
    aiPresetActivateManaged: (...args: unknown[]) => mocks.aiPresetActivateManaged(...args),
  },
}));

const managedPreset = {
  id: "managed-codex",
  name: "ChatGPT subscription",
  providerKind: "codex",
  endpoint: null,
  model: "default",
  active: true,
  credentialPresent: true,
  validationStatus: "ready",
  validationMessage: null,
  validatedAt: null,
};

describe("AI models settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.aiPresetList.mockResolvedValue({ status: "ok", data: [managedPreset] });
    mocks.aiProviderStatus.mockImplementation(async (provider: string) => ({ status: "ok", data: { provider, state: "ready", authenticated: true, installedVersion: "1", detail: null } }));
    mocks.aiPresetDiscoverModels.mockResolvedValue({ status: "ok", data: { models: ["qwen3:8b"], detail: "Found 1 model." } });
    mocks.aiPresetActivateManaged.mockResolvedValue({ status: "ok", data: managedPreset });
  });

  it("shows the actual active preset and detected Ollama models without invented spend", async () => {
    render(<AiModelsSettings />);

    expect(await screen.findByText("ChatGPT subscription")).toBeInTheDocument();
    expect(await screen.findByRole("combobox", { name: "Ollama model" })).toHaveTextContent("qwen3:8b");
    expect(screen.queryByText("$1.84")).not.toBeInTheDocument();
    expect(screen.queryByText("Cap what it spends")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Usage" })).not.toBeInTheDocument();
  });

  it("activates a connected subscription from the real preset command", async () => {
    mocks.aiPresetList.mockResolvedValue({ status: "ok", data: [] });
    render(<AiModelsSettings />);

    fireEvent.click(await screen.findByRole("button", { name: "Use ChatGPT Plus or Pro" }));
    await waitFor(() => expect(mocks.aiPresetActivateManaged).toHaveBeenCalledWith("codex", "default"));
  });

  it("offers the four API providers and only asks compatible APIs for a model", async () => {
    render(<AiModelsSettings />);

    fireEvent.click(await screen.findByRole("button", { name: "Add API preset" }));
    const provider = screen.getByRole("combobox", { name: "API provider" });
    fireEvent.keyDown(provider, { key: "ArrowDown" });

    expect(await screen.findByRole("option", { name: "Anthropic" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "OpenAI" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "OpenAI-compatible" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Dystil AI" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("option", { name: "OpenAI-compatible" }));
    expect(screen.getByPlaceholderText("model ID")).toBeInTheDocument();
  });
});
