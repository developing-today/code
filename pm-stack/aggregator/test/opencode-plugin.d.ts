// The session-identity tool (opencode-tools/) is installed into an
// OpenCode config dir, where @opencode-ai/plugin resolves for real.
// Inside this package the tool is only executed under a test mock, so
// declare the minimal surface it uses to keep the typecheck honest.
declare module "@opencode-ai/plugin" {
  export interface ToolContext {
    sessionID: string;
    agent?: string;
    directory?: string;
    worktree?: string;
  }
  export interface ToolDef {
    description: string;
    args: Record<string, never>;
    execute: (args: Record<string, never>, context: ToolContext) => Promise<string>;
  }
  export function tool(def: ToolDef): ToolDef;
}
