export const KIND_LABELS = {
  decision: "Decision",
  done: "Done",
  check: "Check",
  team: "Team",
  debate: "Debate",
  tasks: "Tasks",
  note: "Note",
  question: "Question",
  heard: "Your words",
  typed: "Typed",
  nixa: "Nixa",
  agent: "Agent",
  to: "To agent",
  info: "Info",
};

export const FALLBACK_KIND_LABEL = "Info";

export const kindLabel = (kind) => (kind && KIND_LABELS[kind]) || FALLBACK_KIND_LABEL;

const INFO_KIND_RULES = [
  [/^decision (?:auto|manual|automatic):|^decided for you|^your yes counts|^autonomy check/i, "decision"],
  [/^debate\b/i, "debate"],
  [/^waits for you:|^say "yes"|already answered/i, "question"],
  [/^checks\b|^all checks passed|^second opinion/i, "check"],
  [/inbox/i, "tasks"],
  [/\bteam\b|\bcrew\b/i, "team"],
  [/\bis done\b|\bfinished\b/i, "done"],
  [/^noted:|^note:|^saved\b|^chat cleaned|\brestarted\b/i, "note"],
];

export const kindOf = (item) => {
  if (item.kind) return item.kind;
  if (item.who === "user") return item.typed ? "typed" : "heard";
  if (item.who === "nixa") return "nixa";
  if (item.who === "agent") return item.to ? "to" : "agent";
  const text = String(item.text ?? "");
  for (const [pattern, kind] of INFO_KIND_RULES) {
    if (pattern.test(text)) return kind;
  }
  return "info";
};
