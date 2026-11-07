// PROBE (not a repo test): image upload completing after its EditorView is destroyed.
// Mirrors main.ts closeEditor(): collab disconnect + view.destroy() when the user
// closes/opens another document while an upload is in flight.
import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { describe, expect, it, vi } from "vitest";
import { richSchema } from "./editor";
import { createImageUploadPlugin } from "./image-upload";

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

describe("probe: upload completes after view.destroy()", () => {
  it("observes success-path and failure-path behaviour", async () => {
    const warns: unknown[][] = [];
    vi.spyOn(console, "warn").mockImplementation((...a: unknown[]) => { warns.push(a); });
    const unhandled: unknown[] = [];
    const onUnhandled = (e: PromiseRejectionEvent) => { unhandled.push(e.reason); };
    window.addEventListener("unhandledrejection", onUnhandled);

    const plugin = createImageUploadPlugin(richSchema);
    if (!plugin) throw new Error("plugin");
    const handlePaste = plugin.props.handlePaste;
    if (!handlePaste) throw new Error("handlePaste");

    const container = document.createElement("div");
    document.body.append(container);
    const doc = richSchema.nodes.doc.create(null, [
      richSchema.nodes.paragraph.create(null, richSchema.text("hello world")),
    ]);
    const view = new EditorView(container, {
      state: EditorState.create({ doc, plugins: [plugin] }),
    });

    const gate = deferred<Response>();
    vi.spyOn(globalThis, "fetch").mockImplementation(() => gate.promise);

    const file = new File(["pngdata"], "a.png", { type: "image/png" });
    const event = { clipboardData: { files: [file] }, preventDefault: vi.fn() };
    const claimed = handlePaste.call(plugin, view, event as unknown as ClipboardEvent, undefined as never);
    console.log("PROBE claimed:", claimed);

    view.destroy();

    gate.resolve({ ok: true, status: 200, json: () => Promise.resolve({ hash: "h", name: "a.png", url: "/files/a.png" }), text: () => Promise.resolve("") } as unknown as Response);
    await new Promise((r) => setTimeout(r, 50));

    console.log("PROBE warns after success-after-destroy:", JSON.stringify(warns.map((w) => w.map(String))));
    console.log("PROBE old view doc after completion:", view.state.doc.toString());
    console.log("PROBE unhandled rejections:", JSON.stringify(unhandled.map(String)));

    warns.length = 0; unhandled.length = 0;
    const view2 = new EditorView(container, { state: EditorState.create({ doc, plugins: [plugin] }) });
    const gate2 = deferred<Response>();
    vi.spyOn(globalThis, "fetch").mockImplementation(() => gate2.promise);
    handlePaste.call(plugin, view2, event as unknown as ClipboardEvent, undefined as never);
    view2.destroy();
    gate2.reject(new Error("network down"));
    await new Promise((r) => setTimeout(r, 50));
    console.log("PROBE warns after failure-after-destroy:", JSON.stringify(warns.map((w) => w.map(String))));
    console.log("PROBE unhandled rejections (failure):", JSON.stringify(unhandled.map(String)));
    window.removeEventListener("unhandledrejection", onUnhandled);
    expect(true).toBe(true);
  });
});
