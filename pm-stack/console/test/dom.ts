// A page, as far as the screens' builders touch one: elements that
// remember their attributes, classes, children and listeners, and can
// be handed an event. Enough to run what draws a screen and read back
// what it drew; what a browser does with it is looked at in a browser.

export class El {
  attrs: Record<string, string> = {};
  kids: (El | string)[] = [];
  listeners: Record<string, ((event: Record<string, unknown>) => void)[]> = {};
  className = "";
  nodeType = 1;
  box = { left: 0, width: 360 };
  constructor(public tag: string) {}
  setAttribute(name: string, value: string) {
    if (name === "class") this.className = String(value);
    else this.attrs[name] = String(value);
  }
  getAttribute(name: string) {
    return name === "class" ? this.className : (this.attrs[name] ?? null);
  }
  addEventListener(name: string, fn: (event: Record<string, unknown>) => void) {
    this.listeners[name] = [...(this.listeners[name] ?? []), fn];
  }
  append(...kids: (El | string)[]) {
    this.kids.push(...kids);
  }
  replaceChildren(...kids: (El | string)[]) {
    this.kids = kids;
  }
  get classList() {
    const el = this;
    const list = () => el.className.split(/\s+/).filter(Boolean);
    return {
      contains: (name: string) => list().includes(name),
      toggle: (name: string, on: boolean) => {
        const next = list().filter((c) => c !== name);
        if (on) next.push(name);
        el.className = next.join(" ");
      },
    };
  }
  get textContent(): string {
    return this.kids.map((kid) => (typeof kid === "string" ? kid : kid.textContent)).join("");
  }
  getBoundingClientRect() {
    return this.box;
  }
  /** Hand it an event, as the browser would. */
  fire(name: string, event: Record<string, unknown> = {}) {
    let prevented = false;
    for (const fn of this.listeners[name] ?? []) fn({ preventDefault: () => (prevented = true), ...event });
    return prevented;
  }
  /** Every element under this one (and this one) that passes. */
  all(pass: (el: El) => boolean): El[] {
    const out: El[] = pass(this) ? [this] : [];
    for (const kid of this.kids) if (typeof kid !== "string") out.push(...kid.all(pass));
    return out;
  }
  /** By tag, or by ".class". */
  find(what: string): El[] {
    return what.startsWith(".")
      ? this.all((el) => el.classList.contains(what.slice(1)))
      : this.all((el) => el.tag === what);
  }
}

/** Put a page in place for the builders to draw on. It has nothing on
 * it to be found: whatever looks for the app's own frame finds none,
 * as when there is no page at all. `away()` takes it out again. */
export function page() {
  const made = {
    createElement: (tag: string) => new El(tag),
    createElementNS: (_: string, tag: string) => new El(tag),
    createTextNode: (text: string) => text,
    getElementById: () => null,
    querySelector: () => null,
  };
  (globalThis as unknown as { document: unknown }).document = made;
  return made;
}

export function away() {
  delete (globalThis as unknown as { document?: unknown }).document;
}
