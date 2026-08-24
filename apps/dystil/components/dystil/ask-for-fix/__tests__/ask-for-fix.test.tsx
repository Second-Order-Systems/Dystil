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
  askForFixStartWatching: vi.fn(),
  askForFixStopWatching: vi.fn(),
  askForFixReviewWatch: vi.fn(),
  askForFixUpdateWatchGuidance: vi.fn(),
}));
const mockRouter = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));

vi.mock("@/lib/utils/tauri", () => ({ commands: mockCommands }));
vi.mock("next/navigation", () => ({ useRouter: () => mockRouter }));

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
    watch: null,
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

  it("moves a new conversation onto its own chat route", async () => {
    const submitted = session({
      messages: [{ messageId: "u1", role: "user", text: "I rebuild a report every Friday.", event: null, createdAt: "now" }],
    });
    mockCommands.askForFixSubmit.mockResolvedValue({ status: "ok", data: submitted });

    render(<AskForFix fresh />);
    const composer = await screen.findByPlaceholderText(/Describe the problem/);
    fireEvent.change(composer, { target: { value: "I rebuild a report every Friday." } });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));

    await waitFor(() => expect(mockRouter.replace).toHaveBeenCalledWith("/home/chat?session=afs_test"));
  });

  it("shows a concise confirmation and locks it only on confirmation", async () => {
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
    expect(screen.getByText("Dystil is ready to solve this workflow.")).toBeInTheDocument();
    expect(screen.getByText(/reusable solution you can keep and use again/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Solve this" }));
    await waitFor(() => expect(mockCommands.askForFixConfirm).toHaveBeenCalledWith("afs_test", null));
  });

  it("supports bounded multi-select and compare renderers with a free-text escape", async () => {
    const multi = session({
      phase: "follow_up",
      questionCount: 2,
      currentQuestionId: "afq_slow_parts",
      currentQuestion: {
        kind: "multi_select",
        text: "Which parts regularly slow this down?",
        helper: "Choose every part that is genuinely involved.",
        options: [
          { id: "finding", label: "Finding current inputs", description: "The right source is hard to locate." },
          { id: "copying", label: "Copying between tools", description: "The same values are entered twice." },
          { id: "approval", label: "Waiting for approval", description: "Another person must respond." },
        ],
        minSelections: 1,
        maxSelections: 2,
      },
      messages: [{ messageId: "a1", role: "assistant", text: "Which parts regularly slow this down?", event: null, createdAt: "now" }],
    });
    const compare = session({
      phase: "follow_up",
      questionCount: 3,
      currentQuestionId: "afq_reading",
      currentQuestion: {
        kind: "compare",
        text: "Which reading is closer?",
        helper: "Neither is required.",
        options: [
          { id: "steps", label: "The steps are the problem", description: "Repeating a known sequence costs the time." },
          { id: "context", label: "Rebuilding context is the problem", description: "Finding the current information costs the time." },
        ],
        minSelections: 1,
        maxSelections: 1,
      },
      messages: [{ messageId: "a2", role: "assistant", text: "Which reading is closer?", event: null, createdAt: "now" }],
    });
    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: multi });
    mockCommands.askForFixSubmit.mockResolvedValue({ status: "ok", data: compare });

    render(<AskForFix />);
    fireEvent.click(await screen.findByRole("button", { name: /Finding current inputs/ }));
    fireEvent.click(screen.getByRole("button", { name: /Copying between tools/ }));
    fireEvent.click(screen.getByRole("button", { name: "Use 2 answers" }));
    await waitFor(() => expect(mockCommands.askForFixSubmit).toHaveBeenCalledWith("afs_test", {
      text: "Finding current inputs — The right source is hard to locate.; Copying between tools — The same values are entered twice.",
      event: { kind: "multi_select", questionId: "afq_slow_parts", selectedOptionIds: ["finding", "copying"] },
    }));

    expect(await screen.findByText("The steps are the problem")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Answer in my own words" }));
    expect(screen.getByPlaceholderText("Answer in your own words…")).toBeInTheDocument();
  });

  it("renders each application-owned final artifact kind", async () => {
    const answered = (artifact: NonNullable<NonNullable<AskSessionView["presentation"]>["artifact"]>) => session({
      phase: "present",
      status: "answered",
      locked: true,
      presentation: {
        route: "answer_now",
        headline: "A useful answer",
        explanation: "This is based on the confirmed understanding.",
        limitations: ["Based on your answers only."],
        artifact,
      },
    });

    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: answered({
      kind: "prompt",
      title: "Reusable brief",
      description: "Instructions to reuse.",
      body: "Prepare the current inputs without making the final decision.",
      steps: [], tool: "", capability: "", instructions: [],
    }) });
    const promptView = render(<AskForFix />);
    expect(await screen.findByText("Reusable brief")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy prompt" })).toBeInTheDocument();
    promptView.unmount();

    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: answered({
      kind: "runbook",
      title: "Weekly preparation",
      description: "A bounded sequence.",
      body: "", steps: ["Collect the current inputs.", "Flag missing values."],
      tool: "", capability: "", instructions: [],
    }) });
    const runbookView = render(<AskForFix />);
    expect(await screen.findByText("Weekly preparation")).toBeInTheDocument();
    expect(screen.getByText("Flag missing values.")).toBeInTheDocument();
    runbookView.unmount();

    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: answered({
      kind: "existing_capability",
      title: "Use the import rule",
      description: "A capability already available.",
      body: "", steps: [], tool: "Spreadsheet", capability: "Scheduled import",
      instructions: ["Point the sheet at the existing export."],
    }) });
    render(<AskForFix />);
    expect(await screen.findByText("Scheduled import")).toBeInTheDocument();
    expect(screen.getByText("Point the sheet at the existing export.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy instructions" })).toBeInTheDocument();
  });

  it("revises a confirmed artifact in the same locked conversation", async () => {
    const answered = session({
      phase: "present",
      status: "answered",
      locked: true,
      messages: [{ messageId: "a1", role: "assistant", text: "Here is the first answer.", event: null, createdAt: "now" }],
      presentation: {
        route: "answer_now",
        headline: "Prepare the report context before review",
        explanation: "Use a short runbook while keeping the final call with you.",
        limitations: ["Based on your answers only."],
        artifact: {
          kind: "runbook",
          title: "Weekly preparation",
          description: "A bounded sequence.",
          body: "",
          steps: ["Collect the current inputs.", "Flag missing values."],
          tool: "",
          capability: "",
          instructions: [],
        },
      },
    });
    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: answered });
    mockCommands.askForFixSubmit.mockResolvedValue({
      status: "ok",
      data: {
        ...answered,
        presentation: {
          ...answered.presentation!,
          headline: "Prepare the report context and block duplicates before review",
        },
      },
    });

    render(<AskForFix />);
    fireEvent.click(await screen.findByRole("button", { name: "Ask Dystil to change it" }));
    const composer = screen.getByPlaceholderText("What should Dystil change in this answer?");
    fireEvent.change(composer, { target: { value: "Add an explicit duplicate check." } });
    fireEvent.click(screen.getByRole("button", { name: "Send answer" }));

    await waitFor(() => expect(mockCommands.askForFixSubmit).toHaveBeenCalledWith("afs_test", {
      text: "Add an explicit duplicate check.",
      event: { kind: "revise", questionId: null, selectedOptionIds: [] },
    }));
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

  it("offers keep watching for insufficient evidence and starts the persistent watch", async () => {
    const insufficient = session({
      phase: "present",
      status: "answered",
      presentation: {
        route: "cannot_see",
        headline: "There is not enough evidence yet",
        explanation: "The observed work does not form a credible end-to-end example.",
        limitations: ["The current matches may be unrelated."],
        artifact: null,
      },
    });
    const watching = session({
      ...insufficient,
      watch: {
        watchId: "afw_test",
        state: "active",
        spec: {
          goal: "Find a credible example",
          relevantSignals: [],
          missingEvidence: ["a complete instance"],
          sufficiencyRule: "One credible example",
        },
        supportingEvidenceCount: 0,
        weekCheckpointDue: false,
        createdAt: "2026-08-04T00:00:00Z",
        updatedAt: "2026-08-04T00:00:00Z",
      },
    });
    mockCommands.askForFixLatest.mockResolvedValue({ status: "ok", data: insufficient });
    mockCommands.askForFixStartWatching.mockResolvedValue({ status: "ok", data: watching });

    render(<AskForFix />);
    fireEvent.click(await screen.findByRole("button", { name: "Keep watching" }));

    await waitFor(() => expect(mockCommands.askForFixStartWatching).toHaveBeenCalledWith("afs_test"));
    expect(await screen.findByText("Dystil is watching for this work")).toBeInTheDocument();
  });
});
