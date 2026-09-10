// Consumer view of the `rust:platform_env_policy` seam — the env snapshot the
// platform injects into an isolate after applying the security policy.

export interface PlatformEnvVar {
  name: string;
  value: string;
}

export interface PlatformEnvPolicyRequest {
  security_policy_json: string;
  process_env: PlatformEnvVar[];
}

export interface PlatformEnvPolicySnapshot {
  env: PlatformEnvVar[];
  server: PlatformEnvVar[];
  process_env: PlatformEnvVar[];
}
