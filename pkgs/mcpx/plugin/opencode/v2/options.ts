/**
 * Moved to plugin/core/options.ts (#1861): the Options type, the
 * v1-way resolver and its helpers live in the harness-free core both
 * adapters consume. This module re-exports them so the v2 tree's
 * existing imports keep working unchanged.
 */
export * from "../../core/options.ts";
