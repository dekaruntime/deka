import type { OAuthConfig } from 'next-auth/providers/index'

export interface TanaProfile {
  sub: string
  email: string
  email_verified?: boolean
  name?: string
  picture?: string
  shop_id?: string
  roles?: string[]
}

export interface TanaProviderOptions {
  /** Client ID registered with the Tana IdP (e.g. "deka.gg") */
  clientId: string
  /** Client secret shared with the Tana IdP */
  clientSecret: string
  /** Base URL of the Tana identity provider */
  idpUrl: string
}

export function TanaProvider(
  options: TanaProviderOptions,
): OAuthConfig<TanaProfile> {
  const { clientId, clientSecret, idpUrl } = options

  const idpBase = idpUrl.replace(/\/$/, '')

  return {
    id: 'tana',
    name: 'Tana',
    type: 'oauth',
    version: '2.0',
    clientId,
    clientSecret,
    idToken: false,
    authorization: {
      url: `${idpBase}/auth/sso/authorize`,
      params: {
        scope: 'openid email profile',
        response_type: 'code',
      },
    },
    token: `${idpBase}/auth/sso/token`,
    userinfo: `${idpBase}/auth/userinfo`,
    checks: ['pkce', 'state'],
    profile(profile: TanaProfile) {
      return {
        id: profile.sub,
        email: profile.email,
        name: profile.name ?? profile.email,
        image: profile.picture ?? null,
      }
    },
  }
}
