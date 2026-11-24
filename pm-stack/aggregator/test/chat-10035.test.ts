// #10035 S6 (design #9539 §4/§6): the chat/session surface's view
// half — the threaded page, reply forms, cross-reference links, the
// archive toggle, and the nav unread badge. The store/API half
// landed in PR #12747; these tests pin the render contract.
import { describe, expect, test } from "bun:test";
import type { Provenance } from "../src/model.ts";
import { PAGES } from "../src/pages.ts";
import type { PromptRecord } from "../src/prompts.ts";
import { renderChat } from "../src/render.ts";

const PROV: Provenance = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
  aa: "x",
};

function prompt(overrides: Partial<PromptRecord> = {}): PromptRecord {
  return {
    scope: "page",
    target: "claude",
    mode: "async",
    page: "queue",
    sectionRef: null,
    prompt: "root question",
    replyTo: null,
    refs: [],
    notify: false,
    attachments: [],
    id: "prompt:root",
    threadId: "prompt:root",
    status: "answered",
    createdAt: "2026-11-23T01:00:00Z",
    answer: "root answer",
    archived: false,
    read: true,
    ...overrides,
  };
}

const ROOT = prompt();
const REPLY = prompt({
  id: "prompt:reply",
  threadId: "prompt:root",
  prompt: "follow-up question",
  replyTo: "prompt:root",
  status: "queued",
  answer: undefined,
  createdAt: "2026-11-23T02:00:00Z",
  read: false,
});
const OTHER = prompt({
  id: "prompt:other",
  threadId: "prompt:other",
  prompt: "unrelated thread",
  refs: ["prompt:reply"],
  createdAt: "2026-11-23T03:00:00Z",
});
const ARCHIVED = prompt({
  id: "prompt:old",
  threadId: "prompt:old",
  prompt: "archived thread",
  archived: true,
  createdAt: "2026-11-22T01:00:00Z",
});
const ALL = [ROOT, REPLY, OTHER, ARCHIVED];

describe("the chat page (#10035)", () => {
  test("is registered in the manifest with its api twin", () => {
    const page = PAGES.find((p) => p.id === "chat");
    expect(page?.route).toBe("/chat");
    expect(page?.api).toBe("/api/prompt/threads");
    expect(page?.nav).toBe("dropdown");
  });

  test("groups prompts into threads, oldest first inside a thread", () => {
    const html = renderChat(ALL, PROV);
    expect(html.split('class="chat-thread"').length - 1).toBe(2);
    // The reply renders inside the root's thread, after the root.
    const threadStart = html.indexOf('id="prompt-prompt:root"');
    const replyAt = html.indexOf('id="prompt-prompt:reply"');
    const otherAt = html.indexOf('id="prompt-prompt:other"');
    expect(threadStart).toBeGreaterThan(-1);
    expect(replyAt).toBeGreaterThan(threadStart);
    // Threads sort by latest activity: the 03:00 thread leads.
    expect(otherAt).toBeLessThan(threadStart);
    expect(html).toContain("in reply to");
    expect(html).toContain("root answer");
  });

  test("every prompt carries a reply form addressed to itself", () => {
    const html = renderChat(ALL, PROV);
    expect(html).toContain('name="replyTo" value="prompt:root"');
    expect(html).toContain('name="replyTo" value="prompt:reply"');
    expect(html).toContain('name="notify" value="true"');
  });

  test("cross-references link to the referenced prompt's card", () => {
    const html = renderChat(ALL, PROV);
    expect(html).toContain('href="#prompt-prompt:reply"');
    expect(html).toContain("Cross-references:");
  });

  test("archived prompts are hidden by default and shown, marked, on request", () => {
    const html = renderChat(ALL, PROV);
    expect(html).not.toContain("archived thread");
    expect(html).toContain("Show archived (1)");
    const withArchived = renderChat(ALL, PROV, { showArchived: true });
    expect(withArchived).toContain("archived thread");
    expect(withArchived).toContain('<span class="badge closed">archived</span>');
    expect(withArchived).toContain("Hide archived");
    expect(withArchived).toContain('data-patch-key="archived" data-patch-value="false"');
  });

  test("unread prompts are counted and offer mark-read; read ones do not", () => {
    const html = renderChat(ALL, PROV);
    expect(html).toContain("1 unread");
    expect(html).toContain('data-prompt-id="prompt:reply" data-patch-key="read" data-patch-value="true"');
    expect(html).not.toContain('data-prompt-id="prompt:root" data-patch-key="read"');
    expect(html).toContain('data-patch-key="archived" data-patch-value="true"');
  });

  test("the empty state names where submissions land", () => {
    const html = renderChat([], PROV);
    expect(html).toContain("No prompts yet");
    expect(html).toContain("0 threads · 0 unread");
  });

  test("every page carries the hidden nav unread badge and its client script", () => {
    const html = renderChat([], PROV);
    expect(html).toContain("data-prompt-unread hidden");
    expect(html).toContain("data-prompt-unread-count");
    expect(html).toContain("/assets/prompt-badge.js");
    expect(html).toContain('href="/chat" data-prompt-unread');
  });
});
