export type ItemOrigin = "dystil";

export type EvidenceStat = { n: string; label: string };

export type FixStep = { n: string; t: string };

export type HomeItem = {
  id: string;
  origin: ItemOrigin;
  when: string;
  short: string;
  title: string;
  evidence: EvidenceStat[];
  evidenceNote: string;
  offer: string;
  fixName: string;
  steps: FixStep[];
  saveAvailable: boolean;
};

export type CorrectionReason = "intended" | "numbers-off" | "not-worth-it";

export type CorrectionOption = {
  reason: CorrectionReason;
  label: string;
  consequence: string;
};

export type Shortcut = {
  id: string;
  title: string;
  meta: string;
  kind: string;
};

export type HomeSource = {
  items: HomeItem[];
  queue: string[];
  originalTotal: number;
  shortcuts: Shortcut[];
  lastSpokeUp: string;
  loading: boolean;
  error: string | null;
  save: (id: string) => Promise<boolean>;
  dismiss: (id: string, reason: CorrectionReason) => Promise<boolean>;
  defer: (id: string) => void;
  bringToFront: (id: string) => void;
  restore: () => void;
  reload: () => Promise<void>;
  copyShortcut: (id: string) => Promise<boolean>;
};

export const CORRECTION_OPTIONS: CorrectionOption[] = [
  {
    reason: "intended",
    label: "I meant to work that way",
    consequence: "Then it is not waste. I will stop calling this a problem.",
  },
  {
    reason: "numbers-off",
    label: "The numbers are off",
    consequence: "I will use this correction when I reconsider the finding.",
  },
  {
    reason: "not-worth-it",
    label: "Right, but not worth fixing",
    consequence: "Noted. I will keep spotting it and keep quiet about it.",
  },
];
