export declare function deployExistingVersion(input: {
  dryRun: boolean;
  deployVersion: string;
  worker: string;
  workerUrl: string;
  createDeployment: (versionId: string) => void;
  reportLive: (versionId: string) => Promise<void>;
  log?: (message: string) => void;
}): Promise<{ deployed: boolean; dryRun: boolean; versionId: string }>;
