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
      {
        id: "uses_social_media_for_work",
        type: "boolean",
        label: "Do you use social media for work?",
        description: "If yes, Dystil will ask which apps matter most in that part of your workflow.",
        required: true,
        outputPath: "identity.usesSocialMediaForWork",
        trueLabel: "Yes",
        falseLabel: "No",
      },
      {
        id: "work_apps",
        type: "multiselect",
        label: "Which apps are central to your work?",
        description: "Pick the main apps you live in. You can also add your own.",
        required: true,
        minSelections: 1,
        maxSelections: 8,
        allowCustomValues: true,
        outputPath: "tools.socialMediaApps",
        placeholder: "Select work apps",
        showWhen: {
          fieldId: "uses_social_media_for_work",
          equals: true,
        },
        options: [
          {
            label: "Instagram",
            value: "instagram",
            description: "Content, DMs, and creator or brand workflow",
          },
          {
            label: "WhatsApp",
            value: "whatsapp",
            description: "Client, community, and team messaging",
          },
          {
            label: "X",
            value: "x",
            description: "Audience, posting, and market monitoring",
          },
          {
            label: "TikTok",
            value: "tiktok",
            description: "Short-form content creation and publishing",
          },
          {
            label: "Facebook",
            value: "facebook",
            description: "Pages, groups, ads, and community management",
          },
          {
            label: "Messenger",
            value: "messenger",
            description: "Facebook direct messages and customer conversations",
          },
          {
            label: "Telegram",
            value: "telegram",
            description: "Channels, groups, and direct messaging",
          },
          {
            label: "Discord",
            value: "discord",
            description: "Communities, support, and team coordination",
          },
          {
            label: "YouTube",
            value: "youtube",
            description: "Publishing, comments, and channel operations",
          },
          {
            label: "Reddit",
            value: "reddit",
            description: "Community engagement and market research",
          }
        ],
      }
    ],
  },
];
