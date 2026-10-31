import type { ChargeServiceRef } from "./charges-match.ts";
import type { QuotaPool, SaasService } from "./model.ts";

// Merchant names, not plan labels: generic plan words are unsafe aliases.
export const PROVIDER_ALIASES: Record<string, string[]> = {
  Anthropic: ["Claude"],
  OpenAI: ["ChatGPT", "Codex"],
  "Meta Model API": ["Meta", "MetaPay"],
  ClinePass: ["Cline Pass"],
  "Open Design": ["OpenDesign"],
  "Command Code": ["CommandCode"],
};

/** Build the match catalog from the same service and pool identities the hub exposes. */
export function chargeServiceCatalog(
  services: readonly Pick<SaasService, "name">[],
  pools: readonly Pick<QuotaPool, "provider">[],
): ChargeServiceRef[] {
  const catalog = new Map<string, ChargeServiceRef>();
  for (const { name } of services) catalog.set(name.toLowerCase(), { name });
  for (const { provider } of pools) {
    const key = provider.toLowerCase();
    if (!catalog.has(key)) catalog.set(key, { name: provider, aliases: PROVIDER_ALIASES[provider] });
  }
  return [...catalog.values()];
}
