/** Persisted account and binding reads for the hosted ID world.
 * These rows in the central tenancy DB are the source of truth. */
export interface TenancySql {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}

export type ProvisioningBlockReason =
  | "binding_missing"
  | "binding_not_active"
  | "account_not_active";

export class TenancyRegistry {
  constructor(private readonly sql: TenancySql) {}

  /** Return why a binding cannot start tenant provisioning, or null
   * when both its binding and owning account are active. */
  provisioningBlockReason(bindingId: string | null): ProvisioningBlockReason | null {
    if (bindingId === null) return "binding_missing";
    const row = this.sql
      .exec(
        "SELECT b.status AS binding_status, a.state AS account_state FROM binding b JOIN account a ON a.account_id = b.account_id WHERE b.binding_id = ?",
        bindingId,
      )
      .toArray()[0];
    if (row === undefined) return "binding_missing";
    if (row.binding_status !== "active") return "binding_not_active";
    if (row.account_state !== "active") return "account_not_active";
    return null;
  }
}
