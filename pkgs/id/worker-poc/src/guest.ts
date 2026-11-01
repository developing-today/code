// PlazaGuest: a TypeScript host for plaza_* ABI guests (pkgs/id/src/sandbox.rs
// is the native reference). The guest receives NO imports; the host drives
// only the plaza_* exports and keeps the model pointer inside guest memory,
// exactly like the native WorldInstance.
//
// ABI (sandbox.rs:3-7, 199-235):
//   plaza_init(i64) -> i32                      model ptr
//   plaza_update(i32, i32, i32, i32) -> i32     (model, participant, event ptr, event len)
//   plaza_view(i32) -> i32                      output ptr; length via plaza_out_len()
//   plaza_records(i32) -> i32                   optional
//   plaza_snapshot(i32) -> i32                  optional, paired with restore
//   plaza_restore(i32, i32) -> i32              (snapshot ptr, len) -> model
//   plaza_alloc(i32) -> i32 / plaza_free(i32, i32)
// Input discipline mirrors copy_input/free_input: allocate max(len, 1),
// pass the logical length, free with the allocation length.

export interface GuestLimits {
  messageBytes: number;
}

export const DEFAULT_LIMITS: GuestLimits = { messageBytes: 1024 * 1024 };

type PlazaExports = {
  memory: WebAssembly.Memory;
  plaza_alloc: (len: number) => number;
  plaza_free: (ptr: number, len: number) => void;
  plaza_init: (seed: bigint) => number;
  plaza_update: (model: number, participant: number, ptr: number, len: number) => number;
  plaza_view: (model: number) => number;
  plaza_records?: (model: number) => number;
  plaza_snapshot?: (model: number) => number;
  plaza_restore?: (ptr: number, len: number) => number;
  plaza_out_len: () => number;
};

export class PlazaGuest {
  private ex: PlazaExports;
  private model: number;
  private limits: GuestLimits;

  private constructor(ex: PlazaExports, model: number, limits: GuestLimits) {
    this.ex = ex;
    this.model = model;
    this.limits = limits;
  }

  static async instantiate(
    bytes: ArrayBuffer | Uint8Array,
    seed: number | bigint,
    limits: GuestLimits = DEFAULT_LIMITS,
  ): Promise<PlazaGuest> {
    const module = await WebAssembly.compile(bytes as BufferSource);
    return PlazaGuest.fromModule(module, seed, limits);
  }

  // workerd path: wrangler's CompiledWasm rule hands us a module, not bytes.
  static async fromModule(
    module: WebAssembly.Module,
    seed: number | bigint,
    limits: GuestLimits = DEFAULT_LIMITS,
  ): Promise<PlazaGuest> {
    const instance = await WebAssembly.instantiate(module, {});
    const ex = instance.exports as unknown as PlazaExports;
    for (const name of ["memory", "plaza_alloc", "plaza_free", "plaza_init", "plaza_update", "plaza_view", "plaza_out_len"] as const) {
      if (ex[name] === undefined) throw new Error(`guest must export \`${name}\``);
    }
    if ((ex.plaza_snapshot === undefined) !== (ex.plaza_restore === undefined)) {
      throw new Error("guest must export both `plaza_snapshot` and `plaza_restore`, or neither");
    }
    const model = ex.plaza_init(BigInt(seed));
    return new PlazaGuest(ex, model, limits);
  }

  private copyInput(input: Uint8Array): { ptr: number; len: number; allocationLen: number } {
    if (input.length > this.limits.messageBytes) {
      throw new Error(`guest message exceeds ${this.limits.messageBytes} bytes`);
    }
    const allocationLen = Math.max(input.length, 1);
    const ptr = this.ex.plaza_alloc(allocationLen);
    if (input.length > 0) {
      new Uint8Array(this.ex.memory.buffer, ptr, input.length).set(input);
    }
    return { ptr, len: input.length, allocationLen };
  }

  private readOutput(outputPtr: number): Uint8Array {
    const len = this.ex.plaza_out_len();
    if (len < 0) throw new Error("guest returned a negative output length");
    if (len > this.limits.messageBytes) throw new Error(`guest output exceeds ${this.limits.messageBytes} bytes`);
    const mem = new Uint8Array(this.ex.memory.buffer);
    if (outputPtr + len > mem.length) throw new Error("guest returned output outside linear memory");
    return mem.slice(outputPtr, outputPtr + len);
  }

  update(participant: number, event: Uint8Array): void {
    const { ptr, len, allocationLen } = this.copyInput(event);
    try {
      this.model = this.ex.plaza_update(this.model, participant | 0, ptr, len);
    } finally {
      this.ex.plaza_free(ptr, allocationLen);
    }
  }

  view(): Uint8Array {
    return this.readOutput(this.ex.plaza_view(this.model));
  }

  records(): Uint8Array | null {
    if (!this.ex.plaza_records) return null;
    return this.readOutput(this.ex.plaza_records(this.model));
  }

  snapshot(): Uint8Array | null {
    if (!this.ex.plaza_snapshot) return null;
    return this.readOutput(this.ex.plaza_snapshot(this.model));
  }

  restore(snapshot: Uint8Array): void {
    if (!this.ex.plaza_restore) throw new Error("guest exports no `plaza_restore`");
    const { ptr, len, allocationLen } = this.copyInput(snapshot);
    try {
      this.model = this.ex.plaza_restore(ptr, len);
    } finally {
      this.ex.plaza_free(ptr, allocationLen);
    }
  }
}
