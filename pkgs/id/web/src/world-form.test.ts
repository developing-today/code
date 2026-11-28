import { describe, expect, it, vi } from "vitest";
import {
  BROWSER_FORM_INPUT_MANIFEST,
  createFormPresenter,
  type FormResultFrame,
  type FormSurface,
  formResultFrame,
  formTextProjection,
  validateFormForPresenter,
  validateFormSurface,
} from "./world-form";

const TOKEN = "0123456789abcdef0123456789abcdef";
const NEXT_TOKEN = "fedcba9876543210fedcba9876543210";

function signupForm(overrides: Partial<FormSurface> = {}): FormSurface {
  return {
    version: 1,
    form_id: "signup",
    presentation_token: TOKEN,
    title: "Sign up",
    fields: [
      { kind: "text", id: "name", label: "Name", required: true, max_length: 40 },
      { kind: "choice", id: "plan", label: "Plan", required: false, options: ["free", "pro"] },
      { kind: "confirm", id: "terms", label: "Accept terms", required: true },
    ],
    actions: [
      { id: "submit", label: "Send" },
      { id: "cancel", label: "Cancel", kind: "cancel" },
    ],
    ...overrides,
  };
}

describe("world form validation", () => {
  it("accepts the reference form shape", () => {
    expect(validateFormSurface(signupForm())).toBeNull();
  });

  it("rejects an unknown version, a bad id, and a control character in the title", () => {
    expect(validateFormSurface(signupForm({ version: 2 }))).toBe("unsupported world form version");
    expect(validateFormSurface(signupForm({ form_id: "bad id!" }))).toBe("invalid form id or title");
    expect(validateFormSurface(signupForm({ title: "a\u0007b" }))).toBe("invalid form id or title");
  });

  it("rejects duplicate ids across fields and actions", () => {
    const duplicate = signupForm({
      fields: [
        { kind: "text", id: "same", label: "A", required: false, max_length: 4 },
        { kind: "text", id: "same", label: "B", required: false, max_length: 4 },
      ],
    });
    expect(validateFormSurface(duplicate)).toBe("duplicate form id");
  });

  it("rejects a malformed presentation token", () => {
    expect(validateFormSurface(signupForm({ presentation_token: "short" }))).toBe("invalid form presentation token");
  });

  it("rejects an inverted number range and a bad date bound", () => {
    expect(
      validateFormSurface(
        signupForm({
          fields: [{ kind: "number", id: "n", label: "N", required: false, min: 10, max: 1 }],
        }),
      ),
    ).toBe("number field min exceeds max");
    expect(
      validateFormSurface(
        signupForm({
          fields: [{ kind: "date", id: "d", label: "D", required: false, min: "2024-02-30" }],
        }),
      ),
    ).toBe("invalid date bound");
  });
});

describe("presenter manifest gating", () => {
  it("passes with the browser manifest, which has text entry and pointer input", () => {
    expect(validateFormForPresenter(signupForm(), BROWSER_FORM_INPUT_MANIFEST)).toBeNull();
  });

  it("refuses a text field when the presenter has no text entry", () => {
    expect(validateFormForPresenter(signupForm(), { keys: 8, text_entry: "none" })).toMatch(
      /text entry unavailable for form field name/,
    );
  });

  it("refuses a selectable field when the presenter has no discrete control", () => {
    expect(validateFormForPresenter(signupForm(), { keys: 0, text_entry: "keyboard" })).toMatch(
      /selectable input unavailable for form field plan/,
    );
  });
});

describe("form result frames", () => {
  it("carries the view's form_id and presentation_token on a submit", () => {
    const frame = formResultFrame(signupForm(), "submit", { name: "Ada", plan: "pro", terms: true });
    expect(frame).toEqual({
      type: "form_result",
      form_id: "signup",
      presentation_token: TOKEN,
      action: "submit",
      values: { name: "Ada", plan: "pro", terms: true },
    });
  });

  it("sends no values on cancel, mirroring the SSH presenter", () => {
    const frame = formResultFrame(signupForm(), "cancel", { name: "ignored" });
    expect(frame?.action).toBe("cancel");
    expect(frame?.values).toEqual({});
  });

  it("refuses to build a frame for an unknown action or a tokenless form", () => {
    expect(formResultFrame(signupForm(), "nope", {})).toBeNull();
    expect(formResultFrame(signupForm({ presentation_token: null }), "submit", {})).toBeNull();
  });
});

