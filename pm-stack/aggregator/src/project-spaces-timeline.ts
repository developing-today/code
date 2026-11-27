// C7 leaf 4 — project spaces and reports timeline index (#12351; design
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c7-rollups-design.md, "Design pages, project spaces, and timeline").
//
// One canonical landing model per project space plus a quarter-first
// timeline index over stored C7 report records. Pure module: no I/O, no
// clock except the caller-supplied `asOf`.
//
// What this module refuses to do, stated once:
//
// * It never invents a missing source section. A section the caller did
//   not supply is `available: false` and renders as "not available".
// * It never owns report records, designs, search, mobile layout, or
//   ingestion. Timeline nodes carry record ids, design refs, and special
//   report references; C6 supplies the page index and mobile mechanics
//   and C8 supplies ingestion/search over the exported reference entries.
// * It never guesses chronology. Membership is by the records' stored
//   period starts converted to America/Chicago wall dates; uncertain
//   boundaries are never presented as exact.

import type { ReportRecord } from "./report-records.ts";

export const SPACES = ["Fleet", "PM stack", "Atlas", "ID"] as const;
export type SpaceKey = (typeof SPACES)[number];

export type SpaceSectionName =
  | "exists"
  | "wanted"
  | "in-progress"
  | "designed"
  | "inspiration"
  | "end state"
  | "open research";

export interface SpaceLink {
  label: string;
  ref: string;
}

export interface ProjectSpaceInput {
  key: SpaceKey;
  title: string;
  exists?: string;
  wanted?: string;
  "in-progress"?: string;
  designed?: string;
  inspiration?: string;
  "end state"?: string;
  "open research"?: string[];
  reportLinks?: SpaceLink[];
  designLinks?: SpaceLink[];
  related?: SpaceKey[];
}

export interface SpaceSection {
  name: SpaceSectionName | "open research" | "reports" | "designs" | "related";
  available: boolean;
  text: string | null;
}

export interface ProjectSpace {
  key: SpaceKey;
  title: string;
  sections: SpaceSection[];
  related: SpaceKey[];
}

const SECTION_NAMES: SpaceSectionName[] = [
  "exists",
  "wanted",
  "in-progress",
  "designed",
  "inspiration",
  "end state",
  "open research",
];

function textSection(name: SpaceSectionName, value: string | string[] | undefined): SpaceSection {
  if (value === undefined) return { name, available: false, text: null };
  const text = Array.isArray(value) ? value.join("\n") : value;
  if (!text.trim()) return { name, available: false, text: null };
  return { name, available: true, text };
}

/** Build one stable landing space; missing sections stay unavailable, never invented. */
export function buildProjectSpace(input: ProjectSpaceInput): ProjectSpace {
  if (!(SPACES as readonly string[]).includes(input.key)) {
    throw new Error(`project-spaces: unknown space ${input.key}`);
  }
  for (const key of input.related ?? []) {
    if (!(SPACES as readonly string[]).includes(key)) {
      throw new Error(`project-spaces: unknown related space ${key}`);
    }
  }
  const sections: SpaceSection[] = [
    textSection("exists", input.exists),
    textSection("wanted", input.wanted),
    textSection("in-progress", input["in-progress"]),
    textSection("designed", input.designed),
    textSection("inspiration", input.inspiration),
    textSection("end state", input["end state"]),
    textSection("open research", input["open research"]),
    {
      name: "reports",
      available: (input.reportLinks ?? []).length > 0,
      text: (input.reportLinks ?? []).map((l) => `${l.label} (${l.ref})`).join("\n") || null,
    },
    {
      name: "designs",
      available: (input.designLinks ?? []).length > 0,
      text: (input.designLinks ?? []).map((l) => `${l.label} (${l.ref})`).join("\n") || null,
    },
    {
      name: "related",
      available: (input.related ?? []).length > 0,
      text: (input.related ?? []).join(", ") || null,
    },
  ];
  void SECTION_NAMES;
  return { key: input.key, title: input.title, sections, related: [...(input.related ?? [])] };
}

