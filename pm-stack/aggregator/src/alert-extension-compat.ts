/** The legacy needsDrew question extension is the current alert writer. */
export type NeedsUserExtensionSelection =
  | { kind: "absent" }
  | { kind: "selected"; alias: "needsDrew"; value: unknown };

/** Select the current alert extension shape. The needsUser alias is not
 * enabled until its compatibility test and reader land. */
export function selectNeedsUserExtension(extensions: Record<string, unknown>): NeedsUserExtensionSelection {
  if (extensions.needsDrew === undefined) return { kind: "absent" };
  return { kind: "selected", alias: "needsDrew", value: extensions.needsDrew };
}
