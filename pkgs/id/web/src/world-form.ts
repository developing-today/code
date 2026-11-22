/**
 * Safe browser presenter for a presented world form (#8938).
 *
 * The wire document is the versioned `FormSurface` from
 * `pkgs/id/crates/id-core/src/world_presentation.rs`. Guest input is data
 * only: there are no HTML, CSS, or executable-validation fields, so every
 * widget built here comes from this client's own input manifest plus the
 * closed field/action model — never from the presenting program.
 *
 * The reference presenter is the SSH terminal
 * (`pkgs/id/crates/id-worlds/src/world_ssh.rs`): it renders only when the
 * form carries a presentation token AND this presenter's manifest can
 * collect every field, sends `form_result` with the view's `form_id` and
 * `presentation_token`, and re-presents with the host-issued token when a
 * `form_result_status` rejection arrives.
 */

export type FormField =
  | { kind: "text"; id: string; label: string; required: boolean; max_length: number }
  | { kind: "choice"; id: string; label: string; required: boolean; options: string[] }
  | { kind: "number"; id: string; label: string; required: boolean; min: number; max: number }
  | { kind: "date"; id: string; label: string; required: boolean; min?: string | null; max?: string | null }
  | { kind: "confirm"; id: string; label: string; required: boolean };

export type FormActionKind = "submit" | "cancel";

export type FormAction = { id: string; label: string; kind?: FormActionKind };

export type FormSurface = {
  kind?: "form";
  version: number;
  form_id: string;
  presentation_token?: string | null;
  title: string;
  fields: FormField[];
  actions: FormAction[];
};

/** The `form_result` client frame the contract names (shell-contract.v1). */
export type FormResultFrame = {
  type: "form_result";
  form_id: string;
  presentation_token: string;
  action: string;
  values: Record<string, string | number | boolean>;
};

/** This presenter's own input affordances (`DeviceManifest.input`). */
export const BROWSER_FORM_INPUT_MANIFEST = {
  keys: 128,
  encoder: false,
  pointer: true,
  text_entry: "keyboard",
} as const;

export type FormInputManifest = {
  keys?: number;
  encoder?: boolean;
  pointer?: boolean;
  text_entry?: string;
};

export const MAX_FORM_ITEMS = 64;
export const MAX_FORM_BYTES = 8 * 1024;
/** `MAX_EVENT_BYTES`: one form answer is one capability event. */
export const MAX_FORM_RESPONSE_BYTES = 1024 * 1024;

function safeFormText(value: string): boolean {
  // Mirrors Rust `safe_form_text`: non-blank and free of control characters.
  if (typeof value !== "string" || value.trim() === "") return false;
  for (let index = 0; index < value.length; index++) {
    const code = value.charCodeAt(index);
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) return false;
  }
  return true;
}

function safeFormId(value: string): boolean {
  return typeof value === "string" && value.length > 0 && value.length <= 64 && /^[A-Za-z0-9_-]+$/.test(value);
}

function safeFormToken(value: string): boolean {
  return typeof value === "string" && value.length === 32 && /^[0-9a-fA-F]{32}$/.test(value);
}

const FORM_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;

function validFormDate(value: string): boolean {
  const match = FORM_DATE.exec(value);
  if (!match) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12 || day < 1) return false;
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
  return day <= (days ?? 0);
}

/**
 * Validate the closed field/action model, mirroring Rust
 * `FormSurface::validate`. Returns a defect string or null when valid.
 */
