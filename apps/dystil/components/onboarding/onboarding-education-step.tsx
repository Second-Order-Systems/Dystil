import { LockKeyhole, Sprout, Compass } from "lucide-react";
import { OnboardingTopbar } from "@/components/onboarding/onboarding-topbar";

export function OnboardingEducationStep({ onContinue, onBack, currentStep, totalSteps }: { onContinue: () => void; onBack: () => void; currentStep: number; totalSteps: number }) {
  const promises = [
    [LockKeyhole, "Stays on device", "Identifying details are stripped locally. Our servers never see them."],
    [Sprout, "Zero drag", "Runs only on spare CPU and battery. Steps back the second you need it."],
    [Compass, "You're in control", "Pause anytime from the menu bar. What we process is purged after."],
  ] as const;

  return (
    <div className="relative h-dvh overflow-y-auto bg-[radial-gradient(1100px_500px_at_85%_-10%,#ebf4ef_0%,transparent_60%),radial-gradient(900px_500px_at_-10%_110%,#eef3ee_0%,transparent_55%)] px-4 py-10 text-foreground">
      <div className="mx-auto w-full max-w-4xl">
        <div className="mb-[22px]"><OnboardingTopbar currentStep={currentStep} totalSteps={totalSteps} /></div>
        <section className="rounded-[20px] border border-border bg-card px-[22px] py-7 shadow-[0_1px_2px_rgba(20,32,27,.04),0_12px_40px_rgba(20,32,27,.07)] sm:px-10 sm:py-[38px]">
          <span className="mb-[18px] inline-flex items-center gap-2 rounded-full bg-primary/10 px-3 py-1.5 text-xs font-semibold uppercase tracking-wide text-primary">
            <span className="h-2 w-2 animate-pulse rounded-full bg-primary" /> Private by design
          </span>
          <h1 className="mb-3 text-[30px] font-bold leading-[1.15] tracking-[-.02em]">Before Dystil helps, here&apos;s <span className="text-primary">exactly</span> what it does.</h1>
          <p className="max-w-[52ch] text-base leading-[1.6] text-muted-foreground">Dystil watches <b className="font-semibold text-foreground">the work, not you</b>. It spots the repetitive work eating your day so it can be handed to AI agents. No tracking, no scoring, no one looking over your shoulder.</p>
          <div className="my-[26px] grid gap-[14px] sm:grid-cols-2">
            <Ledger title="What Dystil looks at" good items={["The apps and tools your work moves through", "The steps you repeat to get things done", "Where work passes from one step or teammate to the next"]} />
            <Ledger title="What it never keeps" items={["Names, faces, or anything personal about you or the people you work with", "The actual content you type, paste, or read", "Passwords, credentials, or business secrets"]} />
          </div>
          <div className="grid gap-3 sm:grid-cols-3">
            {promises.map(([Icon, title, description]) => <div key={title} className="dystil-onboarding-item rounded-[14px] border border-border p-4"><div className="mb-2.5 grid h-[34px] w-[34px] place-items-center rounded-[10px] bg-primary/10 text-primary"><Icon className="h-4 w-4" /></div><h2 className="mb-1 text-sm font-semibold">{title}</h2><p className="text-[13px] leading-[1.5] text-muted-foreground">{description}</p></div>)}
          </div>
          <div className="mt-7 flex items-center justify-between border-t border-border pt-5"><button type="button" onClick={onBack} className="px-2 py-2 text-[15px] text-muted-foreground transition hover:text-foreground">‹ Back</button><button type="button" onClick={onContinue} className="h-10 rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground shadow-[0_6px_18px_hsl(var(--primary)/.28)] transition hover:bg-primary-hover">I&apos;m in, let&apos;s get started →</button></div>
        </section>
      </div>
    </div>
  );
}

function Ledger({ title, items, good = false }: { title: string; items: string[]; good?: boolean }) {
  return <div className={`dystil-onboarding-item rounded-2xl border p-[18px] ${good ? "border-primary/20 bg-primary/[.03]" : "border-border bg-card"}`}><h2 className={`mb-2.5 flex gap-2 text-[13px] font-bold ${good ? "text-primary" : "text-muted-foreground/60"}`}><span>{good ? "✓" : "✕"}</span>{title}</h2><ul className="space-y-2">{items.map((item) => <li key={item} className={`flex gap-2 text-[13.5px] leading-[1.5] ${good ? "text-muted-foreground" : "text-muted-foreground/65"}`}><span className="mt-2 h-[5px] w-[5px] shrink-0 rounded-full bg-current opacity-50" />{item}</li>)}</ul></div>;
}
