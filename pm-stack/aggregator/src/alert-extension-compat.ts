export type NeedsUserExtensionSelection =
  | { kind: "absent" }
  | { kind: "ambiguous" }
  | { kind: "selected"; alias: "needsDrew" | "needsUser"; value: unknown };

/** Select the new or legacy alert extension without allowing mixed shapes. */
export function selectNeedsUserExtension(extensions: Record<string, unknown>): NeedsUserExtensionSelection {
  const hasNeedsDrew = extensions.needsDrew !== undefined;
  const hasNeedsUser = extensions.needsUser !== undefined;
  if (hasNeedsDrew && hasNeedsUser) return { kind: "ambiguous" };
  if (hasNeedsUser) return { kind: "selected", alias: "needsUser", value: extensions.needsUser };
  if (hasNeedsDrew) return { kind: "selected", alias: "needsDrew", value: extensions.needsDrew };
  return { kind: "absent" };
}