export function validateFormSurface(form: unknown): string | null {
  if (!form || typeof form !== "object") return "form must be an object";
  const surface = form as FormSurface;
  if (surface.version !== 1) return "unsupported world form version";
  if (!safeFormId(surface.form_id) || !safeFormText(surface.title)) return "invalid form id or title";
  if (!Array.isArray(surface.fields) || surface.fields.length === 0 || surface.fields.length > MAX_FORM_ITEMS)
    return "invalid form field count";
  if (!Array.isArray(surface.actions) || surface.actions.length === 0 || surface.actions.length > MAX_FORM_ITEMS)
    return "invalid form action count";

  const ids = new Set<string>();
  for (const field of surface.fields) {
    if (!field || typeof field !== "object") return "invalid form field";
    const id = (field as { id?: unknown }).id;
    const label = (field as { label?: unknown }).label;
    if (typeof id !== "string" || typeof label !== "string") return "invalid form field";
    if (!safeFormId(id) || !safeFormText(label)) return "invalid form field id or label";
    if (ids.has(id)) return "duplicate form id";
    ids.add(id);
    switch (field.kind) {
      case "text": {
        if (!Number.isInteger(field.max_length) || (field.max_length as number) <= 0)
          return "text field max_length must be positive";
        break;
      }
      case "choice": {
        const options = field.options;
        if (!Array.isArray(options) || options.length === 0 || options.length > MAX_FORM_ITEMS)
          return "invalid choice option count";
        if (!options.every((option) => typeof option === "string" && safeFormText(option)))
          return "invalid choice option";
        break;
      }
      case "number": {
        if (!Number.isFinite(field.min) || !Number.isFinite(field.max)) return "invalid number bounds";
        if (field.min > field.max) return "number field min exceeds max";
        break;
      }
      case "date": {
        const min = field.min ?? null;
        const max = field.max ?? null;
        if (
          (min !== null && (typeof min !== "string" || !validFormDate(min))) ||
          (max !== null && (typeof max !== "string" || !validFormDate(max)))
        )
          return "invalid date bound";
        if (typeof min === "string" && typeof max === "string" && min > max) return "date field min exceeds max";
        break;
      }
      case "confirm":
        break;
      default:
        return "unsupported form field kind";
    }
  }
  for (const action of surface.actions) {
    if (!action || typeof action !== "object") return "invalid form action";
    if (!safeFormId(action.id) || !safeFormText(action.label)) return "invalid form action id or label";
    if (ids.has(action.id)) return "duplicate form id";
    ids.add(action.id);
    const kind = action.kind ?? "submit";
    if (kind !== "submit" && kind !== "cancel") return "unsupported form action kind";
  }
  const token = surface.presentation_token ?? null;
  if (token !== null && (typeof token !== "string" || !safeFormToken(token))) return "invalid form presentation token";
  let bytes = 0;
  try {
    bytes = new TextEncoder().encode(JSON.stringify(form)).length;
  } catch {
    return "form is not serializable";
  }
  if (bytes > MAX_FORM_BYTES) return `form exceeds ${MAX_FORM_BYTES} bytes`;
  return null;
}

/**
 * Check that THIS presenter's manifest can collect every field, mirroring
 * Rust `FormSurface::validate_device_input`. The hosting server's manifest
 * is irrelevant here: the browser has its own affordances.
 */
export function validateFormForPresenter(form: FormSurface, manifest: FormInputManifest): string | null {
  const defect = validateFormSurface(form);
  if (defect) return defect;
  const canEnterText = typeof manifest.text_entry === "string" && manifest.text_entry !== "none";
  const hasDiscreteControl = (manifest.keys ?? 0) > 0 || manifest.encoder === true || manifest.pointer === true;
  for (const field of form.fields) {
    if ((field.kind === "text" || field.kind === "date") && !canEnterText)
      return `text entry unavailable for form field ${field.id}`;
    if ((field.kind === "choice" || field.kind === "confirm") && !hasDiscreteControl)
      return `selectable input unavailable for form field ${field.id}`;
    if (field.kind === "number" && !canEnterText && !hasDiscreteControl)
      return `numeric input unavailable for form field ${field.id}`;
  }
  return null;
}

/** Build the `form_result` frame for one action, or null when the form cannot answer. */
export function formResultFrame(
  form: FormSurface,
  actionId: string,
  values: Record<string, string | number | boolean>,
): FormResultFrame | null {
  const token = form.presentation_token;
  if (!token || !safeFormToken(token)) return null;
  const action = form.actions.find((candidate) => candidate.id === actionId);
  if (!action) return null;
  const frame: FormResultFrame = {
    type: "form_result",
    form_id: form.form_id,
    presentation_token: token,
    action: action.id,
    // A cancel abandons the form: the reference presenter sends no values.
    values: (action.kind ?? "submit") === "cancel" ? {} : values,
  };
  if (JSON.stringify(frame).length > MAX_FORM_RESPONSE_BYTES) return null;
  return frame;
}

