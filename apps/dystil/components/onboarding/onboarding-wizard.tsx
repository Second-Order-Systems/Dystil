"use client";

import { startTransition, useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { useRouter } from "next/navigation";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowRight,
  Check,
  ChevronLeft,
  Circle,
  Loader2,
  Sparkles,
} from "lucide-react";

import {
  onboardingSteps,
  type OnboardingBooleanField,
  type OnboardingField,
  type OnboardingMultiSelectField,
  type OnboardingOption,
  type OnboardingSelectField,
} from "@/lib/onboarding-schema";
import { appServerUrl } from "@/lib/constants/server";
import { useOnboardingStatus } from "@/lib/hooks/use-onboarding-status";
import { useInstalledApps } from "@/lib/hooks/use-installed-apps";
import { usePlatform } from "@/lib/hooks/use-platform";
import { cn, unflattenObject } from "@/lib/utils";
import { commands } from "@/lib/utils/tauri";
import { getAuthState, subscribeAuthState, type DystilAuthState } from "@/lib/auth-store";
import { Button } from "@/components/ui/button";
import { ToastAction } from "@/components/ui/toast";
import { toast } from "@/components/ui/use-toast";
import { OnboardingPermissionsStep } from "@/components/onboarding/onboarding-permissions-step";
import { OnboardingEducationStep } from "@/components/onboarding/onboarding-education-step";
import { OnboardingTopbar } from "@/components/onboarding/onboarding-topbar";
import { OnboardingAiSetupStep } from "@/components/onboarding/onboarding-ai-setup-step";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { MultiSelect } from "@/components/ui/multi-select";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

type OnboardingAnswerValue = string | string[] | boolean;
type OnboardingAnswers = Record<string, OnboardingAnswerValue | undefined>;

const onboardingFields = onboardingSteps.flatMap((step) => step.fields);
const MACOS_PERMISSION_STEP_ID = "macos-permissions";
const EDUCATION_STEP_ID = "privacy-education";
const AI_SETUP_STEP_ID = "ai-setup";
type ManagedProvider = "codex" | "claude";
type ProviderStatus = { state: string; authenticated?: boolean | null };

const managedProviderName = (provider: ManagedProvider) =>
  provider === "codex" ? "ChatGPT Plus" : "Claude Pro";

function providerSetupTitle(provider: ManagedProvider, stage: string, working = false) {
  return <span className="flex items-center gap-3"><span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-foreground text-background">{working ? <Loader2 className="h-4 w-4 animate-spin" /> : <Sparkles className="h-4 w-4" />}</span><span className="grid gap-0.5"><span className="text-sm font-semibold leading-none">{managedProviderName(provider)}</span><span className="text-xs font-normal leading-none text-muted-foreground">{stage}</span></span></span>;
}

function providerSetupError(error: unknown) {
  const detail = error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : "";
  return detail.trim() || "Dystil could not prepare the official provider CLI. Check your connection and try again.";
}

async function focusCurrentDystilWindow() {
  const window = getCurrentWindow();
  await window.show().catch(() => undefined);
  await window.setFocus().catch(() => undefined);
}

