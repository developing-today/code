// Type surface for promote.mjs (a plain Node script; vitest tests import
// its pure helper). deploymentStatus classifies the live deployment
// against a provenance manifest: "already-promoted" when the manifest
// version is already the sole 100% deployment, "at-baseline" when the
// active version matches the manifest baseline, "drifted" otherwise.
export type DeploymentStatus = "already-promoted" | "at-baseline" | "drifted";

export interface ManifestVersionEntry {
  version_id?: string;
  percentage?: number;
}

export interface ManifestDeployment {
  versions?: ManifestVersionEntry[];
}

export interface PromotionManifest {
  versionId: string;
  baselineVersionId: string;
}

export function deploymentStatus(
  current: ManifestDeployment | undefined,
  manifest: PromotionManifest,
): DeploymentStatus;
