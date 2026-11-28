import { describe, expect, test } from "bun:test";
import { selectNeedsUserExtension } from "../src/alert-extension-compat.ts";

const question = { ledgerId: "q-1", question: "Ship it?", options: [{ id: "yes", label: "Yes" }] };

describe("needs-user alert extension compatibility", () => {
  test("new needsUser and legacy needsDrew select the same question payload", () => {
    const legacy = selectNeedsUserExtension({ needsDrew: question });
    const roleTerm = selectNeedsUserExtension({ needsUser: question });

    expect(legacy).toEqual({ kind: "selected", alias: "needsDrew", value: question });
    expect(roleTerm).toEqual({ kind: "selected", alias: "needsUser", value: question });
  });

  test("rejects a request that supplies both alias keys", () => {
    expect(selectNeedsUserExtension({ needsDrew: question, needsUser: question })).toEqual({ kind: "ambiguous" });
  });
});