/** Terminal projection for a form this presenter cannot safely render. */
export function formTextProjection(form: FormSurface): string {
  const lines = [String(form.title ?? "")];
  const fields = Array.isArray(form.fields) ? form.fields : [];
  fields.forEach((field, index) => {
    const detail =
      field.kind === "text"
        ? `text, max ${field.max_length}`
        : field.kind === "choice"
          ? `choice: ${(field.options ?? []).join(", ")}`
          : field.kind === "number"
            ? `number: ${field.min}..${field.max}`
            : field.kind === "date"
              ? `date${field.min ? ` from ${field.min}` : ""}${field.max ? ` to ${field.max}` : ""}`
              : "yes/no";
    lines.push(`${index + 1}. ${String(field.label)} — ${detail}${field.required ? " (required)" : ""}`);
  });
  const actions = Array.isArray(form.actions) ? form.actions : [];
  actions.forEach((action, index) => {
    lines.push(`- ${String(action.label)}${(action.kind ?? "submit") === "cancel" ? " (cancel)" : ""}`);
    void index;
  });
  return lines.join("\n");
}

export type FormPresenterOptions = {
  /** This presenter's input affordances; defaults to the browser manifest. */
  manifest?: FormInputManifest;
  /** Deliver one validated frame; returns false when the socket is not open. */
  send: (frame: FormResultFrame) => boolean;
};

export type FormPresenter = {
  /** The form currently rendered, or null when none is trusted. */
  active: FormSurface | null;
  /** Render a presented surface; returns true when actionable widgets were built. */
  present: (surface: unknown) => boolean;
  /** Apply a `form_result_status` reply; returns the frame to re-present, if any. */
  settle: (status: unknown) => { accepted: boolean; rejected: boolean; closed: boolean };
  /** The presentation token currently in force for the rendered form. */
  token: () => string | null;
};

type NormalizedStatus = {
  form_id: string;
  ok: boolean;
  presentation_token: string | null;
  error: string;
  reason: string;
};

function parseStatus(status: unknown): NormalizedStatus | null {
  if (!status || typeof status !== "object") return null;
  const value = status as Record<string, unknown>;
  if (typeof value.form_id !== "string") return null;
  const token =
    typeof value.presentation_token === "string" && safeFormToken(value.presentation_token)
      ? value.presentation_token
      : null;
  return {
    form_id: value.form_id,
    ok: value.ok === true,
    presentation_token: token,
    error: typeof value.error === "string" ? value.error : "form_unavailable",
    reason: typeof value.reason === "string" ? value.reason : "unknown",
  };
}

/**
 * Build a presenter bound to one container. Widgets are constructed with
 * `createElement` and `textContent` only — no markup is ever parsed.
 */
