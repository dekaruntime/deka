const ZEGA_URL = process.env.ZEGA_SERVER_URL || 'http://demon:7700'
const ZEGA_TOKEN = process.env.ZEGA_SERVER_TOKEN || ''

type ZegaCqlResponse = {
  ok: boolean
  error?: string
  rows?: Record<string, unknown>[]
}

async function zegaCql(
  query: string,
  params: Record<string, unknown> = {}
): Promise<Record<string, unknown>[]> {
  const res = await fetch(`${ZEGA_URL}/cql`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${ZEGA_TOKEN}`,
    },
    body: JSON.stringify({ query, params }),
  })
  const json = (await res.json()) as ZegaCqlResponse
  if (!json.ok) throw new Error(`zega /cql: ${json.error ?? `HTTP ${res.status}`}`)
  return json.rows ?? []
}

export async function isStaffEmailInZega(email: string): Promise<boolean> {
  const rows = await zegaCql(
    'MATCH (u:User {email: $email}) RETURN u.is_staff AS is_staff',
    { email }
  )
  return rows[0]?.is_staff === true
}

function staffAllowlistEmails(): Set<string> {
  return new Set(
    (process.env.DEKA_STAFF_ALLOWLIST_EMAILS ?? process.env.STAFF_ALLOWLIST_EMAILS ?? '')
      .split(',')
      .map((email) => email.trim().toLowerCase())
      .filter(Boolean),
  )
}

function allowViaEnvFallback(email: string, reason: string): boolean {
  if (!staffAllowlistEmails().has(email)) {
    return false
  }

  console.warn(`[deka-sso] staff allowlist fallback authorized ${email}; reason=${reason}`)
  return true
}

export async function authorizeStaffEmail(emailValue: unknown): Promise<boolean> {
  const email = typeof emailValue === 'string' ? emailValue.trim().toLowerCase() : ''
  if (!email) return false

  try {
    if (await isStaffEmailInZega(email)) {
      return true
    }
  } catch (err) {
    console.error(`[deka-sso] Zega staff lookup failed for ${email}:`, err)
    return allowViaEnvFallback(email, 'zega_lookup_failed')
  }

  return allowViaEnvFallback(email, 'zega_is_staff_false')
}
