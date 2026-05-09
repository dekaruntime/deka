import NextAuth from 'next-auth'
import { TanaProvider } from '@/lib/sso'

const handler = NextAuth({
  providers: [
    TanaProvider({
      idpUrl: process.env.SSO_IDP_URL || 'https://id.tana.gg',
      clientId: 'deka.gg',
      clientSecret: process.env.SSO_CLIENT_SECRET!,
    }),
  ],
  secret: process.env.NEXTAUTH_SECRET,
})

export { handler as GET, handler as POST }