function setUpManagedProvider(provider: ManagedProvider) {
  const name = managedProviderName(provider);
  let stopConnectionWatch: (() => void) | undefined;
  const notice = toast({
    title: providerSetupTitle(provider, "Preparing connection", true),
    description: "Preparing the private Dystil connection. You can keep working while this finishes.",
    showWhenNotificationsDisabled: true,
    persistent: true,
  });

  const startSignIn = async () => {
    notice.update({
      id: notice.id,
      title: providerSetupTitle(provider, "Opening sign-in", true),
      description: "Opening your browser…",
      action: undefined,
      persistent: true,
      open: true,
    });
    try {
      stopConnectionWatch = await listen("ai-provider-login-updated", async () => {
        const status = await invoke<ProviderStatus>("ai_provider_status", { provider }).catch(() => null);
        if (status?.state !== "ready" || status.authenticated !== true) return;
        stopConnectionWatch?.();
        notice.update({
          id: notice.id,
          title: providerSetupTitle(provider, "Connected"),
          description: "Ask Your Work is ready to use this connection.",
          action: undefined,
          persistent: false,
          open: true,
        });
        window.setTimeout(notice.dismiss, 5000);
      });
      const loginMode = await invoke<string>("ai_provider_login", { provider });
      if (provider === "claude" && loginMode === "codeRequired") {
        notice.update({
          id: notice.id,
          title: providerSetupTitle(provider, "Finish sign-in"),
          description: "Complete the browser flow, then paste the authorization code in Settings to connect Claude.",
          persistent: true,
          open: true,
        });
        return;
      }
      notice.update({
        id: notice.id,
        title: providerSetupTitle(provider, "Sign-in is open"),
        description: "Finish in your browser. Dystil will recognize the connection when you return.",
        persistent: true,
        open: true,
      });
    } catch (error) {
      stopConnectionWatch?.();
      notice.update({
        id: notice.id,
        title: providerSetupTitle(provider, "Sign-in needs attention"),
        description: providerSetupError(error),
        variant: "destructive",
        persistent: true,
        open: true,
      });
    }
  };

  void (async () => {
    try {
      const status = await invoke<ProviderStatus>("ai_provider_status", { provider });
      if (status.state !== "ready") {
        notice.update({
          id: notice.id,
          title: providerSetupTitle(provider, "Installing privately", true),
          description: undefined,
          persistent: true,
          open: true,
        });
        await invoke("ai_provider_install", { provider });
      }
      await invoke("ai_preset_activate_managed", { providerKind: provider, model: "default" });
      // Installation can finish after onboarding has navigated away or while
      // Dystil is behind another app. Bring the app back only for the explicit
      // sign-in decision, never while package installation is still running.
      await focusCurrentDystilWindow();
      notice.update({
        id: notice.id,
        title: providerSetupTitle(provider, "Ready to connect"),
        description: "Sign in when you are ready to use Ask Your Work.",
        action: <ToastAction altText={`Sign in to ${name}`} onClick={() => void startSignIn()}>Sign in</ToastAction>,
        persistent: true,
        open: true,
      });
    } catch (error) {
      notice.update({
        id: notice.id,
        title: providerSetupTitle(provider, "Setup needs attention"),
        description: providerSetupError(error),
        variant: "destructive",
        action: <ToastAction altText={`Retry ${name} setup`} className="border-current bg-transparent text-current hover:bg-background/15" onClick={() => setUpManagedProvider(provider)}>Retry setup</ToastAction>,
        persistent: true,
        open: true,
      });
    }
  })();
}

function getOnboardingStepIds(showPermissionStep: boolean) {
  return [
    ...onboardingSteps.map((step) => step.id),
    EDUCATION_STEP_ID,
    ...(showPermissionStep ? [MACOS_PERMISSION_STEP_ID] : []),
    AI_SETUP_STEP_ID,
  ];
}

function isFieldVisible(
  field: OnboardingField,
  answers: OnboardingAnswers,
): boolean {
  if (!field.showWhen) return true;
  return answers[field.showWhen.fieldId] === field.showWhen.equals;
}

function validateField(
  field: OnboardingField,
  value: OnboardingAnswerValue | undefined,
): string | null {
  if (field.type === "boolean") {
    if (field.required && typeof value !== "boolean") {
      return "Choose one option to continue.";
    }
    return null;
  }

  if (field.type === "multiselect") {
    const selected = Array.isArray(value) ? value : [];
    if (field.required && selected.length === 0) {
      return "Select at least one option.";
    }
    if (field.minSelections && selected.length < field.minSelections) {
      return `Select at least ${field.minSelections} options.`;
    }
    if (field.maxSelections && selected.length > field.maxSelections) {
      return `Keep this to ${field.maxSelections} options or fewer.`;
    }
    return null;
  }

  const stringValue = typeof value === "string" ? value.trim() : "";

  if (field.required && stringValue.length === 0) {
    return "This field is required.";
  }

  if (
    (field.type === "text" || field.type === "textarea") &&
    field.minLength &&
    stringValue.length > 0 &&
    stringValue.length < field.minLength
  ) {
    return `Use at least ${field.minLength} characters.`;
  }

  return null;
}

function buildOutputPayload(answers: OnboardingAnswers) {
  const flattened: Record<string, OnboardingAnswerValue> = {};

  for (const field of onboardingFields) {
    if (!isFieldVisible(field, answers)) continue;
    const value = answers[field.id];
    if (typeof value === "undefined") continue;
    if (typeof value === "string" && value.trim().length === 0) continue;
    if (Array.isArray(value) && value.length === 0) continue;
    flattened[field.outputPath] = value;
  }

  return unflattenObject(flattened);
}

