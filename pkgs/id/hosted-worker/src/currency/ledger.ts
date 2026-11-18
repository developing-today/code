// Currency PoC (#2298) — ledger behind the gateway interface.
//
// Drew's ruling hosted-boundary-2298 (2026-11-12): do not block on
// TigerBeetle; an in-process ledger stub behind the gateway
// interface is acceptable for the PoC, and the gateway is the only
// ledger client either way. This stub mirrors the TigerBeetle
// semantics the PoC's proofs depend on:
//
//   - a transfer ID (here: the client request ID) is unique;
//     re-creating it with identical fields returns the existing
//     transfer (idempotent retry), re-creating it with ANY field
//     mutated fails with `exists_with_different_fields` — a hard
//     error, never a second transfer;
//   - holds are two-phase transfers: pending reserves funds,
//     post finalises, void releases the reservation;
//   - every account carries a transfer watermark (the sequence of
//     the last transfer touching it), which the gateway signs into
//     balance snapshots.
//
// Simplification: amounts are integer minor units in a single
// currency/ledger code; balances are (posted, pending) pairs.

export class LedgerError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export type TransferState = "posted" | "pending" | "voided";

export interface TransferRecord {
  request_id: string;
  debit_account: string;
  credit_account: string;
  amount: number;
  state: TransferState;
  seq: number;
  intent_hash: string;
}

export interface TransferInput {
  request_id: string;
  debit_account: string;
  credit_account: string;
  amount: number;
  intent_hash: string;
}

export interface AccountBalance {
  balance: number;
  pending: number;
  watermark: number;
}

export interface Ledger {
  createAccount(id: string, openingBalance: number): void;
  balance(account: string): AccountBalance;
  transfer(input: TransferInput): { transfer: TransferRecord; deduplicated: boolean };
  hold(input: TransferInput): { transfer: TransferRecord; deduplicated: boolean };
  postHold(requestId: string): TransferRecord;
  voidHold(requestId: string): TransferRecord;
  lookupByRequestId(requestId: string): TransferRecord | null;
  transferCount(): number;
}

interface Account {
  posted: number;
  pending: number;
  watermark: number;
}

export class InProcessLedger implements Ledger {
  #accounts = new Map<string, Account>();
  #transfers = new Map<string, TransferRecord>();
  #seq = 0;

  createAccount(id: string, openingBalance: number): void {
    if (this.#accounts.has(id)) {
      throw new LedgerError("account_exists", "account already exists: " + id);
    }
    this.#accounts.set(id, { posted: openingBalance, pending: 0, watermark: 0 });
  }

  #account(id: string): Account {
    const acct = this.#accounts.get(id);
    if (!acct) throw new LedgerError("account_not_found", "unknown account: " + id);
    return acct;
  }

  balance(account: string): AccountBalance {
    const acct = this.#account(account);
    return { balance: acct.posted, pending: acct.pending, watermark: acct.watermark };
  }

  // The idempotency gate every create passes through: identical
  // retry returns the stored record; a mutated field under the same
  // request ID is the exists_with_different_* hard error.
  #existing(input: TransferInput): TransferRecord | null {
    const found = this.#transfers.get(input.request_id);
    if (!found) return null;
    if (found.intent_hash !== input.intent_hash) {
      throw new LedgerError(
        "exists_with_different_fields",
        "transfer " + input.request_id + " already exists with different fields",
      );
    }
    return found;
  }

  #record(input: TransferInput, state: TransferState): TransferRecord {
    const rec: TransferRecord = { ...input, state, seq: ++this.#seq };
    this.#transfers.set(input.request_id, rec);
    this.#account(input.debit_account).watermark = rec.seq;
    this.#account(input.credit_account).watermark = rec.seq;
    return rec;
  }

  transfer(input: TransferInput): { transfer: TransferRecord; deduplicated: boolean } {
    const existing = this.#existing(input);
    if (existing) return { transfer: existing, deduplicated: true };
    const debit = this.#account(input.debit_account);
    this.#account(input.credit_account);
    if (!Number.isSafeInteger(input.amount) || input.amount <= 0) {
      throw new LedgerError("invalid_amount", "amount must be a positive integer");
    }
    if (debit.posted - debit.pending < input.amount) {
      throw new LedgerError("insufficient_funds", "debit account cannot cover the transfer");
    }
    debit.posted -= input.amount;
    this.#account(input.credit_account).posted += input.amount;
    return { transfer: this.#record(input, "posted"), deduplicated: false };
  }

  hold(input: TransferInput): { transfer: TransferRecord; deduplicated: boolean } {
    const existing = this.#existing(input);
    if (existing) return { transfer: existing, deduplicated: true };
    const debit = this.#account(input.debit_account);
    this.#account(input.credit_account);
    if (!Number.isSafeInteger(input.amount) || input.amount <= 0) {
      throw new LedgerError("invalid_amount", "amount must be a positive integer");
    }
    if (debit.posted - debit.pending < input.amount) {
      throw new LedgerError("insufficient_funds", "debit account cannot cover the hold");
    }
    debit.pending += input.amount;
    return { transfer: this.#record(input, "pending"), deduplicated: false };
  }

  #pending(requestId: string): TransferRecord {
    const rec = this.#transfers.get(requestId);
    if (!rec) throw new LedgerError("transfer_not_found", "unknown transfer: " + requestId);
    if (rec.state !== "pending") {
      throw new LedgerError("transfer_not_pending", "transfer " + requestId + " is " + rec.state);
    }
    return rec;
  }

  postHold(requestId: string): TransferRecord {
    const rec = this.#pending(requestId);
    const debit = this.#account(rec.debit_account);
    debit.pending -= rec.amount;
    debit.posted -= rec.amount;
    this.#account(rec.credit_account).posted += rec.amount;
    rec.state = "posted";
    return rec;
  }

  voidHold(requestId: string): TransferRecord {
    const rec = this.#pending(requestId);
    this.#account(rec.debit_account).pending -= rec.amount;
    rec.state = "voided";
    return rec;
  }

  lookupByRequestId(requestId: string): TransferRecord | null {
    return this.#transfers.get(requestId) ?? null;
  }

  transferCount(): number {
    return this.#transfers.size;
  }
}
