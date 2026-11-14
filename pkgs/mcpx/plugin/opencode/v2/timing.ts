/**
 * Moved to plugin/core/timing.ts (#1861): the timing record shape and
 * the in-memory log live in the harness-free core both adapters
 * consume. This module re-exports them so the v2 tree's existing
 * imports keep working unchanged.
 */
export * from "../../core/timing.ts";