function normalizeRoleOptions(roles: string[]): OnboardingOption[] {
  const seen = new Set<string>();
  const options: OnboardingOption[] = [];

  for (const role of roles) {
    const normalized = role.trim();
    const key = normalized.toLowerCase();
    if (!normalized || seen.has(key)) continue;
    seen.add(key);
    options.push({
      label: normalized,
      value: normalized,
    });
  }

  return options;
}

function BooleanField({
  field,
  value,
  onChange,
}: {
  field: OnboardingBooleanField;
  value: OnboardingAnswerValue | undefined;
  onChange: (value: boolean) => void;
}) {
  const isTrue = value === true;
  const isFalse = value === false;

  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <button
        type="button"
        onClick={() => onChange(true)}
        className={cn(
          "group rounded-2xl border px-4 py-4 text-left transition-all duration-200",
          isTrue
            ? "border-primary bg-primary/10 text-foreground"
            : "border-border bg-background/70 hover:border-primary/40 hover:bg-muted/60",
        )}
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-sm font-medium">{field.trueLabel}</div>
          </div>
          <div className="mt-0.5">
            {isTrue ? (
              <Check className="h-4 w-4 text-primary" />
            ) : (
              <Circle className="h-4 w-4 text-muted-foreground/60" />
            )}
          </div>
        </div>
      </button>
      <button
        type="button"
        onClick={() => onChange(false)}
        className={cn(
          "group rounded-2xl border px-4 py-4 text-left transition-all duration-200",
          isFalse
            ? "border-primary bg-primary/10 text-foreground"
            : "border-border bg-background/70 hover:border-primary/40 hover:bg-muted/60",
        )}
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-sm font-medium">{field.falseLabel}</div>
          </div>
          <div className="mt-0.5">
            {isFalse ? (
              <Check className="h-4 w-4 text-primary" />
            ) : (
              <Circle className="h-4 w-4 text-muted-foreground/60" />
            )}
          </div>
        </div>
      </button>
    </div>
  );
}

function MultiSelectField({
  field,
  value,
  onChange,
}: {
  field: OnboardingMultiSelectField;
  value: OnboardingAnswerValue | undefined;
  onChange: (value: string[]) => void;
}) {
  return (
    <MultiSelect
      options={field.options}
      value={Array.isArray(value) ? value : []}
      onValueChange={onChange}
      placeholder={field.placeholder ?? "Select options"}
      maxCount={field.maxSelections ?? 3}
      allowCustomValues={field.allowCustomValues}
      validateCustomValue={(input) => input.trim().length > 1}
      className="min-h-11 w-full justify-between rounded-xl border-border bg-background/70"
    />
  );
}

