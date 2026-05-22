import { NextResponse } from 'next/server'
import type { NextRequest } from 'next/server'
import { getToken } from 'next-auth/jwt'

export async function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl

  // Allow NextAuth API routes and static assets
  if (
    pathname.startsWith('/api/auth') ||
    pathname === '/signin' ||
    pathname.startsWith('/_next/static') ||
    pathname.startsWith('/_next/image') ||
    pathname === '/favicon.ico'
  ) {
    return NextResponse.next()
  }

  // Verify NextAuth JWT session (handles both secure and non-secure cookie names)
  const token = await getToken({
    req: request,
    secret: process.env.NEXTAUTH_SECRET,
  })

  if (!token) {
    const signInUrl = new URL('/signin', request.url)
    signInUrl.searchParams.set(
      'callbackUrl',
      request.nextUrl.pathname + request.nextUrl.search
    )
    return NextResponse.redirect(signInUrl)
  }

  return NextResponse.next()
}

export const config = {
  matcher: ['/((?!_next/static|_next/image|favicon.ico).*)'],
}
