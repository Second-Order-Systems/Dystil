import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ReadyToUse } from "../../pages/ready-to-use";
import { commands } from "@/lib/utils/tauri";

vi.mock("@/lib/utils/tauri", () => ({ commands: {
  getReadyToUse: vi.fn(), getWorthFixingSummary: vi.fn(), getReadyArtifact: vi.fn(), getReadyArtifactProvenance: vi.fn(), recordReadyArtifactUsed: vi.fn(), openReadyCapability: vi.fn(), removeReadyArtifact: vi.fn(), proposeReadyArtifactChange: vi.fn(), retryReadyArtifactChange: vi.fn(), confirmReadyArtifactChange: vi.fn(), rejectReadyArtifactChange: vi.fn(),
} }));

const card = { artifactId: "a1", title: "Prepare the weekly report", kind: "prompt" as const, description: "Turn notes into the usual report.", lastUsedAt: null, primaryAction: "copy" as const, secondaryAction: "open" as const };
const detail = { card, body: "Use these notes to prepare the weekly report.", keptAt: "2026-01-01T00:00:00Z", changeCount: 0, changes: [], provenanceAvailable: true, provenanceLabel: "Report workflow" };

describe("Ready to use", () => {
  beforeEach(() => { vi.clearAllMocks(); vi.mocked(commands.getWorthFixingSummary).mockResolvedValue({ status: "ok", data: { selected: [], eligibleCount: 0, watchingCount: 0, pendingObservationCount: 0, manualRefreshReady: false, processing: false, staleEvidenceCount: 0, providerReady: true, lastSuccessfulWakeAt: null } }); vi.mocked(commands.recordReadyArtifactUsed).mockResolvedValue({ status: "ok", data: { artifactId: "a1", lastUsedAt: "2026-01-02T00:00:00Z" } }); });

  it("renders a truthful empty state linked to Worth fixing", async () => {
    const onWorthFixing = vi.fn();
    vi.mocked(commands.getReadyToUse).mockResolvedValue({ status: "ok", data: { items: [], nextCursor: null } });
    render(<ReadyToUse onAsk={vi.fn()} onWorthFixing={onWorthFixing} />);
    fireEvent.click(await screen.findByRole("button", { name: "See Worth fixing" }));
    expect(onWorthFixing).toHaveBeenCalledOnce();
    expect(screen.getByRole("heading", { name: "Keep a finding once. Use it again from here." })).toBeInTheDocument();
    expect(screen.getByText("Runbook")).toBeInTheDocument();
  });

  it("opens detail inline with provenance and an explicit change preview", async () => {
    vi.mocked(commands.getReadyToUse).mockResolvedValue({ status: "ok", data: { items: [card], nextCursor: null } });
    vi.mocked(commands.getReadyArtifact).mockResolvedValue({ status: "ok", data: detail });
    vi.mocked(commands.getReadyArtifactProvenance).mockResolvedValue({ status: "ok", data: [{ evidenceId: "e1", occurredAt: "2026-01-01T00:00:00Z", app: "Editor", description: "Prepared a report.", available: true }] });
    vi.mocked(commands.proposeReadyArtifactChange).mockResolvedValue({ status: "ok", data: { changeJobId: "c1", artifactId: "a1", title: card.title, body: "A shorter report prompt.", changedLineCount: 1 } });
    render(<ReadyToUse onAsk={vi.fn()} onWorthFixing={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Open it" }));
    expect(await screen.findByText(detail.body)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Where this came from" }));
    expect(await screen.findByText("Prepared a report.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Ask Dystil to change it" }));
    fireEvent.change(screen.getByLabelText("What should it do differently?"), { target: { value: "Make it shorter" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview change" }));
    expect(await screen.findByRole("heading", { name: "How it would read" })).toBeInTheDocument();
    expect(screen.getByText("A shorter report prompt.")).toBeInTheDocument();
  });

  it("records copy use only after the clipboard succeeds", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    vi.mocked(commands.getReadyToUse).mockResolvedValue({ status: "ok", data: { items: [card], nextCursor: null } });
    vi.mocked(commands.getReadyArtifact).mockResolvedValue({ status: "ok", data: detail });
    render(<ReadyToUse onAsk={vi.fn()} onWorthFixing={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Copy it" }));
    expect(await screen.findByText(/Copied “Prepare the weekly report”/)).toBeInTheDocument();
    expect(writeText).toHaveBeenCalledWith(detail.body);
    expect(commands.recordReadyArtifactUsed).toHaveBeenCalledWith("a1", "copy");
  });

  it("does not record use when copying fails", async () => {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockRejectedValue(new Error("Clipboard unavailable")) } });
    vi.mocked(commands.getReadyToUse).mockResolvedValue({ status: "ok", data: { items: [card], nextCursor: null } });
    vi.mocked(commands.getReadyArtifact).mockResolvedValue({ status: "ok", data: detail });
    render(<ReadyToUse onAsk={vi.fn()} onWorthFixing={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Copy it" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Clipboard unavailable");
    expect(commands.recordReadyArtifactUsed).not.toHaveBeenCalled();
  });
});
