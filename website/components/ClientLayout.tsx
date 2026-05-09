'use client'

import { usePathname } from "next/navigation"
import { ThemeProvider } from "@/context/theme-context"
import { LangContextProvider } from "@/context/lang"
import { MonacoLoader } from "@/components/MonacoLoader"
import Footer from "@/components/core/Footer"
import { languages } from "@/i18n"

interface ClientLayoutProps {
  children: React.ReactNode
  initialLang?: string
}

export function ClientLayout({ children, initialLang }: ClientLayoutProps) {
  const pathname = usePathname()
  const hideFooter = pathname?.startsWith('/help') ||
                     pathname?.startsWith('/cli') ||
                     pathname?.startsWith('/api') ||
                     pathname?.startsWith('/docs/') ||
                     pathname?.startsWith('/ui')

  return (
    <ThemeProvider>
      <LangContextProvider initialLang={initialLang ?? (languages[0]?.code ?? 'en')}>
        <MonacoLoader />
        {children}
        {!hideFooter && <Footer />}
      </LangContextProvider>
    </ThemeProvider>
  )
}