/** Render one landing space as Markdown; unavailable sections are explicit. */
export function renderProjectSpace(space: ProjectSpace): string {
  const lines = [`# ${space.title}`, ""];
  for (const section of space.sections) {
    if (section.name === "reports" || section.name === "designs" || section.name === "related") {
      lines.push(section.available ? `## ${section.name}\n\n${section.text}` : `## ${section.name}\n\nnot available`);
    } else {
      lines.push(section.available ? `## ${section.name}\n\n${section.text}` : `## ${section.name}: not available`);
    }
    lines.push("");
  }
  return `${lines.join("\n").trim()}\n`;
}

export interface DesignMilestone {
  ref: string;
  title: string;
  /** America/Chicago calendar day the milestone attaches to. */
  day: string;
}

export interface SpecialReport {
  id: string;
  title: string;
  /** America/Chicago calendar day the report attaches to. */
  day: string;
}

export interface TimelineDay {
  day: string;
  recordIds: string[];
  designMilestones: DesignMilestone[];
  specialReports: SpecialReport[];
}

export interface TimelineWeek {
  week: string;
  days: TimelineDay[];
}

export interface TimelineMonth {
  month: string;
  weeks: TimelineWeek[];
}

export interface TimelineQuarter {
  quarter: string;
  current: boolean;
  empty: boolean;
  months: TimelineMonth[];
}

export interface TimelineRecordRef {
  recordId: string;
  periodKind: string;
  periodId: string;
}

export interface TimelineIndex {
  asOf: string;
  currentQuarter: string;
  quarters: TimelineQuarter[];
  records: TimelineRecordRef[];
}

export interface BuildTimelineOptions {
  records: readonly ReportRecord[];
  asOf: string;
  designMilestones?: readonly DesignMilestone[];
  specialReports?: readonly SpecialReport[];
}

const ctDate = new Intl.DateTimeFormat("en-CA", {
  timeZone: "America/Chicago",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
});

function ctWallDate(at: string): string {
  const parsed = new Date(Date.parse(at));
  if (Number.isNaN(parsed.getTime())) throw new Error(`project-spaces-timeline: invalid asOf ${at}`);
  return ctDate.format(parsed);
}

function quarterOf(month: string): string {
  const [year, m] = month.split("-").map(Number) as [number, number];
  return `${year}-Q${Math.floor((m - 1) / 3) + 1}`;
}

function mondayOfWeek(weekId: string): string {
  const match = /^weekly:(.+?):America\/Chicago$/.exec(weekId);
  if (!match) throw new Error(`project-spaces-timeline: malformed weekly period ${weekId}`);
  return match[1]!;
}

