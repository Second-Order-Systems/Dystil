import { cn } from "@/lib/utils";

export function DystilMark({ className, animate = false }: { className?: string; animate?: boolean }) {
  return (
    <svg viewBox="0 0 100 110" className={cn("overflow-visible", className)} aria-hidden="true">
      {animate && <ellipse className="dystil-home-drop-glow" cx="50" cy="84" rx="20" ry="18" />}
      <path className={animate ? "dystil-home-funnel" : undefined} d="M28 22 L50 56 L72 22 M50 56 L50 62" fill="none" stroke="currentColor" strokeWidth="7" strokeLinecap="round" strokeLinejoin="round" />
      <path className={animate ? "dystil-home-drop" : undefined} d="M50 64 C58 74 63 80 63 87 C63 94 57 99 50 99 C43 99 37 94 37 87 C37 80 42 74 50 64 Z" fill="hsl(var(--primary))" />
      {animate && <>
        <circle className="dystil-home-speck dystil-home-speck-1" cx="50" cy="56" r="3" />
        <circle className="dystil-home-speck dystil-home-speck-2" cx="50" cy="56" r="2.6" />
        <circle className="dystil-home-speck dystil-home-speck-3" cx="50" cy="56" r="3.2" />
        <circle className="dystil-home-speck dystil-home-speck-4" cx="50" cy="56" r="2.4" />
      </>}
    </svg>
  );
}

export function DystilWordmark({ className, highlightY = false }: { className?: string; highlightY?: boolean }) {
  return <span className={cn("font-sans text-[15px] font-semibold tracking-[0.34em]", className)}>{highlightY ? <><span>D</span><span className="text-primary">Y</span><span>STIL</span></> : "DYSTIL"}</span>;
}

export function DystilBrand({ className, highlightY = false, animate = true, vertical = false, large = false }: { className?: string; highlightY?: boolean; animate?: boolean; vertical?: boolean; large?: boolean }) {
  return (
    <div className={cn("flex items-center justify-center gap-3", vertical && "flex-col gap-4", className)} aria-label="Dystil">
      <div className={cn("grid h-[34px] w-[34px] place-items-center rounded-[10px] bg-[#f5f1e9] text-foreground shadow-[0_4px_10px_rgba(20,20,20,0.08)]", large && "h-[92px] w-[92px] rounded-[25px] shadow-[0_16px_38px_rgba(20,20,20,0.10)]")}>
        <DystilMark className={cn("h-[22px] w-5", large && "h-[62px] w-[58px]")} animate={animate} />
      </div>
      <DystilWordmark highlightY={highlightY} className={large ? "text-[28px] tracking-[0.42em]" : undefined} />
    </div>
  );
}
