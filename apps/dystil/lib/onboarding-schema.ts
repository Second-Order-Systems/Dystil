export type OnboardingOption = {
  label: string;
  value: string;
  description?: string;
};

export type OnboardingVisibilityRule = {
  fieldId: string;
  equals: string | boolean;
};

type OnboardingFieldBase = {
  id: string;
  label: string;
  description?: string;
  required?: boolean;
  outputPath: string;
  showWhen?: OnboardingVisibilityRule;
};

export type OnboardingTextField = OnboardingFieldBase & {
  type: "text" | "textarea";
  placeholder?: string;
  minLength?: number;
};

export type OnboardingSelectField = OnboardingFieldBase & {
  type: "select";
  placeholder?: string;
  options: OnboardingOption[];
};

export type OnboardingMultiSelectField = OnboardingFieldBase & {
  type: "multiselect";
  options: OnboardingOption[];
  placeholder?: string;
  minSelections?: number;
  maxSelections?: number;
  allowCustomValues?: boolean;
  optionsSource?: "installed_apps";
};

export type OnboardingBooleanField = OnboardingFieldBase & {
  type: "boolean";
  trueLabel: string;
  falseLabel: string;
};

export type OnboardingField =
  | OnboardingTextField
  | OnboardingSelectField
  | OnboardingMultiSelectField
  | OnboardingBooleanField;

export type OnboardingStep = {
  id: string;
  title: string;
  description: string;
  fields: OnboardingField[];
};

export const onboardingSteps: OnboardingStep[] = [
  {
    id: "identity",
    title: "About you",
    description: "Start with the basics so Dystil knows who it is working for.",
    fields: [
      {
        id: "name",
        type: "text",
        label: "What should Dystil call you?",
        required: true,
        minLength: 2,
        outputPath: "identity.name",
        placeholder: "Your name",
      },
      {
        id: "role",
        type: "select",
        label: "Which role best matches your work?",
        required: true,
        outputPath: "identity.role",
        placeholder: "Choose your function",
        options: [
          { label: "Engineering", value: "engineering" },
          { label: "Quality Assurance", value: "quality_assurance" },
          { label: "IT & Infrastructure", value: "it_infrastructure" },
          { label: "Data & Analytics", value: "data_analytics" },
          { label: "Product", value: "product" },
          { label: "Design", value: "design" },
          { label: "Project Delivery", value: "project_delivery" },
          { label: "Research", value: "research" },
          { label: "Sales", value: "sales" },
          { label: "Marketing", value: "marketing" },
          { label: "Customer Support", value: "customer_support" },
          { label: "Operations", value: "operations" },
          { label: "Finance", value: "finance" },
          { label: "People & HR", value: "people_hr" },
          { label: "Legal", value: "legal" },
          { label: "Procurement", value: "procurement" },
          { label: "Subject Matter Expert", value: "subject_matter_expert" },
          { label: "Administration", value: "administration" },
          { label: "Leadership", value: "leadership" },
        ],
      },
    ],
  },
];
