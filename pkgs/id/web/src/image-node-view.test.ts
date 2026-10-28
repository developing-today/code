/**
 * Tests for the image NodeView resize handles (#1389).
 *
 * The west handles (nw/sw) sit on the LEFT edge of the image, so their
 * drag direction is inverted relative to the east handles: dragging
 * outward (left, negative dx) must grow the image, dragging inward must
 * shrink it. Before the fix, startResize discarded the corner and every
 * handle used `startWidth + dx`, so nw/sw dragged in the wrong direction
 * and mouse-up persisted the wrong width/height attrs.
 *
 * happy-dom has no layout engine, so the tests stub offsetWidth /
 * offsetHeight on the img to reflect its inline style — exactly what a
 * real browser's layout would report — and on the editor DOM for the
 * max-width clamp.
 */

import type { Node as PMNode } from "prosemirror-model";
import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, describe, expect, it } from "vitest";
import { richSchema } from "./editor";
import { ImageNodeView } from "./image-node-view";

const START_W = 200;
const START_H = 100;
const EDITOR_W = 1000;

/** Build a view whose doc is one paragraph holding one 200×100 image. */
function createViewWithImage(): { view: EditorView; container: HTMLElement; imageNode: PMNode } {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const imageNode = richSchema.nodes.image.create({
    src: "/api/images/abc123",
    alt: "cat",
    width: START_W,
    height: START_H,
  });
  const doc = richSchema.node("doc", null, [richSchema.node("paragraph", null, [imageNode])]);
  const state = EditorState.create({ schema: richSchema, doc });
  const view = new EditorView(container, { state });
  Object.defineProperty(view.dom, "offsetWidth", { configurable: true, get: () => EDITOR_W });
  return { view, container, imageNode };
}

/** Create the NodeView under test and stub layout sizes from inline style. */
function createNodeView(view: EditorView, imageNode: PMNode): { nv: ImageNodeView; img: HTMLImageElement } {
  // The image node is the first child of the paragraph at position 0.
  const nv = new ImageNodeView(imageNode, view, () => 1);
  document.body.appendChild(nv.dom);
  const img = nv.dom.querySelector<HTMLImageElement>("img");
  if (!img) throw new Error("node view did not render an img");
  Object.defineProperty(img, "offsetWidth", {
    configurable: true,
    get() {
      return Number.parseFloat(this.style.width) || 0;
    },
  });
  Object.defineProperty(img, "offsetHeight", {
    configurable: true,
    get() {
      return Number.parseFloat(this.style.height) || 0;
    },
  });
  return { nv, img };
}

function mouse(target: Document | Element, type: string, clientX: number): void {
  (target as EventTarget).dispatchEvent(new MouseEvent(type, { bubbles: true, clientX, clientY: 0 }));
}

/** Drag a corner handle from x=100 by `dx`, returning the live img size. */
function drag(nv: ImageNodeView, img: HTMLImageElement, corner: string, dx: number): { w: number; h: number } {
  const handle = nv.dom.querySelector<HTMLElement>(`.pm-image-resize-${corner}`);
  expect(handle).not.toBeNull();
  mouse(handle as Element, "mousedown", 100);
  mouse(document, "mousemove", 100 + dx);
  const size = { w: img.offsetWidth, h: img.offsetHeight };
  mouse(document, "mouseup", 100 + dx);
  return size;
}

const views: EditorView[] = [];
const nodeViews: ImageNodeView[] = [];

function setup(): { view: EditorView; nv: ImageNodeView; img: HTMLImageElement } {
  const { view, imageNode } = createViewWithImage();
  views.push(view);
  const { nv, img } = createNodeView(view, imageNode);
  nodeViews.push(nv);
  return { view, nv, img };
}

afterEach(() => {
  for (const nv of nodeViews) {
    nv.destroy();
    nv.dom.remove();
  }
  nodeViews.length = 0;
  for (const view of views) {
    view.destroy();
  }
  views.length = 0;
  document.body.innerHTML = "";
});

// ── East handles: unchanged behaviour ──────────────────────────────

describe("east resize handles (ne/se)", () => {
  it("se: dragging right widens, maintaining aspect ratio", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "se", 40)).toEqual({ w: 240, h: 120 });
  });

  it("se: dragging left shrinks", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "se", -40)).toEqual({ w: 160, h: 80 });
  });

  it("ne: dragging right widens", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "ne", 40)).toEqual({ w: 240, h: 120 });
  });
});

// ── West handles: inverted direction (#1389) ───────────────────────

describe("west resize handles (nw/sw)", () => {
  it("nw: dragging outward (left) widens, maintaining aspect ratio", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "nw", -40)).toEqual({ w: 240, h: 120 });
  });

  it("nw: dragging inward (right) shrinks", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "nw", 40)).toEqual({ w: 160, h: 80 });
  });

  it("sw: dragging outward (left) widens", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "sw", -40)).toEqual({ w: 240, h: 120 });
  });

  it("sw: dragging inward (right) shrinks", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "sw", 40)).toEqual({ w: 160, h: 80 });
  });
});

// ── Clamps apply after the sign fix ────────────────────────────────

describe("resize clamps", () => {
  it("min-width 50px applies to a west drag inward", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "nw", 1000)).toEqual({ w: 50, h: 25 });
  });

  it("editor max-width applies to a west drag outward", () => {
    const { nv, img } = setup();
    // Max width is editor width - 40 = 960; height follows the 2:1 ratio.
    expect(drag(nv, img, "sw", -5000)).toEqual({ w: 960, h: 480 });
  });

  it("editor max-width still applies to an east drag", () => {
    const { nv, img } = setup();
    expect(drag(nv, img, "se", 5000)).toEqual({ w: 960, h: 480 });
  });
});

// ── Committed attrs on mouse-up ────────────────────────────────────

describe("committed attrs on mouse-up", () => {
  it("nw outward drag commits the GROWN width/height to the node attrs", () => {
    const { view, nv, img } = setup();
    drag(nv, img, "nw", -40);
    const committed = view.state.doc.firstChild?.firstChild;
    expect(committed?.attrs.width).toBe(240);
    expect(committed?.attrs.height).toBe(120);
  });

  it("se drag commits its width/height to the node attrs", () => {
    const { view, nv, img } = setup();
    drag(nv, img, "se", 40);
    const committed = view.state.doc.firstChild?.firstChild;
    expect(committed?.attrs.width).toBe(240);
    expect(committed?.attrs.height).toBe(120);
  });
});
