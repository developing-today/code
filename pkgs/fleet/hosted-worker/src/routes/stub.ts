// The skeleton stub every door route answers with in this slice
// (#12583): authentication and tenancy plumbing are real, business
// logic is not — the handler refuses 501 and names the issue that
// fills the door, so a caller can tell "door exists, not yet
// implemented" apart from "no such route" (404) and "refused" (401/403).

import { FailClosed } from "../errors.ts";

export function stubHandler(door: string, fillingIssue: string): () => never {
  return () => {
    throw new FailClosed(
      501,
      "not_implemented",
      `the ${door} door is a skeleton stub in #12583; its implementation is ${fillingIssue}`,
    );
  };
}
