import { DystilBrand } from "@/components/dystil-brand";
import { cn } from "@/lib/utils";

export function OnboardingTopbar({ currentStep }: { currentStep: 0 | 1 | 2 }) {
  return <div className="space-y-[18px]">
    <DystilBrand highlightY />
    <div className="flex justify-center gap-2" aria-label={`Onboarding step ${currentStep + 1} of 3`}>
      {[0, 1, 2].map((index) => <span key={index} className={cn("h-1 w-[34px] rounded-full transition-colors duration-300", index < currentStep ? "bg-primary-hover" : index === currentStep ? "bg-primary" : "bg-border")} />)}
    </div>
  </div>;
}
