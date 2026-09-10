// Consumer view of the `rust:storefront` seam — the request/response envelope
// the storefront handler is handed and must return.
//
// Checked by `deka contract-check --manifest contracts/seams.json`. Every field
// declared here must exist, with this type, in
// `runtime_core::storefront_envelope::storefront_contract()`. Reading a field
// the producer does not emit fails the build.

export interface StorefrontRequest {
  method: string;
  url: string;
  path: string;
  pathname: string;
  headers: Record<string, string>;
  body: string | null;
}

export interface StorefrontResponse {
  status: number;
  headers: Record<string, string>;
  body: string;
  body_base64?: string;
  upgrade?: Record<string, string>;
}
