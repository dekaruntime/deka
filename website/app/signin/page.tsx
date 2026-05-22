import { Suspense } from 'react'
import { SignInClient } from './signin-client'

export default function SignInPage() {
  return (
    <Suspense fallback={<main />}>
      <SignInClient />
    </Suspense>
  )
}