/** Build the quarter-first timeline index; the current quarter always exists. */
export function buildTimelineIndex(options: BuildTimelineOptions): TimelineIndex {
  const wall = ctWallDate(options.asOf);
  const currentQuarter = quarterOf(wall.slice(0, 7));
  const milestones = [...(options.designMilestones ?? [])].sort((a, b) => a.day.localeCompare(b.day) || a.ref.localeCompare(b.ref));
  const specials = [...(options.specialReports ?? [])].sort((a, b) => a.day.localeCompare(b.day) || a.id.localeCompare(b.id));

  const dayIds = new Set<string>();
  const weekToDays = new Map<string, Set<string>>();
  const monthToWeeks = new Map<string, Set<string>>();
  const quarterToMonths = new Map<string, Set<string>>();
  const dayRecords = new Map<string, string[]>();
  const recordRefs: TimelineRecordRef[] = [];

  for (const record of options.records) {
    recordRefs.push({ recordId: record.id, periodKind: record.period.kind, periodId: record.period.id });
    if (record.period.kind === "daily") {
      const day = ctWallDate(record.period.periodStart);
      dayIds.add(day);
      (dayRecords.get(day) ?? dayRecords.set(day, []).get(day)!).push(record.id);
    } else if (record.period.kind === "weekly") {
      const monday = mondayOfWeek(record.period.id);
      if (!monthToWeeks.has(wall.slice(0, 7))) void 0;
      // Register the week under its month and quarter via its Monday wall date.
      const month = monday.slice(0, 7);
      const quarter = quarterOf(month);
      if (!quarterToMonths.has(quarter)) quarterToMonths.set(quarter, new Set());
      quarterToMonths.get(quarter)!.add(month);
      if (!monthToWeeks.has(month)) monthToWeeks.set(month, new Set());
      monthToWeeks.get(month)!.add(monday);
      if (!weekToDays.has(monday)) weekToDays.set(monday, new Set());
    } else if (record.period.kind === "monthly") {
      const month = /^monthly:(.+?):America\/Chicago$/.exec(record.period.id)?.[1];
      if (!month) throw new Error(`project-spaces-timeline: malformed monthly period ${record.period.id}`);
      const quarter = quarterOf(month);
      if (!quarterToMonths.has(quarter)) quarterToMonths.set(quarter, new Set());
      quarterToMonths.get(quarter)!.add(month);
      if (!monthToWeeks.has(month)) monthToWeeks.set(month, new Set());
    } else if (record.period.kind === "quarterly") {
      const quarter = /^quarterly:(.+?):America\/Chicago$/.exec(record.period.id)?.[1];
      if (!quarter) throw new Error(`project-spaces-timeline: malformed quarterly period ${record.period.id}`);
      if (!quarterToMonths.has(quarter)) quarterToMonths.set(quarter, new Set());
    }
  }

  // Every daily record implies its week, month, and quarter containers.
  for (const day of dayIds) {
    const month = day.slice(0, 7);
    const quarter = quarterOf(month);
    if (!quarterToMonths.has(quarter)) quarterToMonths.set(quarter, new Set());
    quarterToMonths.get(quarter)!.add(month);
    if (!monthToWeeks.has(month)) monthToWeeks.set(month, new Set());
  }

  // Attach daily ids to the week that contains them: prefer an explicit
  // weekly record whose Monday is the week's start, else derive the Monday.
  const explicitWeeks = new Set(weekToDays.keys());
  for (const day of dayIds) {
    const parsed = new Date(`${day}T12:00:00Z`);
    const dow = (parsed.getUTCDay() + 6) % 7;
    const monday = new Date(parsed.getTime() - dow * 86_400_000).toISOString().slice(0, 10);
    const week = explicitWeeks.has(monday) ? monday : monday;
    if (!weekToDays.has(week)) weekToDays.set(week, new Set());
    weekToDays.get(week)!.add(day);
    const month = day.slice(0, 7);
    monthToWeeks.get(month)!.add(week);
  }

  // Milestones and specials imply their day containers too.
  for (const item of [...milestones.map((m) => m.day), ...specials.map((s) => s.day)]) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(item)) throw new Error(`project-spaces-timeline: malformed day ${item}`);
    dayIds.add(item);
    const month = item.slice(0, 7);
    const quarter = quarterOf(month);
    if (!quarterToMonths.has(quarter)) quarterToMonths.set(quarter, new Set());
    quarterToMonths.get(quarter)!.add(month);
    if (!monthToWeeks.has(month)) monthToWeeks.set(month, new Set());
    const parsed = new Date(`${item}T12:00:00Z`);
    const dow = (parsed.getUTCDay() + 6) % 7;
    const monday = new Date(parsed.getTime() - dow * 86_400_000).toISOString().slice(0, 10);
    if (!weekToDays.has(monday)) weekToDays.set(monday, new Set());
    weekToDays.get(monday)!.add(item);
    monthToWeeks.get(month)!.add(monday);
  }

  if (!quarterToMonths.has(currentQuarter)) quarterToMonths.set(currentQuarter, new Set());

  const milestoneByDay = new Map<string, DesignMilestone[]>();
  for (const m of milestones) {
    if (!milestoneByDay.has(m.day)) milestoneByDay.set(m.day, []);
    milestoneByDay.get(m.day)!.push(m);
  }
  const specialByDay = new Map<string, SpecialReport[]>();
  for (const s of specials) {
    if (!specialByDay.has(s.day)) specialByDay.set(s.day, []);
    specialByDay.get(s.day)!.push(s);
  }

  const quarters: TimelineQuarter[] = [...quarterToMonths.keys()]
    .sort()
    .map((quarter) => {
      const months: TimelineMonth[] = [...quarterToMonths.get(quarter)!].sort().map((month) => {
        const weeks: TimelineWeek[] = [...(monthToWeeks.get(month) ?? new Set())]
          .sort()
          .filter((week) => {
            // A week belongs to the month holding most of its days; keep it
            // under every month that has one of its days (single-parent by day).
            const days = [...(weekToDays.get(week) ?? new Set())];
            return days.some((d) => d.slice(0, 7) === month);
          })
          .map((week) => {
            const days: TimelineDay[] = [...(weekToDays.get(week) ?? new Set())]
              .sort()
              .filter((day) => day.slice(0, 7) === month)
              .map((day) => ({
                day,
                recordIds: [...(dayRecords.get(day) ?? [])].sort(),
                designMilestones: [...(milestoneByDay.get(day) ?? [])],
                specialReports: [...(specialByDay.get(day) ?? [])],
              }));
            return { week, days };
          });
        return { month, weeks };
      });
      const hasContent =
        recordRefs.some((r) => {
          if (r.periodKind === "quarterly") return r.periodId.includes(quarter);
          if (r.periodKind === "monthly") return quarterOf(r.periodId.slice("monthly:".length, "monthly:".length + 7)) === quarter;
          return quarterToMonths.get(quarter)!.size > 0 && options.records.some((rec) => rec.period.kind !== "quarterly" && rec.period.kind !== "monthly");
        }) ||
        months.some((m) => m.weeks.some((w) => w.days.length > 0));
      return { quarter, current: quarter === currentQuarter, empty: !hasContent, months };
    });

  return { asOf: options.asOf, currentQuarter, quarters, records: recordRefs.sort((a, b) => a.recordId.localeCompare(b.recordId)) };
}

