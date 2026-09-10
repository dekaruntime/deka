// Consumer view of the `rust:data_backend` seam — the envelope shapes exchanged
// with the Zega backend (CQL query/execute, KV get/set/del).
//
// This is the boundary that historically broke as `{ok, result}` versus
// `{ok, value}`. Renaming `value` here — or in the Rust producer — fails
// `deka contract-check`.

export interface CqlQueryRequest {
  cypher: string;
  params: Record<string, unknown>;
}

export interface CqlQueryResponse {
  ok: boolean;
  rows: Record<string, unknown>[];
  count: number;
  error?: string;
}

export interface CqlExecuteResponse {
  ok: boolean;
  error?: string;
}

export interface KvKeyRequest {
  key: string;
}

export interface KvSetRequest {
  key: string;
  value: string;
  ttl?: number;
}

export interface KvGetResponse {
  ok: boolean;
  value: string | null;
  error?: string;
}

export interface KvSetResponse {
  ok: boolean;
  error?: string;
}

export interface KvDelResponse {
  ok: boolean;
  deleted: number;
  error?: string;
}
