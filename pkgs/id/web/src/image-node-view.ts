/**
 * Custom ProseMirror NodeView for images.
 *
 * Features:
 * - Click image to select and show toolbar popover
 * - Edit alt-text inline via popover input
 * - Drag corner handles to resize (maintains aspect ratio)
 * - Width/height persisted as node attributes
 */

import type { Node } from "prosemirror-model";
import type { EditorView, NodeView } from "prosemirror-view";

// ============================================================================
// ImageNodeView
// ============================================================================

export class ImageNodeView implements NodeView {
  dom: HTMLElement;
  private img: HTMLImageElement;
  private popover: HTMLElement | null = null;
  private node: Node;
  private view: EditorView;
  private getPos: () => number | undefined;

  // Resize state
  private resizing = false;
  private startX = 0;
  private startY = 0;
  private startWidth = 0;
  private startHeight = 0;
  private aspectRatio = 1;

  // Bound handlers for cleanup
  private boundMouseMove: ((e: MouseEvent) => void) | null = null;
  private boundMouseUp: ((e: MouseEvent) => void) | null = null;
  private boundDocClick: ((e: MouseEvent) => void) | null = null;

  constructor(node: Node, view: EditorView, getPos: () => number | undefined) {
    this.node = node;
    this.view = view;
    this.getPos = getPos;

    // Build DOM: figure > img + resize handles
    const figure = document.createElement("figure");
    figure.className = "pm-image-view";
    figure.contentEditable = "false";

    const img = document.createElement("img");
    img.src = node.attrs.src || "";
    if (node.attrs.alt) img.alt = node.attrs.alt;
    if (node.attrs.title) img.title = node.attrs.title;
    if (node.attrs.width) {
      img.style.width = `${node.attrs.width}px`;
    }
    if (node.attrs.height) {
      img.style.height = `${node.attrs.height}px`;
    }
    img.className = "pm-image";
    img.draggable = false;

    // Click to show popover
    figure.addEventListener("click", (e) => {
      e.stopPropagation();
      this.showPopover();
    });

    // Resize handles (4 corners)
    for (const corner of ["nw", "ne", "sw", "se"] as const) {
      const handle = document.createElement("span");
      handle.className = `pm-image-resize-handle pm-image-resize-${corner}`;
      handle.contentEditable = "false";
      handle.addEventListener("mousedown", (e) => {
        e.preventDefault();
        e.stopPropagation();
        this.startResize(e, corner);
      });
      figure.appendChild(handle);
    }

    figure.appendChild(img);

    this.img = img;
    this.dom = figure;

    // Document click to dismiss popover
    this.boundDocClick = (e: MouseEvent) => {
      if (
        this.popover &&
        !this.popover.contains(e.target as HTMLElement) &&
        !this.dom.contains(e.target as HTMLElement)
      ) {
        this.hidePopover();
      }
    };
    document.addEventListener("click", this.boundDocClick);
  }

  // ── Popover for alt-text editing ──

