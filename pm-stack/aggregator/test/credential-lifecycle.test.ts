import { describe, expect, test } from "bun:test";
import { buildAttentionFlags } from "../src/derive.ts";
import { lifecycleForEntry, safeProviderReason } from "../src/credential-lifecycle.ts";
import { renderQuota } from "../src/render.ts";
import { overlayQuotaCurrent } from "../src/quota-current.ts";
import { sanitizeQuotaView } from "../src/feed.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaView } from "../src/model.ts";

const NOW = new Date("2026-11-03T10:00:00.000Z");

function entry(partial: Partial<QuotaCurrentEntry> = {}): QuotaCurrentEntry {
  return {
    providerId: "claude",
    providerName: "Claude",
    serviceId: "anthropic:claude-pro",
    state: "ok",
    measuredAt: NOW.toISOString(),
    source: "openchamber-quota",
    credentialWatch: true,
    windows: [],
    ...partial,
  };
}

function file(providers: QuotaCurrentEntry[]): QuotaCurrentFile {
  return {
    version: 1,
    generatedAt: NOW.toISOString(),
    source: "openchamber-quota",
    heartbeat: { tickAt: NOW.toISOString(), ok: true, durationMs: 1, perProvider: {} },
    providers,
  };
}

const emptyView = (): QuotaView => ({
  pools: [{ id: "anthropic:claude-pro", provider: "Claude", plan: "Pro", state: "healthy", windows: [] }],
  burn: [],
  choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
});

describe("credential lifecycle", () => {
  test("uses authoritative expiry boundaries and leaves missing expiry unknown", () => {
    expect(lifecycleForEntry(entry({ credentialExpiresAt: "2026-11-03T09:59:59.000Z" }), NOW).state).toBe("expired");
    expect(lifecycleForEntry(entry({ credentialExpiresAt: "2026-11-05T09:59:59.000Z" }), NOW).state).toBe("expiring");
    expect(lifecycleForEntry(entry({ credentialExpiresAt: "2026-11-06T10:00:01.000Z" }), NOW).state).toBe("ok");
    expect(lifecycleForEntry(entry(), NOW).state).toBe("unknown");
  });

  test("keeps not-configured distinct from provider errors and only labels explicit rejection", () => {
    expect(lifecycleForEntry(entry({ state: "unconfigured" }), NOW).state).toBe("not-configured");
    expect(lifecycleForEntry(entry({ state: "error", error: "HTTP 401" }), NOW).state).toBe("provider-error");
    expect(lifecycleForEntry(entry({ state: "error", error: "credential rejected by provider" }), NOW).state).toBe(
      "rejected",
    );
  });

  test("explicit expired Claude fixture creates a safe user action and attention item", () => {
    const out = overlayQuotaCurrent(
      emptyView(),
      file([
        entry({
          state: "error",
          error: "Claude session expired. Open Claude Code to sign in again.",
        }),
        entry({ providerId: "turso", providerName: "Turso", serviceId: "turso", credentialWatch: undefined }),
      ]),
      NOW,
    );
    expect(out.credentialLifecycle).toHaveLength(1);
    expect(out.credentialLifecycle?.[0]).toMatchObject({
      state: "expired",
      userAction: { owner: "user", title: "Open Claude Code to sign in again" },
    });
    const flags = buildAttentionFlags(out, { services: [], payg: [] });
    expect(flags).toContainEqual(
      expect.objectContaining({ kind: "credential-lifecycle", label: expect.stringContaining("expired") }),
    );
    const html = renderQuota(out, {} as never);
    expect(html).toContain("Credential lifecycle");
    expect(html).toContain("Open Claude Code to sign in again");
  });

  test("provider reasons are retained without bearer or token values", () => {
    expect(safeProviderReason("credential rejected: Bearer abc.def token=secret-value")).toBe(
      "credential rejected: Bearer [redacted] token=[redacted]",
    );
  });

  test("feed sanitization redacts a lifecycle reason before it reaches public JSON", () => {
    const view = emptyView();
    view.credentialLifecycle = [
      lifecycleForEntry(
        entry({
          state: "error",
          error: "credential rejected: Bearer abc.def token=secret-value",
        }),
        NOW,
      ),
    ];
    const sanitized = sanitizeQuotaView(view);
    expect(sanitized.view.credentialLifecycle?.[0].reason).toBe(
      "credential rejected: Bearer [redacted] token=[redacted]",
    );
  });
});
