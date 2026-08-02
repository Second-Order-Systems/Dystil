export const signals: Array<{ title: string; description: string }> = [
  { title: "The same work, over and over", description: "You do it the same way every time. If nothing about it changes, it does not need you." },
  { title: "Work that arrives on a schedule", description: "The Monday report, the month-end close. Most of that time is setup and waiting, and it can be done before you sit down." },
  { title: "Work where you make the call", description: "The judgement has to be yours. Rebuilding the same groundwork before every one of them does not." },
  { title: "Work that could come out better", description: "The report, the reply, the summary. Done to the standard you would want if you had the time." },
  { title: "What you would do if you had the time", description: "The prep before the call, the check before the decision. Skipped because the day is full, not because it does not matter." },
];

export const readyFixes = [
  { id: "monday-update", title: "Draft the Monday update", description: "Pull the week’s completed work into the same update format you use every Monday.", evidence: "Seen across 6 Mondays · Excel and Outlook", steps: ["Collect completed items from the weekly tracker", "Draft the update in your usual section order", "Leave it ready for your review before sending"] },
  { id: "review-context", title: "Prepare context before pipeline reviews", description: "Gather recent account notes and open decisions before the recurring review call.", evidence: "Seen across 4 review cycles · HubSpot and Slack", steps: ["Find accounts on the review agenda", "Collect their latest notes and unresolved decisions", "Prepare one concise brief for the call"] },
  { id: "client-check", title: "Run the client-report check", description: "Apply the same final checks you make before a client report goes out.", evidence: "Seen across 9 reports · Google Docs and Outlook", steps: ["Check dates, totals, and client names", "Flag missing sections or unresolved comments", "Return the report with issues clearly marked"] },
];