export interface TimelineLinkCheck {
  recordIds: Set<string>;
  designRefs: Set<string>;
}

/** Verify every timeline reference resolves; returns human-readable missing links. */
export function verifyTimelineLinks(timeline: TimelineIndex, known: TimelineLinkCheck): string[] {
  const missing: string[] = [];
  for (const ref of timeline.records) {
    if (!known.recordIds.has(ref.recordId)) missing.push(`record ${ref.recordId}`);
  }
  const walk = (quarters: TimelineQuarter[]) => {
    for (const q of quarters)
      for (const m of q.months)
        for (const w of m.weeks)
          for (const d of w.days)
            for (const milestone of d.designMilestones) {
              if (!known.designRefs.has(milestone.ref)) missing.push(`design ${milestone.ref}`);
            }
  };
  walk(timeline.quarters);
  return missing.sort();
}

export interface TimelineIndexEntry {
  id: string;
  kind: "quarter" | "month" | "week" | "day" | "milestone" | "special";
  text: string;
}

/**
 * Reference entries for the external C6 page index and C8 ingestion.
 * This is the integration contract: C7 exports references, it never
 * implements search, mobile layout, or ingestion itself.
 */
export function timelineIndexEntries(timeline: TimelineIndex): TimelineIndexEntry[] {
  const entries: TimelineIndexEntry[] = [];
  for (const q of timeline.quarters) {
    entries.push({ id: `quarter:${q.quarter}`, kind: "quarter", text: `Quarter ${q.quarter}${q.current ? " (current)" : ""}` });
    for (const m of q.months) {
      entries.push({ id: `month:${m.month}`, kind: "month", text: `Month ${m.month}` });
      for (const w of m.weeks) {
        entries.push({ id: `week:${w.week}`, kind: "week", text: `Week of ${w.week}` });
        for (const d of w.days) {
          entries.push({
            id: `day:${d.day}`,
            kind: "day",
            text: `Day ${d.day}: ${d.recordIds.length} record(s), ${d.designMilestones.length} design milestone(s), ${d.specialReports.length} special report(s)`,
          });
          for (const milestone of d.designMilestones) {
            entries.push({ id: `milestone:${milestone.ref}@${d.day}`, kind: "milestone", text: `${milestone.title} (${milestone.ref}) — ${d.day}` });
          }
          for (const special of d.specialReports) {
            entries.push({ id: `special:${special.id}`, kind: "special", text: `${special.title} — ${d.day}` });
          }
        }
      }
    }
  }
  return entries;
}
