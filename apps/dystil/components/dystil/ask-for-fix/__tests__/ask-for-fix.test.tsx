import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AskSessionView } from "@/lib/utils/tauri";
import { AskForFix } from "@/components/dystil/pages/ask-for-fix";

const mockCommands = vi.hoisted(() => ({
  askForFixLatest: vi.fn(),
  askForFixCreate: vi.fn(),
  askForFixSubmit: vi.fn(),
  askForFixConfirm: vi.fn(),
  askForFixRetry: vi.fn(),
  askForFixCancel: vi.fn(),
  askForFixKeepArtifact: vi.fn(),
}));

vi.mock("@/lib/utils/tauri", () => ({ commands: mockCommands }));

function session(overrides: Partial<AskSessionView> = {}): AskSessionView {
  return {
    sessionId: "afs_test",
    phase: "understand",
    status: "active",
    questionCount: 0,
    maxQuestions: 5,
    messages: [],
    understanding: {
      synthesis: "",
      grounding: [],
      inferences: [],
      preservedBoundary: "",
      uncertainty: [],
      solutionTarget: "",
    },
    currentQuestionId: null,
    currentQuestion: null,
    presentation: null,
    locked: false,
    lastErrorCode: null,
    lastErrorDetail: null,
    provider: null,
    model: null,
    cachedInputTokens: 0,
    artifactKeptId: null,
    createdAt: "2026-08-04T00:00:00Z",
    updatedAt: "2026-08-04T00:00:00Z",
    ...overrides,
  };
}

describe("AskForFix", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: null });
    mockCommands.askForFixCreate.mockResolvedValue({ status: "ok", data: session() });
  });

  it("turns choice-card selections into a semantic message plus a raw event", async () => {
    const followUp = session({
      phase: "follow_up",
      questionCount: 1,
      currentQuestionId: "afq_where",
      currentQuestion: {
        kind: "single_select",
        text: "Where does the work happen?",
        helper: "Choose the closest answer, or use your own words.",
        options: [
          { id: "one_app", label: "In one app", description: "It starts and ends in the same tool." },
          { id: "between_apps", label: "Between apps", description: "Information moves between tools." },
        ],
        minSelections: 1,
        maxSelections: 1,
      },
      messages: [
        { messageId: "u1", role: "user", text: "I rebuild a report every Friday.", event: null, createdAt: "now" },
        { messageId: "a1", role: "assistant", text: "Where does the work happen?", event: null, createdAt: "now" },
      ],
    });
    mockCommands.askForFixSubmit
      .mockResolvedValueOnce({ status: "ok", data: followUp })
      .mockResolvedValueOnce({ status: "ok", data: { ...followUp, currentQuestion: null } });

    render(<AskForFix />);
    const composer = await screen.findByPlaceholderText(/Describe the problem/);
    fireEvent.change(composer, { target: { value: "I rebuild a report every Friday." } });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));

    expect(await screen.findByText("Between apps")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Between apps/ }));
    fireEvent.click(screen.getByRole("button", { name: "Use this answer" }));

    await waitFor(() => expect(mockCommands.askForFixSubmit).toHaveBeenCalledTimes(2));
    expect(mockCommands.askForFixSubmit).toHaveBeenLastCalledWith("afs_test", {
      text: "Between apps — Information moves between tools.",
      event: { kind: "single_select", questionId: "afq_where", selectedOptionIds: ["between_apps"] },
    });
  });

  it("renders Dystil's causal understanding and locks it only on confirmation", async () => {
    const consolidation = session({
      phase: "consolidate",
      messages: [{ messageId: "a1", role: "assistant", text: "I have a working model.", event: null, createdAt: "now" }],
      understanding: {
        synthesis: "The report is not the repeated work; reconstructing its context is.",
        grounding: ["A report is rebuilt every Friday", "Inputs live in several files"],
        inferences: ["The reusable starting context is missing"],
        preservedBoundary: "The user's final judgement",
        uncertainty: ["Whether source layouts stay stable"],
        solutionTarget: "A prepared current starting point",
      },
    });
    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: consolidation });
    mockCommands.askForFixConfirm.mockResolvedValue({ status: "ok", data: { ...consolidation, phase: "present", status: "working", locked: true } });

    render(<AskForFix />);
    expect(await screen.findByText(/reconstructing its context/i)).toBeInTheDocument();
    expect(screen.getByText("The user's final judgement")).toBeInTheDocument();
    expect(screen.getByText("Whether source layouts stay stable")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Solve this" }));
    await waitFor(() => expect(mockCommands.askForFixConfirm).toHaveBeenCalledWith("afs_test"));
  });

  it("surfaces a durable provider error with a retry action", async () => {
    mockCommands.askForFixLatest.mockResolvedValue({
      status: "ok",
      data: session({ lastErrorCode: "provider_not_ready", lastErrorDetail: "No model is active." }),
    });
    mockCommands.askForFixRetry.mockResolvedValue({ status: "ok", data: session() });

    render(<AskForFix />);
    expect(await screen.findByText(/Connect an AI model in Settings/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(mockCommands.askForFixRetry).toHaveBeenCalledWith("afs_test"));
  });
});
