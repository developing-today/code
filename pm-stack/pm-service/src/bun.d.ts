/**
 * Minimal ambient declarations for the Bun runtime surface this
 * package uses, so `tsc --noEmit` stays dependency-light (the
 * package's only devDependencies are the Workers types + TypeScript,
 * mirroring pm-stack/decision-events-worker). These declare only
 * what src/ calls; they are not a general Bun type library.
 */

declare module "bun:sqlite" {
  export class Database {
    constructor(path?: string);
    exec(sql: string): void;
    query(sql: string): {
      run(...params: (string | number | null)[]): unknown;
      all(...params: (string | number | null)[]): unknown[];
      get(...params: (string | number | null)[]): unknown;
    };
    transaction<T>(fn: () => T): () => T;
  }
}

declare const Bun: {
  file(path: string): { text(): Promise<string> };
  Glob: new (
    pattern: string,
  ) => {
    scan(options: { cwd: string }): AsyncIterable<string>;
  };
};

/** Bun extends ImportMeta with the module's URL and directory. */
interface ImportMeta {
  url: string;
  dir: string;
}