export function OnboardingWizard() {
  const [answers, setAnswers] = useState<OnboardingAnswers>({});
  const [currentStepIndex, setCurrentStepIndex] = useState(0);
  const [attemptedAdvance, setAttemptedAdvance] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [permissionsReady, setPermissionsReady] = useState(false);
  const [aiSetupReady, setAiSetupReady] = useState(false);
  const [aiSetup, setAiSetup] = useState<{ choice: "codex" | "claude" | "local" } | null>(null);
  const [hasRestoredStoredStep, setHasRestoredStoredStep] = useState(false);
  const [authState, setAuthState] = useState<DystilAuthState>(() => getAuthState());
  const { apps: installedApps } = useInstalledApps();
  const { isMac, isLoading: isPlatformLoading } = usePlatform();
  const {
    phase: onboardingPhase,
    onboarding,
    refresh: refreshOnboardingStatus,
  } = useOnboardingStatus();
  const router = useRouter();

  const showPermissionStep = isMac;
  const stepIds = getOnboardingStepIds(showPermissionStep);
  const currentStepId = stepIds[currentStepIndex] ?? null;
  const isPermissionStep = currentStepId === MACOS_PERMISSION_STEP_ID;
  const isEducationStep = currentStepId === EDUCATION_STEP_ID;
  const isAiSetupStep = currentStepId === AI_SETUP_STEP_ID;
  const currentStep = isPermissionStep || isEducationStep || isAiSetupStep ? null : onboardingSteps[currentStepIndex];
  const visibleFields = currentStep
    ? currentStep.fields.filter((field) => isFieldVisible(field, answers))
    : [];

  const resolveMultiSelectField = (field: OnboardingMultiSelectField) => {
    if (field.optionsSource !== "installed_apps") {
      return field;
    }

    const seen = new Set<string>();
    const mergedOptions = [
      ...installedApps.map((app) => ({
        label: app,
        value: app,
        description: "Installed on this device",
      })),
      ...field.options,
    ].filter((option) => {
      const key = option.label.trim().toLowerCase();
      if (key.length === 0 || seen.has(key)) return false;
      seen.add(key);
      return true;
    });

    return {
      ...field,
      options: mergedOptions.map((option) => ({
        ...option,
        iconUrl: appServerUrl(`/app-icon?name=${encodeURIComponent(option.label)}`),
      })),
    };
  };

  const resolveSelectField = (field: OnboardingSelectField) => {
    if (field.id !== "role") {
      return field;
    }

    const orgRoles = normalizeRoleOptions(authState.user?.org?.roles ?? []);
    if (orgRoles.length === 0) {
      return field;
    }

    return {
      ...field,
      options: orgRoles,
    };
  };

  useEffect(() => {
    const unsubscribe = subscribeAuthState(setAuthState);
    return () => {
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    void refreshOnboardingStatus();
  }, [refreshOnboardingStatus]);

  useEffect(() => {
    if (hasRestoredStoredStep) return;
    if (onboardingPhase !== "ready") return;
    if (isPlatformLoading) return;

    const storedStepId = onboarding?.currentStep;
    if (storedStepId) {
      const resumedIndex = stepIds.indexOf(storedStepId);
      if (resumedIndex >= 0) {
        setCurrentStepIndex(resumedIndex);
      }
    }

    setHasRestoredStoredStep(true);
  }, [
    hasRestoredStoredStep,
    isPlatformLoading,
    isMac,
    onboarding?.currentStep,
    onboardingPhase,
  ]);

  useEffect(() => {
    if (!hasRestoredStoredStep) return;
    if (!currentStepId) return;

    void commands.setOnboardingStep(currentStepId).catch((error) => {
      console.error("Failed to persist onboarding step:", error);
    });
  }, [currentStepId, hasRestoredStoredStep]);

  const stepErrors = visibleFields.reduce<Record<string, string>>(
    (errors, field) => {
      const error = validateField(field, answers[field.id]);
      if (error) {
        errors[field.id] = error;
      }
      return errors;
    },
    {},
  );

  const updateAnswer = (fieldId: string, value: OnboardingAnswerValue) => {
    setAnswers((current) => ({
      ...current,
      [fieldId]: typeof value === "string" ? value : value,
    }));
  };

  const goBack = () => {
    startTransition(() => {
      setCurrentStepIndex((current) => Math.max(current - 1, 0));
      setAttemptedAdvance(false);
      setSubmitError(null);
    });
  };

  const goForward = () => {
    setAttemptedAdvance(true);
    if (Object.keys(stepErrors).length > 0) return;

    startTransition(() => {
      setCurrentStepIndex((current) =>
        Math.min(current + 1, stepIds.length - 1),
      );
      setAttemptedAdvance(false);
      setSubmitError(null);
    });
  };

  const finishOnboarding = async () => {
    if ((isPermissionStep && !permissionsReady) || (isAiSetupStep && !aiSetupReady) || (isAiSetupStep && !aiSetup)) return;
    setAttemptedAdvance(true);
    if (Object.keys(stepErrors).length > 0) return;

    const payload = buildOutputPayload(answers);
    const selectedWorkApps = Array.isArray(answers.work_apps)
      ? answers.work_apps
        .map((app) => (typeof app === "string" ? app.trim() : ""))
        .filter(Boolean)
      : [];
    console.log("dystil_onboarding_answers", payload);

    setIsSubmitting(true);
    setSubmitError(null);

    try {
      if (aiSetup) {
        await invoke("set_onboarding_ai_setup", { setup: aiSetup });
      }
      const capturePolicy = await commands.applyOnboardingCapturePolicy(
        selectedWorkApps,
      );
      if (capturePolicy.status !== "ok") {
        throw new Error(
          typeof capturePolicy.error === "string"
            ? capturePolicy.error
            : "Failed to apply capture privacy settings.",
        );
      }

      const result = await commands.completeOnboarding(payload);
      if (result.status !== "ok") {
        throw new Error(
          typeof result.error === "string"
            ? result.error
            : "Failed to complete onboarding.",
        );
      }

      if (aiSetup?.choice === "codex" || aiSetup?.choice === "claude") {
        setUpManagedProvider(aiSetup.choice);
      }
      router.replace("/home");
    } catch (error) {
      setSubmitError(error instanceof Error ? error.message : String(error));
    } finally {
      setIsSubmitting(false);
    }
  };

  const continueFromEducation = () => {
    goForward();
  };

  if (isEducationStep) {
    return <OnboardingEducationStep onContinue={continueFromEducation} onBack={goBack} currentStep={currentStepIndex} totalSteps={stepIds.length} />;
  }

  return (
    <div className="relative h-dvh overflow-x-hidden overflow-y-auto bg-[radial-gradient(1100px_500px_at_85%_-10%,#ebf4ef_0%,transparent_60%),radial-gradient(900px_500px_at_-10%_110%,#eef3ee_0%,transparent_55%)] px-4 py-10 scrollbar-hide">
      <div className="absolute inset-0 overflow-hidden">
        <div className="absolute left-[8%] top-[10%] h-48 w-48 rounded-full bg-primary/10 blur-3xl" />
        <div className="absolute right-[10%] top-[22%] h-64 w-64 rounded-full bg-primary/5 blur-3xl" />
        <div className="absolute bottom-[6%] left-[22%] h-56 w-56 rounded-full bg-muted blur-3xl" />
        <div className="absolute inset-0 bg-[radial-gradient(circle_at_top,transparent_0,transparent_42%,rgba(255,255,255,0.02)_100%)]" />
      </div>

      <div className="relative mx-auto w-full max-w-4xl">
        <div className="w-full space-y-6">
          <OnboardingTopbar currentStep={currentStepIndex} totalSteps={stepIds.length} />
          {/* <div className="flex items-center gap-3 rounded-[24px] border border-border/70 bg-background/75 px-4 py-3 backdrop-blur sm:px-5"> */}
          {/*   <Badge variant="outline" className="rounded-full px-3 py-1"> */}
          {/*     Guided setup */}
          {/*   </Badge> */}
          {/*   <div className="rounded-full border border-border bg-muted/70 p-2"> */}
          {/*     <Sparkles className="h-4 w-4 text-primary" /> */}
          {/*   </div> */}
          {/*   <div className="shrink-0 text-xs uppercase tracking-[0.18em] text-muted-foreground"> */}
          {/*     {currentStepIndex + 1} / {onboardingSteps.length} */}
          {/*   </div> */}
          {/*   <Progress value={progress} className="h-2 flex-1 rounded-full bg-muted" /> */}
          {/* </div> */}

          <Card className="rounded-[24px] border-border/80 bg-card shadow-[0_1px_2px_rgba(20,32,27,.04),0_12px_40px_rgba(20,32,27,.07)]">
            <CardHeader className="space-y-3 px-[22px] pb-0 pt-7 sm:px-10 sm:pt-[38px]">
              <div className="flex items-center justify-between gap-4">
                <CardTitle className="text-[30px] font-bold leading-[1.15] normal-case tracking-[-.02em]">
                  {isPermissionStep ? "Let’s switch these on together." : isAiSetupStep ? "Choose how Dystil helps you return to your work." : currentStep?.id === "identity" ? <>Tell us about <span className="text-primary">yourself.</span></> : currentStep?.title}
                </CardTitle>
              </div>
              <CardDescription className="max-w-[52ch] text-base leading-[1.6]">
                {isPermissionStep
                  ? "macOS asks you to confirm each one in System Settings. We’ll show you exactly where to tap. It takes about 20 seconds."
                  : currentStep?.description}
              </CardDescription>
            </CardHeader>

            <CardContent className="px-[22px] pb-7 pt-6 sm:px-10 sm:pb-8">
              <AnimatePresence mode="wait">
                <motion.div
                  key={isPermissionStep ? MACOS_PERMISSION_STEP_ID : currentStep?.id}
                  initial={{ opacity: 0, y: 16 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -12 }}
                  transition={{ duration: 0.22, ease: "easeOut" }}
                  className="space-y-6"
                >
                  {isPermissionStep ? (
                    <OnboardingPermissionsStep onReadyChange={setPermissionsReady} />
                  ) : isAiSetupStep ? (
                    <OnboardingAiSetupStep onReadyChange={setAiSetupReady} onSetupChange={setAiSetup} />
                  ) : visibleFields.map((field) => {
                    const error = attemptedAdvance ? stepErrors[field.id] : null;
                    const value = answers[field.id];
                    const resolvedSelectField =
                      field.type === "select" ? resolveSelectField(field) : null;

                    return (
                      <div key={field.id} className="dystil-onboarding-item space-y-3">
                        <div className="space-y-1">
                          <label className="text-sm font-medium text-foreground">
                            {field.label}
                          </label>
                          {field.description ? (
                            <p className="text-sm text-muted-foreground">
                              {field.description}
                            </p>
                          ) : null}
                        </div>

                        {field.type === "text" ? (
                          <Input
                            value={typeof value === "string" ? value : ""}
                            onChange={(event) =>
                              updateAnswer(field.id, event.target.value)
                            }
                            placeholder={field.placeholder}
                            className="h-12 rounded-xl border-border bg-background/70"
                          />
                        ) : null}

                        {field.type === "textarea" ? (
                          <Textarea
                            value={typeof value === "string" ? value : ""}
                            onChange={(event) =>
                              updateAnswer(field.id, event.target.value)
                            }
                            placeholder={field.placeholder}
                            rows={5}
                            className="rounded-2xl border-border bg-background/70"
                          />
                        ) : null}

                        {resolvedSelectField ? (
                          <Select
                            value={
                              typeof value === "string" && value.length > 0
                                ? value
                                : undefined
                            }
                            onValueChange={(nextValue) =>
                              updateAnswer(field.id, nextValue)
                            }
                          >
                            <SelectTrigger className="h-12 rounded-xl border-border bg-background/70 font-sans">
                              <SelectValue placeholder={resolvedSelectField.placeholder} />
                            </SelectTrigger>
                            <SelectContent>
                              {resolvedSelectField.options.map((option) => (
                                <SelectItem key={option.value} value={option.value}>
                                  {option.label}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        ) : null}

                        {field.type === "multiselect" ? (
                          <MultiSelectField
                            field={resolveMultiSelectField(field)}
                            value={value}
                            onChange={(nextValue) => updateAnswer(field.id, nextValue)}
                          />
                        ) : null}

                        {field.type === "boolean" ? (
                          <BooleanField
                            field={field}
                            value={value}
                            onChange={(nextValue) => updateAnswer(field.id, nextValue)}
                          />
                        ) : null}

                        {error ? (
                          <p className="text-sm text-destructive">{error}</p>
                        ) : null}
                      </div>
                    );
                  })}
                </motion.div>
              </AnimatePresence>

              {submitError ? (
                <div className="mt-6 rounded-xl border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                  {submitError}
                </div>
              ) : null}

              <div className="mt-8 flex flex-col gap-3 border-t border-border/70 pt-6 sm:flex-row sm:items-center sm:justify-between">
                {stepIds.length > 1 ? <Button
                  variant="ghost"
                  onClick={goBack}
                  disabled={currentStepIndex === 0 || isSubmitting}
                >
                  <ChevronLeft className="h-4 w-4" />
                  Back
                </Button>
                  : <div></div>
                }
                <div className="flex items-center gap-3">
                  {currentStepIndex === stepIds.length - 1 ? (
                    <Button
                      onClick={finishOnboarding}
                      disabled={
                        isSubmitting ||
                        (isPermissionStep && !permissionsReady) ||
                        (isAiSetupStep && (!aiSetupReady || !aiSetup))
                      }
                      className="h-10 rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground shadow-[0_6px_18px_hsl(var(--primary)/.28)] hover:bg-primary-hover"
                    >
                      {isSubmitting ? "Finishing..." : "Finish setup"}
                    </Button>
                  ) : (
                    <Button
                      onClick={goForward}
                      disabled={
                        isSubmitting ||
                        (currentStepIndex === onboardingSteps.length - 1 && isPlatformLoading)
                      }
                      className="h-10 rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground shadow-[0_6px_18px_hsl(var(--primary)/.28)] hover:bg-primary-hover"
                    >
                      Continue
                      <ArrowRight className="h-4 w-4" />
                    </Button>
                  )}
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
