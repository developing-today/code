export type Jurisdiction = "us" | "eu";

export interface DirectoryRoute {
  activeJurisdiction: Jurisdiction;
  routingEpoch: number;
}

export interface MigrationState {
  migrationId: string;
  sourceJurisdiction: Jurisdiction;
  targetJurisdiction: Jurisdiction;
  activeJurisdiction: Jurisdiction;
  routingEpoch: number;
  phase: "pending" | "frozen" | "verified" | "switched" | "active" | "aborted";
  sourceFrozen: boolean;
  targetServing: boolean;
}

export type MigrationEvent =
  | { type: "freeze-acknowledged" }
  | { type: "snapshot-verified" }
  | { type: "switch" }
  | { type: "activate-target" }
  | { type: "abort" };

export class MigrationTransitionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MigrationTransitionError";
  }
}

export function beginMigration(
  directory: DirectoryRoute,
  request: { migrationId: string; targetJurisdiction: Jurisdiction },
): MigrationState {
  if (!request.migrationId) throw new MigrationTransitionError("migration ID must not be empty");
  if (directory.activeJurisdiction === request.targetJurisdiction) {
    throw new MigrationTransitionError("target jurisdiction must differ from source");
  }
  if (!Number.isSafeInteger(directory.routingEpoch) || directory.routingEpoch < 0) {
    throw new MigrationTransitionError("routing epoch must be a non-negative safe integer");
  }
  if (directory.routingEpoch === Number.MAX_SAFE_INTEGER) {
    throw new MigrationTransitionError("routing epoch cannot advance safely");
  }

  return {
    migrationId: request.migrationId,
    sourceJurisdiction: directory.activeJurisdiction,
    targetJurisdiction: request.targetJurisdiction,
    activeJurisdiction: directory.activeJurisdiction,
    routingEpoch: directory.routingEpoch + 1,
    phase: "pending",
    sourceFrozen: false,
    targetServing: false,
  };
}

function assertState(state: MigrationState): void {
  if (!state.migrationId || state.sourceJurisdiction === state.targetJurisdiction) {
    throw new MigrationTransitionError("invalid migration identity");
  }
  if (!Number.isSafeInteger(state.routingEpoch) || state.routingEpoch < 1) {
    throw new MigrationTransitionError("routing epoch must be a positive safe integer");
  }

  const expected: Record<MigrationState["phase"], [Jurisdiction, boolean, boolean]> = {
    pending: [state.sourceJurisdiction, false, false],
    frozen: [state.sourceJurisdiction, true, false],
    verified: [state.sourceJurisdiction, true, false],
    switched: [state.targetJurisdiction, true, false],
    active: [state.targetJurisdiction, true, true],
    aborted: [state.sourceJurisdiction, false, false],
  };
  const [active, frozen, serving] = expected[state.phase];
  if (
    state.activeJurisdiction !== active ||
    state.sourceFrozen !== frozen ||
    state.targetServing !== serving
  ) {
    throw new MigrationTransitionError(`state invariants do not hold for phase ${state.phase}`);
  }
}

export function transitionMigration(state: MigrationState, event: MigrationEvent): MigrationState {
  assertState(state);

  switch (event.type) {
    case "freeze-acknowledged":
      if (state.phase !== "pending") {
        throw new MigrationTransitionError(`cannot acknowledge freeze during ${state.phase}`);
      }
      return { ...state, phase: "frozen", sourceFrozen: true };
    case "snapshot-verified":
      if (state.phase !== "frozen") {
        throw new MigrationTransitionError(`cannot verify snapshot during ${state.phase}`);
      }
      return { ...state, phase: "verified" };
    case "switch":
      if (state.phase !== "verified") {
        throw new MigrationTransitionError(`cannot switch during ${state.phase}`);
      }
      if (state.routingEpoch === Number.MAX_SAFE_INTEGER) {
        throw new MigrationTransitionError("routing epoch cannot advance safely");
      }
      return {
        ...state,
        activeJurisdiction: state.targetJurisdiction,
        routingEpoch: state.routingEpoch + 1,
        phase: "switched",
      };
    case "activate-target":
      if (state.phase !== "switched") {
        throw new MigrationTransitionError(`cannot activate target during ${state.phase}`);
      }
      return { ...state, phase: "active", targetServing: true };
    case "abort":
      if (state.phase !== "pending" && state.phase !== "frozen" && state.phase !== "verified") {
        throw new MigrationTransitionError("cannot abort after switch");
      }
      if (state.routingEpoch === Number.MAX_SAFE_INTEGER) {
        throw new MigrationTransitionError("routing epoch cannot advance safely");
      }
      return {
        ...state,
        activeJurisdiction: state.sourceJurisdiction,
        routingEpoch: state.routingEpoch + 1,
        phase: "aborted",
        sourceFrozen: false,
        targetServing: false,
      };
  }
}
