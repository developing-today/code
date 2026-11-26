// The authoritative account/binding reader for the hosted ID world.

import { FailClosed, parseBindingId } from "./routing.ts";

/** The small D1 statement surface used by the central tenancy registry. */
export interface TenancyRegistryStatement {
  bind(...values: Array<string | number | null>): TenancyRegistryStatement;
  first<T extends Record<string, unknown>>(): Promise<T | null>;
}

/** Central tenancy database binding (D1 in the local Worker harness). */
export interface TenancyDatabase {
  prepare(query: string): TenancyRegistryStatement;
}

export interface RegistryBinding {
  bindingId: string;
  bindingStatus: string;
  accountState: string;
}

export type ProvisioningBlockReason = "binding_missing" | "binding_not_active" | "account_not_active";

type RegistryRow = Record<string, unknown> & {
  binding_id: string;
  binding_status: string;
  account_state: string;
};

const LOOKUP_BINDING = `
  SELECT b.binding_id AS binding_id,
         b.status AS binding_status,
         a.state AS account_state
    FROM binding b
    JOIN account a ON a.account_id = b.account_id
   WHERE b.binding_id = ?
`;

export class TenancyRegistry {
  constructor(private readonly database?: TenancyDatabase) {}

  /** Read the central binding and its owning account state. Missing rows
   * return null; database and corrupt-row errors fail closed. */
  async binding(bindingId: string | null): Promise<RegistryBinding | null> {
    if (bindingId === null) return null;
    const validatedBindingId = parseBindingId(bindingId);
    if (!this.database) {
      throw new FailClosed(
        503,
        "tenancy_registry_unavailable",
        "central tenancy registry is not configured; failing closed",
      );
    }

    let row: RegistryRow | null;
    try {
      row = await this.database.prepare(LOOKUP_BINDING).bind(validatedBindingId).first<RegistryRow>();
    } catch {
      throw new FailClosed(
        503,
        "tenancy_registry_unavailable",
        "central tenancy registry is unavailable; failing closed",
      );
    }
    if (row === null) return null;
    if (
      typeof row.binding_id !== "string" ||
      row.binding_id !== validatedBindingId ||
      typeof row.binding_status !== "string" ||
      typeof row.account_state !== "string"
    ) {
      throw new FailClosed(
        503,
        "invalid_tenancy_registry_record",
        "central tenancy registry returned an invalid binding record; failing closed",
      );
    }
    return {
      bindingId: row.binding_id,
      bindingStatus: row.binding_status,
      accountState: row.account_state,
    };
  }

  /** Why an account/binding pair cannot start tenant provisioning, or null
   * when both authoritative rows are active. */
  async provisioningBlockReason(bindingId: string | null): Promise<ProvisioningBlockReason | null> {
    const row = await this.binding(bindingId);
    if (row === null) return "binding_missing";
    if (row.bindingStatus !== "active") return "binding_not_active";
    if (row.accountState !== "active") return "account_not_active";
    return null;
  }
}
