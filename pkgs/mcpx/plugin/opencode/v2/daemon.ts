/**
 * The daemon core is harness-free and shared with v1: discovery, the /v1
 * socket client, ranking and the remembered-target types all live in
 * ../mcpx/daemon.ts and are imported unchanged (design doc §5). This module
 * exists so the v2 tree names its dependency in one place.
 */
export * from "../mcpx/daemon.ts";
