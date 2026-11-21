/** GitHub Projects read by the fleet-feed collector (#9062).
 * Keep node IDs, organization identity, and stable attribution together. */
export interface BoardDef {
  id: string;
  org: string;
  number: number;
  nodeId: string;
  url: string;
  role: "overall" | "project";
  parentBoardId?: string;
  siblingOrder: number;
  title: string;
}

export const PROJECT_BOARDS: readonly BoardDef[] = [
  {
    id: "devtoday-main",
    org: "developing-today",
    number: 4,
    nodeId: "PVT_kwDOAZNvF84Bouay",
    url: "https://github.com/orgs/developing-today/projects/4",
    role: "overall",
    siblingOrder: 0,
    title: "Developing Today",
  },
  {
    id: "forks-main",
    org: "developing-today-forks",
    number: 1,
    nodeId: "PVT_kwDOCEOo084BrYAh",
    url: "https://github.com/orgs/developing-today-forks/projects/1",
    role: "project",
    parentBoardId: "devtoday-main",
    siblingOrder: 1,
    title: "Developing Today Forks",
  },
  {
    id: "lab-cluster",
    org: "Clustered-Computing",
    number: 1,
    nodeId: "PVT_kwDOAlO1HM4BrYAm",
    url: "https://github.com/orgs/Clustered-Computing/projects/1",
    role: "project",
    parentBoardId: "devtoday-main",
    siblingOrder: 2,
    title: "Lab Cluster",
  },
];

/** Reject ambiguous identities or dangling/cyclic parent references at startup/test time. */
export function validateBoardRegistry(boards: readonly BoardDef[]): string[] {
  const problems: string[] = [];
  const byId = new Map<string, BoardDef>();
  for (const board of boards) {
    if (!board.id.trim()) problems.push("board id must not be empty");
    if (byId.has(board.id)) problems.push(`duplicate board id: ${board.id}`);
    byId.set(board.id, board);
    if (!Number.isInteger(board.number) || board.number < 1) problems.push(`invalid project number: ${board.id}`);
    if (!board.nodeId.startsWith("PVT_")) problems.push(`invalid project node id: ${board.id}`);
    try {
      if (new URL(board.url).pathname !== `/orgs/${board.org}/projects/${board.number}`)
        problems.push(`project URL does not match org/number: ${board.id}`);
    } catch {
      problems.push(`invalid project URL: ${board.id}`);
    }
  }
  const identities = new Set<string>();
  for (const board of boards) {
    const identity = `${board.org.toLowerCase()}#${board.number}`;
    if (identities.has(identity)) problems.push(`duplicate project identity: ${identity}`);
    identities.add(identity);
    if (board.parentBoardId && !byId.has(board.parentBoardId))
      problems.push(`unknown parent ${board.parentBoardId}: ${board.id}`);
    const visited = new Set([board.id]);
    let parent = board.parentBoardId;
    while (parent) {
      if (visited.has(parent)) {
        problems.push(`board hierarchy cycle: ${board.id}`);
        break;
      }
      visited.add(parent);
      parent = byId.get(parent)?.parentBoardId;
    }
  }
  return problems;
}
