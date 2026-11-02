type Label = string | { name: string };

export interface EpicIssue {
  number: number;
  title: string;
  body: string;
  labels: Label[];
  projectItems?: unknown[];
}

const labelNames = (labels: Label[]): string[] => labels.map((label) => (typeof label === "string" ? label : label.name));

export function isEpicCandidate(issue: Pick<EpicIssue, "title" | "labels">): boolean {
  return labelNames(issue.labels).includes("epic") || /^epic(?:\s|\(|:)/i.test(issue.title.trim());
}

function hasProject4(items: unknown[] | undefined): boolean {
  return (items ?? []).some((item) => {
    if (!item || typeof item !== "object") return false;
    const project = (item as { project?: { number?: unknown } }).project;
    return project?.number === 4 || project?.number === "4";
  });
}

function hasDefinitionOfDone(body: string): boolean {
  const heading = /^#{1,3}\s*(?:definition of done|done when)\s*$/im.exec(body);
  if (!heading) return false;
  const afterHeading = body.slice(heading.index + heading[0].length);
  return afterHeading.split(/^#{1,3}\s/m, 1)[0].trim().length > 0;
}

function hasNamedOwner(body: string): boolean {
  return /^\s*owner\s*:\s*\S/im.test(body);
}

export function epicConformanceFindings(issue: EpicIssue, opts: { canReadProjects?: boolean } = {}): string[] {
  const labels = labelNames(issue.labels);
  const findings: string[] = [];
  if (!labels.includes("epic")) findings.push("missing epic label");
  if (!labels.includes("kind/feature")) findings.push("missing kind/feature label");
  if (!labels.some((label) => label.startsWith("app/") && label.length > 4)) {
    findings.push("missing canonical app/<component> label");
  }
  if (opts.canReadProjects !== false && !hasProject4(issue.projectItems)) {
    findings.push("not attached to org Project #4 (set Area, Horizon, and Kind=Epic there)");
  }
  if (!hasDefinitionOfDone(issue.body)) findings.push("missing Definition of done section");
  if (!hasNamedOwner(issue.body)) findings.push("missing named Owner: line");
  return findings;
}

function ghJson<T>(args: string[]): T {
  const result = Bun.spawnSync(["gh", ...args], { stdout: "pipe", stderr: "pipe" });
  const stdout = new TextDecoder().decode(result.stdout);
  if (result.exitCode !== 0) {
    const stderr = new TextDecoder().decode(result.stderr).trim();
    throw new Error(`gh ${args.join(" ")} failed: ${stderr || `exit ${result.exitCode}`}`);
  }
  return JSON.parse(stdout) as T;
}

export function runLiveAudit(): number {
  const common = ["--repo", "developing-today/code", "--state", "open", "--limit", "500", "--json", "number,title,body,labels"];
  const labelled = ghJson<EpicIssue[]>(["issue", "list", ...common, "--label", "epic"]);
  const titled = ghJson<EpicIssue[]>(["issue", "list", ...common, "--search", "epic in:title"]);
  const candidates = new Map<number, EpicIssue>();
  for (const issue of [...labelled, ...titled]) {
    if (isEpicCandidate(issue)) candidates.set(issue.number, issue);
  }

  const projectProbe = Bun.spawnSync(["gh", "project", "list", "--owner", "developing-today", "--limit", "1"], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const canReadProjects = projectProbe.exitCode === 0;
  if (!canReadProjects) {
    const error = new TextDecoder().decode(projectProbe.stderr).trim();
    if (!error.includes("missing required scopes") || !error.includes("read:project")) {
      throw new Error(`could not verify Project #4 access: ${error || `exit ${projectProbe.exitCode}`}`);
    }
    console.log("Project #4 membership was not checked: gh auth lacks read:project.");
  }

  let nonconforming = 0;
  for (const issue of [...candidates.values()].sort((a, b) => a.number - b.number)) {
    const project = canReadProjects
      ? ghJson<{ projectItems: unknown[] }>([
          "issue", "view", String(issue.number), "--repo", "developing-today/code", "--json", "projectItems",
        ])
      : undefined;
    const findings = epicConformanceFindings({ ...issue, projectItems: project?.projectItems }, { canReadProjects });
    if (findings.length) {
      nonconforming++;
      console.log(`#${issue.number} ${issue.title} — ${findings.join("; ")}`);
    }
  }
  console.log(`Epic conformance: ${candidates.size - nonconforming}/${candidates.size} conforming; ${nonconforming} need attention.`);
  return nonconforming;
}

if (import.meta.main) {
  try {
    process.exitCode = runLiveAudit() ? 1 : 0;
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  }
}
