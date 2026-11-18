import { describe, expect, test } from "bun:test";
import { Script } from "node:vm";
import { fixtureSource } from "../src/sources/fixtures.ts";
import {
  FRAG_WATCHDOG_MS,
  renderAttentionShell,
  renderBoardShell,
  renderOverviewShell,
  renderQueueShell,
  renderTodayShell,
} from "../src/render.ts";

// Shell fragment watchdog (#1503): a stalled/failed fragment must surface,
// not spin — every shell page carries the watchdog script, which swaps a
// still-loading skeleton for a notice with a ?full=1 link after 10 s.
describe("shell fragment watchdog (#1503)", () => {
  test("watchdog bound is 10 seconds", () => {
    expect(FRAG_WATCHDOG_MS).toBe(10_000);
  });

  test("every shell page carries the watchdog and keeps its fragment load", async () => {
    const src = fixtureSource();
    const shells = [
      renderBoardShell(src.provenance()),
      renderAttentionShell(src.provenance()),
      renderTodayShell(src.provenance(), undefined, null, await src.quota()),
      renderQueueShell(src.provenance()),
      renderOverviewShell(src.provenance()),
    ];
    expect(shells).toHaveLength(5);
    for (const html of shells) {
      // Oracle: the JavaScript parser must accept the actual rendered script,
      // not merely find recognizable strings inside invalid JavaScript (#11410).
      const watchdog = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].filter((match) =>
        match[1].includes('p.className = "frag-failed"'),
      );
      expect(watchdog).toHaveLength(1);
      expect(() => new Script(watchdog[0][1])).not.toThrow();
      // A relative ?full=1 link must resolve against this page, not a base tag.
      expect(html).not.toMatch(/<base\b/i);
      expect(html).toContain("frag-failed");
      expect(html).toContain("the live view did not arrive");
      expect(html).toContain("?full=1");
      // #10605: first paint is a Datastar v1 `data-init` (one-shot), not
      // the v0-era `data-on-load`, which the vendored bundle never fires.
      expect(html).toContain("data-init=\"@get('");
      expect(html).not.toContain("data-on-load");
      expect(html).toContain('class="skel"');
    }
  });

  test("attention shell watchdog covers all three fragment containers", () => {
    const src = fixtureSource();
    const html = renderAttentionShell(src.provenance());
    expect(html).toContain('id="attn-header"');
    expect(html).toContain('id="attn-queues"');
    expect(html).toContain('id="attn-sessions"');
    expect(html).toContain("frag-failed");
  });

  test("today shell keeps its health strip ahead of the skeleton", async () => {
    const src = fixtureSource();
    const html = renderTodayShell(src.provenance(), undefined, null, await src.quota());
    expect(html).toContain("frag-failed");
    expect(html.indexOf('class="today-health-strip"')).toBeLessThan(html.indexOf('class="skel"'));
  });
});

// Oracle: the watchdog contract, exercised with the actual emitted JavaScript
// and a minimal DOM model (no HTML parser or live browser/network required).
test("rendered watchdog replaces only stalled skeletons after its deadline (#11410)", () => {
  const html = renderAttentionShell(fixtureSource().provenance());
  const body = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].find((match) =>
    match[1].includes('p.className = "frag-failed"'),
  )![1];

  class Element {
    className = "";
    textContent = "";
    children: Element[] = [];
    attributes: Record<string, string> = {};
    constructor(readonly tagName: string) {}
    setAttribute(name: string, value: string) {
      this.attributes[name] = value;
    }
    appendChild(child: Element) {
      this.children.push(child);
      return child;
    }
    // Forbid an HTML-interpolation implementation, even one that parses.
    set innerHTML(_value: string) {
      throw new Error("watchdog must use DOM text");
    }
  }
  const notices: Element[] = [];
  function fragment(loading: boolean) {
    const skeleton = {
      replaceWith(notice: Element) {
        loading = false;
        notices.push(notice);
      },
    };
    return {
      querySelector(selector: string) {
        expect(selector).toBe(".skel");
        return loading ? skeleton : null;
      },
    };
  }
  const fragments = [fragment(true), fragment(false), fragment(true)];
  const timers: Array<{ callback: () => void; delay: number }> = [];
  new Script(body).runInNewContext({
    document: {
      querySelectorAll(selector: string) {
        expect(selector).toBe("[data-frag]");
        return fragments;
      },
      createElement(tag: string) {
        return new Element(tag);
      },
    },
    setTimeout(callback: () => void, delay: number) {
      timers.push({ callback, delay });
    },
  });
  expect(timers).toHaveLength(1);
  expect(timers[0].delay).toBe(10_000);
  expect(notices).toHaveLength(0);
  timers[0].callback();
  expect(notices).toHaveLength(2);
  for (const notice of notices) {
    expect(notice.tagName).toBe("p");
    expect(notice.className).toBe("frag-failed");
    expect(notice.textContent).toBe("the live view did not arrive — ");
    expect(notice.children).toHaveLength(1);
    const link = notice.children[0];
    expect(link.tagName).toBe("a");
    expect(link.textContent).toBe("load the full page");
    expect(link.attributes.href).toBe("?full=1");
    for (const path of ["/", "/board", "/attention", "/today", "/queue"]) {
      expect(new URL(link.attributes.href, "https://pm.example" + path + "?old=1#anchor").href).toBe(
        "https://pm.example" + path + "?full=1",
      );
    }
  }
  timers[0].callback();
  expect(notices).toHaveLength(2);
});
