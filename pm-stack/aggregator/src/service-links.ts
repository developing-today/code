// Service-link registry (#1136): service id -> URL templates for the live
// stack services the dashboard hands the reader off to. One module owns
// the URL shapes; sources and renderers never scatter string literals.
// Browser forms only: the public hostname for Kaneo, the tailnet
// MagicDNS hostname for the rest — never loopback or raw IPs (the
// dashboard is read over the tailnet; a loopback link strands the reader).
import type { BoardItem, RoadmapRow } from "./model.ts";

export const TAILNET_HOST = "amd-6.raptor-pumpkinseed.ts.net";

export type ServiceId = "kaneo" | "grist" | "pad" | "ready-for-agent";

export interface ServiceLinkDef {
  label: string;
  /** Browser home for the service. */
  home: string;
}

export const SERVICE_LINKS: Record<ServiceId, ServiceLinkDef> = {
  kaneo: { label: "Kaneo", home: "https://kaneo.security.cab" },
  grist: { label: "Grist", home: `http://${TAILNET_HOST}:8484` },
  pad: { label: "Pad", home: `http://${TAILNET_HOST}:7777` },
  "ready-for-agent": { label: "ready-for-agent", home: `http://${TAILNET_HOST}:6056` },
};

/**
 * Kaneo task deep link. Route shape verified against the live Kaneo web
 * bundle on 2026-10-29: /dashboard/workspace/$workspaceId/project/$projectId/task/$taskId
 * (the app's own by-ticket route redirects to exactly this canonical form).
 */
export function kaneoTaskUrl(workspaceId: string, projectId: string, taskId: string): string {
  return `${SERVICE_LINKS.kaneo.home}/dashboard/workspace/${encodeURIComponent(workspaceId)}/project/${encodeURIComponent(projectId)}/task/${encodeURIComponent(taskId)}`;
}

/** Kaneo project (board) deep link — same verified route family. */
export function kaneoProjectUrl(workspaceId: string, projectId: string): string {
  return `${SERVICE_LINKS.kaneo.home}/dashboard/workspace/${encodeURIComponent(workspaceId)}/project/${encodeURIComponent(projectId)}`;
}

/** Grist document link; the roadmap table lives in this document. */
export function gristDocUrl(docId: string): string {
  return `${SERVICE_LINKS.grist.home}/doc/${encodeURIComponent(docId)}`;
}

export interface ItemServiceLink {
  service: ServiceId;
  label: string;
  url: string;
}

/** The outbound service links carried by one board item / roadmap row. */
export function itemServiceLinks(subject: BoardItem | RoadmapRow): ItemServiceLink[] {
  const out: ItemServiceLink[] = [];
  if ("kaneoUrl" in subject && subject.kaneoUrl) {
    out.push({ service: "kaneo", label: "Kaneo card", url: subject.kaneoUrl });
  }
  if ("gristUrl" in subject && subject.gristUrl) {
    out.push({ service: "grist", label: "Grist roadmap document", url: subject.gristUrl });
  }
  return out;
}

/** Home links for every stack service (projects-page strip, #1136). */
export function serviceHomeLinks(): ItemServiceLink[] {
  return (Object.keys(SERVICE_LINKS) as ServiceId[]).map((id) => ({
    service: id,
    label: SERVICE_LINKS[id].label,
    url: SERVICE_LINKS[id].home,
  }));
}
