export const CAPTURE_CATEGORIES = [
  { id: "jobBoards", title: "Job boards and CVs", description: "Indeed, Greenhouse, Lever and similar sites" },
  { id: "personalMessaging", title: "Personal messaging", description: "WhatsApp, Messages, Telegram, Messenger, Discord" },
  { id: "personalEmail", title: "Personal email", description: "Mail and personal webmail" },
  { id: "hrLegal", title: "HR and legal portals", description: "Workday, BambooHR, DocuSign and Deel" },
  { id: "payrollSalary", title: "Payroll and salary", description: "ADP, Paychex, Gusto and Rippling" },
] as const;

export const CAPTURE_CATEGORY_COPY = Object.fromEntries(
  CAPTURE_CATEGORIES.map((category) => [category.id, { title: category.title, description: category.description }]),
) as Record<string, { title: string; description: string }>;