  private showPopover(): void {
    if (this.popover) return;

    const popover = document.createElement("div");
    popover.className = "pm-image-popover";

    const label = document.createElement("label");
    label.textContent = "Alt text:";
    label.className = "pm-image-popover-label";

    const input = document.createElement("input");
    input.type = "text";
    input.className = "pm-image-popover-input";
    input.value = this.node.attrs.alt || "";
    input.placeholder = "Describe this image…";

    // Commit on Enter or blur
    const commit = () => {
      const newAlt = input.value;
      if (newAlt !== (this.node.attrs.alt || "")) {
        this.updateAttrs({ alt: newAlt });
      }
      this.hidePopover();
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        commit();
      }
      if (e.key === "Escape") {
        e.preventDefault();
        this.hidePopover();
      }
    });
    input.addEventListener("blur", commit);

    // Show width/height info
    const sizeInfo = document.createElement("span");
    sizeInfo.className = "pm-image-popover-size";
    const w = this.node.attrs.width || this.img.naturalWidth || "auto";
    const h = this.node.attrs.height || this.img.naturalHeight || "auto";
    sizeInfo.textContent = `${w} × ${h}`;

    popover.appendChild(label);
    popover.appendChild(input);
    popover.appendChild(sizeInfo);

    this.dom.appendChild(popover);
    this.popover = popover;

    // Focus the input
    requestAnimationFrame(() => input.focus());
  }

  private hidePopover(): void {
    if (this.popover) {
      this.popover.remove();
      this.popover = null;
    }
  }

  // ── Resize ──

  private startResize(e: MouseEvent, _corner: string): void {
    this.resizing = true;
    this.startX = e.clientX;
    this.startY = e.clientY;
    this.startWidth = this.img.offsetWidth || this.img.naturalWidth || 200;
    this.startHeight = this.img.offsetHeight || this.img.naturalHeight || 200;
    this.aspectRatio = this.startWidth / this.startHeight;

    this.dom.classList.add("pm-image-resizing");

    this.boundMouseMove = (ev: MouseEvent) => this.onResizeMove(ev);
    this.boundMouseUp = (_ev: MouseEvent) => this.onResizeEnd();
    document.addEventListener("mousemove", this.boundMouseMove);
    document.addEventListener("mouseup", this.boundMouseUp);
  }

  private onResizeMove(e: MouseEvent): void {
    if (!this.resizing) return;

    const dx = e.clientX - this.startX;
    // Use horizontal delta to maintain aspect ratio
    let newWidth = Math.max(50, this.startWidth + dx);
    let newHeight = Math.round(newWidth / this.aspectRatio);

    // Clamp to reasonable bounds
    const maxWidth = this.view.dom.offsetWidth - 40;
    if (newWidth > maxWidth) {
      newWidth = maxWidth;
      newHeight = Math.round(newWidth / this.aspectRatio);
    }

    this.img.style.width = `${newWidth}px`;
    this.img.style.height = `${newHeight}px`;
  }

  private onResizeEnd(): void {
    if (!this.resizing) return;
    this.resizing = false;
    this.dom.classList.remove("pm-image-resizing");

    if (this.boundMouseMove) {
      document.removeEventListener("mousemove", this.boundMouseMove);
      this.boundMouseMove = null;
    }
    if (this.boundMouseUp) {
      document.removeEventListener("mouseup", this.boundMouseUp);
      this.boundMouseUp = null;
    }

    // Commit final dimensions
    const newWidth = Math.round(this.img.offsetWidth);
    const newHeight = Math.round(this.img.offsetHeight);
    if (newWidth > 0 && newHeight > 0) {
      this.updateAttrs({ width: newWidth, height: newHeight });
    }
  }

  // ── Attr update helper ──

  private updateAttrs(attrs: Record<string, unknown>): void {
    const pos = this.getPos();
    if (pos == null) return;
    const tr = this.view.state.tr.setNodeMarkup(pos, undefined, {
      ...this.node.attrs,
      ...attrs,
    });
    this.view.dispatch(tr);
  }

  // ── NodeView interface ──

  update(node: Node): boolean {
    if (node.type !== this.node.type) return false;
    this.node = node;

    // Sync DOM
    if (this.img.src !== node.attrs.src) this.img.src = node.attrs.src || "";
    if (node.attrs.alt != null) this.img.alt = node.attrs.alt;
    if (node.attrs.title != null) this.img.title = node.attrs.title;

    if (node.attrs.width) {
      this.img.style.width = `${node.attrs.width}px`;
    } else {
      this.img.style.width = "";
    }
    if (node.attrs.height) {
      this.img.style.height = `${node.attrs.height}px`;
    } else {
      this.img.style.height = "";
    }

    return true;
  }

  selectNode(): void {
    this.dom.classList.add("ProseMirror-selectednode");
  }

  deselectNode(): void {
    this.dom.classList.remove("ProseMirror-selectednode");
    this.hidePopover();
  }

  stopEvent(event: Event): boolean {
    // Let the nodeView handle click and mouse events on handles
    if (event.type === "mousedown" || event.type === "click") return true;
    return false;
  }

  ignoreMutation(): boolean {
    return true;
  }

  destroy(): void {
    this.hidePopover();
    if (this.boundDocClick) {
      document.removeEventListener("click", this.boundDocClick);
    }
    if (this.boundMouseMove) {
      document.removeEventListener("mousemove", this.boundMouseMove);
    }
    if (this.boundMouseUp) {
      document.removeEventListener("mouseup", this.boundMouseUp);
    }
  }
}