export function createFormPresenter(root: HTMLElement, options: FormPresenterOptions): FormPresenter {
  const manifest = options.manifest ?? BROWSER_FORM_INPUT_MANIFEST;
  let active: FormSurface | null = null;
  /** The token the guest's view renders, paired with the host's current token. */
  let retryAlias: { formId: string; viewToken: string; currentToken: string } | null = null;

  const clear = () => {
    root.replaceChildren();
  };

  const presenter: FormPresenter = {
    get active() {
      return active;
    },
    present(surface: unknown) {
      if (surface === null || surface === undefined) {
        // A non-form view replaced the form; clear only if we were rendering
        // one, so an ordinary screen/text redraw is left untouched.
        if (active === null) return false;
        active = null;
        retryAlias = null;
        clear();
        return false;
      }
      if (validateFormForPresenter(surface as FormSurface, manifest) !== null) {
        // Untrusted manifest or invalid document: render no actionable
        // widget, only the escape-free text projection.
        active = null;
        retryAlias = null;
        clear();
        if (surface && typeof surface === "object") {
          const fallback = document.createElement("pre");
          fallback.className = "id-world-form-fallback";
          fallback.textContent = formTextProjection(surface as FormSurface);
          root.append(fallback);
        }
        return false;
      }
      const form = surface as FormSurface;
      // A redraw may still carry the program's pre-rejection token while a
      // retry token is in force; keep the host-issued token active.
      if (retryAlias && retryAlias.formId === form.form_id) {
        if (form.presentation_token === retryAlias.viewToken) {
          form.presentation_token = retryAlias.currentToken;
        } else {
          retryAlias = null;
        }
      }
      if (active && active.form_id === form.form_id && active.presentation_token === form.presentation_token) {
        return true;
      }
      active = form;
      render();
      return true;
    },
    settle(status: unknown) {
      const parsed = parseStatus(status);
      const accepted = { accepted: false, rejected: false, closed: false };
      if (!parsed || !active || active.form_id !== parsed.form_id) {
        return { ...accepted, closed: true };
      }
      if (parsed.ok) {
        active = null;
        retryAlias = null;
        clear();
        return { accepted: true, rejected: false, closed: true };
      }
      if (parsed.presentation_token) {
        const previous = active.presentation_token ?? null;
        if (previous !== null) {
          const viewToken =
            retryAlias && retryAlias.formId === parsed.form_id && retryAlias.currentToken === previous
              ? retryAlias.viewToken
              : previous;
          retryAlias = { formId: parsed.form_id, viewToken, currentToken: parsed.presentation_token };
        }
        active = { ...active, presentation_token: parsed.presentation_token };
        render();
        return { accepted: false, rejected: true, closed: false };
      }
      active = null;
      retryAlias = null;
      clear();
      return { accepted: false, rejected: false, closed: true };
    },
    token() {
      return active?.presentation_token ?? null;
    },
  };

  function collect(form: FormSurface): Record<string, string | number | boolean> {
    const values: Record<string, string | number | boolean> = {};
    for (const field of form.fields) {
      const input = root.querySelector<HTMLInputElement | HTMLSelectElement>(`[data-world-form-field="${field.id}"]`);
      if (!input) continue;
      const raw = input.value;
      if (raw === "") continue;
      if (field.kind === "number") {
        const parsed = Number(raw);
        if (Number.isFinite(parsed)) values[field.id] = parsed;
      } else if (field.kind === "confirm") {
        values[field.id] = raw === "true";
      } else {
        values[field.id] = raw;
      }
    }
    return values;
  }

  function render(): void {
    const form = active;
    if (!form) return;
    const container = document.createElement("form");
    container.className = "id-world-form";
    container.setAttribute("data-world-form", form.form_id);
    container.addEventListener("submit", (event) => event.preventDefault());

    const heading = document.createElement("h3");
    heading.textContent = form.title;
    container.append(heading);

    for (const field of form.fields) {
      const row = document.createElement("label");
      row.className = "id-world-form-field";
      const caption = document.createElement("span");
      caption.textContent = `${field.label}${field.required ? " *" : ""}`;
      row.append(caption);
      let control: HTMLInputElement | HTMLSelectElement;
      if (field.kind === "choice") {
        const select = document.createElement("select");
        // The empty option keeps an optional choice unanswered.
        const blank = document.createElement("option");
        blank.value = "";
        blank.textContent = "—";
        select.append(blank);
        for (const option of field.options) {
          const element = document.createElement("option");
          element.value = option;
          element.textContent = option;
          select.append(element);
        }
        control = select;
      } else if (field.kind === "confirm") {
        const select = document.createElement("select");
        for (const [value, text] of [
          ["", "—"],
          ["true", "Yes"],
          ["false", "No"],
        ] as const) {
          const element = document.createElement("option");
          element.value = value;
          element.textContent = text;
          select.append(element);
        }
        control = select;
      } else {
        const input = document.createElement("input");
        if (field.kind === "text") {
          input.type = "text";
          input.maxLength = field.max_length;
        } else if (field.kind === "number") {
          input.type = "number";
          input.min = String(field.min);
          input.max = String(field.max);
          input.step = "1";
        } else {
          input.type = "date";
          if (field.min) input.min = field.min;
          if (field.max) input.max = field.max;
        }
        input.required = field.required;
        control = input;
      }
      control.setAttribute("data-world-form-field", field.id);
      row.append(control);
      container.append(row);
    }

    const actions = document.createElement("div");
    actions.className = "id-world-form-actions";
    for (const action of form.actions) {
      const kind = action.kind ?? "submit";
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = action.label;
      button.setAttribute("data-world-form-action", action.id);
      if (kind === "cancel") button.className = "id-world-form-cancel";
      button.addEventListener("click", () => {
        const frame = formResultFrame(form, action.id, collect(form));
        if (frame) options.send(frame);
      });
      actions.append(button);
    }
    container.append(actions);
    root.replaceChildren(container);
  }

  return presenter;
}
