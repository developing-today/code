// Minimal Bun ambient types for the few globals the projector uses, plus the
// `bun:test` module, so `tsc --noEmit` works without a `bun-types` dependency
// (which can't be fetched offline).
declare namespace Bun {
  interface Server {
    hostname: string;
    port: number;
  }
  function serve(opts: {
    port: number;
    hostname?: string;
    fetch: (req: Request) => Response | Promise<Response>;
  }): Server;
  const file: (path: string) => { text(): Promise<string> };
}
declare var Bun: any;

declare module "bun:test" {
  export function describe(label: string, fn: () => void): void;
  export function test(label: string, fn: () => void | Promise<void>): void;
  export function expect(value: unknown): any;
  export function beforeAll(fn: () => void | Promise<void>): void;
  export function afterAll(fn: () => void | Promise<void>): void;
}