describe("form presenter", () => {
  function mounted() {
    const root = document.createElement("div");
    document.body.append(root);
    const sent: FormResultFrame[] = [];
    const presenter = createFormPresenter(root, {
      send: (frame) => {
        sent.push(frame);
        return true;
      },
    });
    return { root, sent, presenter };
  }

  it("renders one action control per form action, and no markup from the program", () => {
    const { root, presenter } = mounted();
    expect(presenter.present(signupForm())).toBe(true);
    const buttons = [...root.querySelectorAll("[data-world-form-action]")].map((node) => node.textContent);
    expect(buttons).toEqual(["Send", "Cancel"]);
    const input = root.querySelector<HTMLInputElement>('[data-world-form-field="name"]');
    expect(input?.type).toBe("text");
    // The title is textContent, never parsed markup.
    expect(root.querySelector("h3")?.textContent).toBe("Sign up");
    expect(root.innerHTML).not.toContain("<script");
  });

  it("submits collected field values in a form_result frame", () => {
    const { root, sent, presenter } = mounted();
    presenter.present(signupForm());
    root.querySelector<HTMLInputElement>('[data-world-form-field="name"]')!.value = "Ada";
    root.querySelector<HTMLSelectElement>('[data-world-form-field="plan"]')!.value = "pro";
    root.querySelector<HTMLSelectElement>('[data-world-form-field="terms"]')!.value = "true";
    root.querySelector<HTMLButtonElement>('[data-world-form-action="submit"]')!.click();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({
      type: "form_result",
      form_id: "signup",
      presentation_token: TOKEN,
      action: "submit",
      values: { name: "Ada", plan: "pro", terms: true },
    });
  });

  it("sends a cancel frame with the view token and no values", () => {
    const { root, sent, presenter } = mounted();
    presenter.present(signupForm());
    root.querySelector<HTMLInputElement>('[data-world-form-field="name"]')!.value = "ignored";
    root.querySelector<HTMLButtonElement>('[data-world-form-action="cancel"]')!.click();
    expect(sent).toEqual([
      { type: "form_result", form_id: "signup", presentation_token: TOKEN, action: "cancel", values: {} },
    ]);
  });

  it("renders no actionable widget for a form its manifest cannot collect", () => {
    const root = document.createElement("div");
    const send = vi.fn(() => true);
    const presenter = createFormPresenter(root, { manifest: { keys: 0, text_entry: "none" }, send });
    expect(presenter.present(signupForm())).toBe(false);
    expect(root.querySelectorAll("[data-world-form-field]")).toHaveLength(0);
    expect(root.querySelectorAll("[data-world-form-action]")).toHaveLength(0);
    expect(root.querySelector("pre")?.textContent).toContain("Sign up");
    expect(presenter.active).toBeNull();
  });

  it("re-presents with the returned token after a rejected answer", () => {
    const { presenter } = mounted();
    presenter.present(signupForm());
    const outcome = presenter.settle({
      type: "form_result_status",
      form_id: "signup",
      ok: false,
      presentation_token: NEXT_TOKEN,
      error: "invalid_values",
      reason: "name required",
    });
    expect(outcome.rejected).toBe(true);
    expect(presenter.token()).toBe(NEXT_TOKEN);
  });

  it("keeps the host token when a redraw still carries the program's old token", () => {
    const { presenter } = mounted();
    presenter.present(signupForm());
    presenter.settle({ type: "form_result_status", form_id: "signup", ok: false, presentation_token: NEXT_TOKEN });
    // The guest program redraws with the token it still has.
    expect(presenter.present(signupForm())).toBe(true);
    expect(presenter.token()).toBe(NEXT_TOKEN);
  });

  it("clears the form once the host accepts the answer", () => {
    const { root, presenter } = mounted();
    presenter.present(signupForm());
    const outcome = presenter.settle({ type: "form_result_status", form_id: "signup", ok: true });
    expect(outcome).toEqual({ accepted: true, rejected: false, closed: true });
    expect(presenter.active).toBeNull();
    expect(root.querySelector("[data-world-form]")).toBeNull();
  });

  it("closes the form when a rejection carries no retry token", () => {
    const { presenter } = mounted();
    presenter.present(signupForm());
    const outcome = presenter.settle({
      type: "form_result_status",
      form_id: "signup",
      ok: false,
      error: "form_closed",
      reason: "program reformatted",
    });
    expect(outcome.closed).toBe(true);
    expect(presenter.active).toBeNull();
  });

  it("ignores a status for a form that is no longer active", () => {
    const { presenter } = mounted();
    expect(presenter.settle({ form_id: "signup", ok: true })).toEqual({
      accepted: false,
      rejected: false,
      closed: true,
    });
  });

  it("renders an escape-free text projection for a rejected form", () => {
    const projection = formTextProjection(signupForm());
    expect(projection.split("\n")[0]).toBe("Sign up");
    expect(projection).toContain("1. Name — text, max 40 (required)");
    expect(projection).not.toContain("\u001b");
  });
});
