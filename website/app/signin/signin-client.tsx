'use client'

import { signIn } from 'next-auth/react'
import { useSearchParams } from 'next/navigation'
import { useEffect, useRef } from 'react'

export function SignInClient() {
  const searchParams = useSearchParams()
  const callbackUrl = searchParams.get('callbackUrl') || '/'
  const startedRef = useRef(false)

  useEffect(() => {
    if (startedRef.current) {
      return
    }

    startedRef.current = true
    void signIn('tana', { callbackUrl })
  }, [callbackUrl])

  return (
    <main className="min-h-screen bg-[#f7f3ea] text-[#16130f] flex items-center justify-center px-6">
      <button
        type="button"
        onClick={() => signIn('tana', { callbackUrl })}
        className="rounded-md bg-[#16130f] px-4 py-2 text-sm font-medium text-white hover:bg-[#2a251d]"
      >
        Continue with Tana
      </button>
    </main>
  )
}
