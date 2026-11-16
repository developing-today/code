export interface Deployment {
  created_on?: string | number;
  createdOn?: string | number;
  metadata?: { created_on?: string | number };
  versions?: Array<{ version_id?: string; percentage?: number | string }>;
}

export interface HealthResponse {
  ok: boolean;
  status?: number;
  text(): Promise<string>;
}

export declare function verifyDeployment(input: {
  versionId: string;
  deployments: Deployment[];
  healthUrl: string;
  fetchHealth(url: string): Promise<HealthResponse>;
}): Promise<{ versionId: string; healthUrl: string }>;
