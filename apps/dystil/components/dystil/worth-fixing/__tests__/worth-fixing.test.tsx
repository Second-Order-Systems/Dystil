import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WorthFixing } from "../../pages/worth-fixing";
import { commands } from "@/lib/utils/tauri";

vi.mock("@/lib/utils/tauri", () => ({ commands: {
  getWorthFixingSummary: vi.fn(), getWorthFixingEvidence: vi.fn(), keepWorthFixingFinding: vi.fn(), dismissWorthFixingFinding: vi.fn(), correctWorthFixingFinding: vi.fn(), refreshWorthFixing: vi.fn(), getOtherWorthFixingFindings: vi.fn(),
} }));

const summary = (overrides = {}) => ({ selected: [], eligibleCount: 0, watchingCount: 0, pendingObservationCount: 0, manualRefreshReady: false, processing: false, staleEvidenceCount: 0, providerReady: true, lastSuccessfulWakeAt: null, ...overrides });
const finding = { findingId: "finding-1", label: "There is a faster way", claim: "You rebuild the same report instructions.", whyWorthFixing: "A prepared prompt avoids repeated setup.", handoffType: "prompt" as const, handoffTitle: "Prepare the report", handoffPreview: "Use the supplied notes to create the weekly report.", occurrenceCount: 2, cadence: "none" as const, evidenceAvailable: true };

describe("Worth fixing", () => {
  beforeEach(() => { vi.clearAllMocks(); vi.mocked(commands.getOtherWorthFixingFindings).mockResolvedValue({ status: "ok", data: { items: [], nextCursor: null } }); });

  it("renders the first-open explanation without fabricated findings", async () => {
    vi.mocked(commands.getWorthFixingSummary).mockResolvedValue({ status: "ok", data: summary() });
    render(<WorthFixing onAsk={vi.fn()} onReady={vi.fn()} onSetup={vi.fn()} />);
    expect(await screen.findByRole("heading", { name: "Dystil has started reading how you work." })).toBeInTheDocument();
    expect(screen.queryByText(finding.claim)).not.toBeInTheDocument();
  });

  it("offers a manual check only when enough work has accumulated", async () => {
    vi.mocked(commands.getWorthFixingSummary).mockResolvedValue({ status: "ok", data: summary({ pendingObservationCount: 1 }) });
    const view = render(<WorthFixing onAsk={vi.fn()} onReady={vi.fn()} onSetup={vi.fn()} />);
    expect(await screen.findByRole("heading", { name: "Dystil has started reading how you work." })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check recent work" })).not.toBeInTheDocument();

    vi.mocked(commands.getWorthFixingSummary).mockResolvedValue({ status: "ok", data: summary({ pendingObservationCount: 2, manualRefreshReady: true }) });
    view.unmount();
    render(<WorthFixing onAsk={vi.fn()} onReady={vi.fn()} onSetup={vi.fn()} />);
    expect(await screen.findByRole("heading", { name: "Dystil has started reading how you work." })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Check recent work" })).toBeInTheDocument();
  });

  it("shows provider setup while preserving a real finding", async () => {
    vi.mocked(commands.getWorthFixingSummary).mockResolvedValue({ status: "ok", data: summary({ selected: [finding], eligibleCount: 1, providerReady: false }) });
    render(<WorthFixing onAsk={vi.fn()} onReady={vi.fn()} onSetup={vi.fn()} />);
    expect(await screen.findByText(finding.claim)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open model settings" })).toBeInTheDocument();
  });

  it("loads evidence lazily and suppresses a duplicate keep", async () => {
    vi.mocked(commands.getWorthFixingSummary).mockResolvedValue({ status: "ok", data: summary({ selected: [finding], eligibleCount: 1 }) });
    vi.mocked(commands.getWorthFixingEvidence).mockResolvedValue({ status: "ok", data: [{ evidenceId: "e1", occurredAt: "2026-01-01T00:00:00Z", app: "Editor", description: "Opened the same report template.", available: true }] });
    let resolveKeep!: (value: Awaited<ReturnType<typeof commands.keepWorthFixingFinding>>) => void;
    vi.mocked(commands.keepWorthFixingFinding).mockReturnValue(new Promise((resolve) => { resolveKeep = resolve; }));
    render(<WorthFixing onAsk={vi.fn()} onReady={vi.fn()} onSetup={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Show me what you saw" }));
    expect(screen.getByRole("button", { name: "Hide what Dystil saw" })).toHaveAttribute("aria-expanded", "true");
    expect(await screen.findByText("Opened the same report template.")).toBeInTheDocument();
    const keep = screen.getByRole("button", { name: "Keep this" });
    fireEvent.click(keep); fireEvent.click(keep);
    expect(commands.keepWorthFixingFinding).toHaveBeenCalledTimes(1);
    resolveKeep({ status: "ok", data: { artifact: { artifactId: "a1", title: "Prepare", kind: "prompt", description: "Body", lastUsedAt: null, primaryAction: "copy", secondaryAction: "open" }, summary: summary(), alreadyKept: false } });
    await waitFor(() => expect(screen.getByText(/Kept “Prepare”/)).toBeInTheDocument());
    expect(screen.getByRole("heading", { level: 1 })).toHaveFocus();
  });
});
