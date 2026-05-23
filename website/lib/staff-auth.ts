import neo4j, { type Driver } from 'neo4j-driver'

type StaffRow = {
  is_staff?: boolean | null
}

let driver: Driver | null = null

function getNeo4jDriver(): Driver {
  if (!driver) {
    const uri = process.env.NEO4J_URI || process.env.NEO4J_URL || 'bolt://localhost:7688'
    const user = process.env.NEO4J_USER || 'neo4j'
    const password = process.env.NEO4J_PASSWORD || 'password'
    driver = neo4j.driver(uri, neo4j.auth.basic(user, password))
  }
  return driver
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

export async function isStaffEmailInNeo4j(email: string): Promise<boolean> {
  const session = getNeo4jDriver().session()
  try {
    const result = await session.run(
      'MATCH (u:User {email: $email}) RETURN u.is_staff AS is_staff',
      { email },
    )
    const row = result.records[0]?.toObject() as StaffRow | undefined
    return row?.is_staff === true
  } finally {
    await session.close()
  }
}

export async function authorizeStaffEmail(emailValue: unknown): Promise<boolean> {
  const email = typeof emailValue === 'string' ? emailValue.trim().toLowerCase() : ''
  if (!email) return false

  try {
    if (await isStaffEmailInNeo4j(email)) {
      return true
    }
  } catch (err) {
    console.error(`[deka-sso] Neo4j staff lookup failed for ${email}:`, err)
    return allowViaEnvFallback(email, 'neo4j_lookup_failed')
  }

  return allowViaEnvFallback(email, 'neo4j_is_staff_false')
}
