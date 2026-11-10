export declare const USAGE: string;

export declare function parseArgs(argv: string[]): {
  dryRun: boolean;
  deployVersion: string | null;
};
