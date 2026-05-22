import { expect, test, type APIResponse } from '@playwright/test'
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http'
import { AddressInfo } from 'node:net'

const idpPort = 3101

let authorizeHit = false
let server: Server

function sendJson(response: ServerResponse, body: unknown) {
  response.writeHead(200, { 'content-type': 'application/json' })
  response.end(JSON.stringify(body))
}

function handleAuthorize(request: IncomingMessage, response: ServerResponse) {
  authorizeHit = true

  const requestUrl = new URL(request.url ?? '/', `http://127.0.0.1:${idpPort}`)
  expect(requestUrl.searchParams.get('client_id')).toBe('deka.gg')
  expect(requestUrl.searchParams.get('scope')).toContain('openid')

  const redirectUri = requestUrl.searchParams.get('redirect_uri')
  const state = requestUrl.searchParams.get('state')
  expect(redirectUri).toBe('http://127.0.0.1:3100/api/auth/callback/tana')
  expect(state).toBeTruthy()

  const callbackUrl = new URL(redirectUri!)
  callbackUrl.searchParams.set('code', 'mock-auth-code')
  callbackUrl.searchParams.set('state', state!)

  response.writeHead(302, { location: callbackUrl.toString() })
  response.end()
}

function handleToken(request: IncomingMessage, response: ServerResponse) {
  expect(request.method).toBe('POST')

  sendJson(response, {
    access_token: 'mock-access-token',
    token_type: 'Bearer',
    expires_in: 3600,
  })
}

function handleUserinfo(_request: IncomingMessage, response: ServerResponse) {
  sendJson(response, {
    sub: 'usr_deka_sso_canary',
    email: 'canary@tana.gg',
    email_verified: true,
    name: 'Deka SSO Canary',
  })
}

test.beforeAll(async () => {
  authorizeHit = false

  server = createServer((request, response) => {
    const requestUrl = new URL(request.url ?? '/', `http://127.0.0.1:${idpPort}`)

    if (requestUrl.pathname === '/auth/sso/authorize') {
      handleAuthorize(request, response)
      return
    }

    if (requestUrl.pathname === '/auth/sso/token') {
      handleToken(request, response)
      return
    }

    if (requestUrl.pathname === '/auth/userinfo') {
      handleUserinfo(request, response)
      return
    }

    response.writeHead(404)
    response.end('not found')
  })

  await new Promise<void>((resolve) => {
    server.listen(idpPort, '127.0.0.1', () => resolve())
  })

  const address = server.address() as AddressInfo
  expect(address.port).toBe(idpPort)
})

test.afterAll(async () => {
  await new Promise<void>((resolve, reject) => {
    server.close((error) => {
      if (error) {
        reject(error)
        return
      }
      resolve()
    })
  })
})

function redirectLocation(response: APIResponse) {
  const location = response.headers().location
  expect(location).toBeTruthy()
  return location!
}

test('deka.gg redirects through Tana SSO and sets a session cookie', async ({ request }) => {
  const protectedPage = await request.get('/', { maxRedirects: 0 })
  expect(protectedPage.status()).toBe(307)

  const signInPath = redirectLocation(protectedPage)
  expect(signInPath).toContain('/signin')

  const csrf = await request.get('/api/auth/csrf')
  expect(csrf.ok()).toBe(true)
  const csrfBody = (await csrf.json()) as { csrfToken: string }

  const signIn = await request.post('/api/auth/signin/tana', {
    form: {
      csrfToken: csrfBody.csrfToken,
      callbackUrl: '/',
    },
    maxRedirects: 0,
  })
  expect(signIn.status()).toBe(302)

  const authorizeUrl = redirectLocation(signIn)
  expect(authorizeUrl).toContain(`http://127.0.0.1:${idpPort}/auth/sso/authorize`)

  const authorize = await request.get(authorizeUrl, { maxRedirects: 0 })
  expect(authorize.status()).toBe(302)
  expect(authorizeHit).toBe(true)

  const callbackUrl = redirectLocation(authorize)
  expect(callbackUrl).toContain('http://127.0.0.1:3100/api/auth/callback/tana')

  const callback = await request.get(callbackUrl, { maxRedirects: 0 })
  expect(callback.status()).toBe(302)

  const finalPath = redirectLocation(callback)
  expect(finalPath).toBe('http://127.0.0.1:3100/')

  const storage = await request.storageState()
  expect(
    storage.cookies.some((cookie) => cookie.name === 'next-auth.session-token'),
  ).toBe(true)
})
