import corpus from "./test-fixtures/prosemirror-step-corpus.json";
import { Node } from "prosemirror-model";
import { Step } from "prosemirror-transform";
import { describe, expect, it } from "vitest";
import { rawSchema, richSchema } from "./editor";

const schemas = { rich: richSchema, raw: rawSchema };

describe("checked-in ProseMirror compatibility corpus", () => {
  for (const fixture of corpus.cases) {
    it(`${fixture.mode}: ${fixture.accepted ? "accepts" : `rejects ${fixture.rejection}`}`, () => {
      const schema = schemas[fixture.mode as keyof typeof schemas];
      let doc = Node.fromJSON(schema, fixture.base);
      let rejected = false;

      for (const stepJSON of fixture.steps) {
        try {
          if (!stepJSON || typeof stepJSON !== "object" || Array.isArray(stepJSON)) {
            throw new Error("step must be an object");
          }
          const step = Step.fromJSON(schema, stepJSON);
          const result = step.apply(doc);
          if (result.failed || !result.doc) throw new Error(result.failed ?? "step produced no document");
          doc = result.doc;
        } catch {
          rejected = true;
          break;
        }
      }

      expect(rejected).toBe(!fixture.accepted);
      if (fixture.accepted) expect(doc.toJSON()).toEqual(fixture.final);
    });
  }
});
