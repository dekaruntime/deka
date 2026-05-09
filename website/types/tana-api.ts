/**
 * Stub types for the legacy "Tana blockchain API" client.
 *
 * The dashboard / auth / feed routes in this Next.js app reference an
 * earlier product iteration that no longer matches the current platform.
 * Types stay loose (`any`-shaped) so the dead code typechecks while we
 * decide what to resurrect vs delete.
 *
 * TODO: audit dashboard/auth/feed routes; either rewire against the
 * current Tana commerce API or delete wholesale.
 */

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type TanaUser = any
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type TanaBalance = any
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type TanaCurrency = any
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type TanaTransaction = any
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type SessionVerifyResponse = any
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type TanaApiError = any
