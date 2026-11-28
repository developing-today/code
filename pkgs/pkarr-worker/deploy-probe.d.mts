export declare function isNotFoundProbeFailure(output: string | null | undefined): boolean;

export declare function probeFailureOutput(err: unknown): string;

export declare function probeWorkerExists(runGet: () => unknown, worker: string): boolean;
