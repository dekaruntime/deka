'use client'

import { useEffect, useMemo, useRef, useState } from 'react'
import Link from 'next/link'
import { ChevronUp, Sun, Moon } from 'lucide-react'
import { useTheme } from '@/context/theme-context'
import { useLangOnClient } from '@/context/lang'
import { languages } from '@/i18n'

const footerLinks = [
  { name: 'legal', href: '/legal' },
  { name: 'docs', href: '/docs' },
  { name: 'blog', href: '/blog' },
  { name: 'status', href: '/status' },
]

export default function Footer() {
  const { theme, toggleTheme } = useTheme()
  const [isLangOpen, setIsLangOpen] = useState(false)
  const [downloadCount, setDownloadCount] = useState(100000)
  const langRef = useRef<HTMLDivElement | null>(null)
  const { data: lang, action: setLang } = useLangOnClient()
  const selectedLang = lang ?? (languages[0]?.code ?? 'en')

  const handleSelect = async (code: string) => {
    if (code === selectedLang) {
      setIsLangOpen(false)
      return
    }
    setLang?.(code)
    setIsLangOpen(false)
    // Language preference persists via the cookie set by `useLangOnClient` —
    // no server-side user record to update, so nothing else to do.
  }

  useEffect(() => {
    const interval = window.setInterval(() => {
      setDownloadCount((prev) => prev + Math.floor(Math.random() * 6) + 1)
    }, 900)

    const handleClick = (event: MouseEvent) => {
      if (!langRef.current?.contains(event.target as Node)) {
        setIsLangOpen(false)
      }
    }

    const handleKeydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setIsLangOpen(false)
      }
    }

    document.addEventListener('mousedown', handleClick)
    document.addEventListener('keydown', handleKeydown)

    return () => {
      window.clearInterval(interval)
      document.removeEventListener('mousedown', handleClick)
      document.removeEventListener('keydown', handleKeydown)
    }
  }, [])

  const formattedDownloads = useMemo(
    () => downloadCount.toLocaleString('en-US'),
    [downloadCount]
  )

  return (
    <footer className="relative z-50 border-t border-black/10 bg-white/70 px-4 py-12 backdrop-blur-md dark:border-white/10 dark:bg-black/60 lg:px-8">
      <div className="mx-auto flex max-w-7xl flex-col items-center justify-between gap-4 md:flex-row">
        <Link href="/" className="group flex items-center gap-3">
          <img
            src="/android-chrome-512x512.png"
            alt="deka"
            className="h-9 w-9 rounded-full"
          />
          <span className="text-sm text-black/60 transition-colors group-hover:text-black/80 dark:text-white/60 dark:group-hover:text-white/80">
            {formattedDownloads} downloads and counting
          </span>
        </Link>
        <div className="flex flex-wrap items-center gap-2 text-sm text-black/70 dark:text-white/70">
          {footerLinks.map((item) => (
            <Link
              key={item.name}
              href={item.href}
              className="rounded-full px-3 py-2 transition-colors hover:bg-black/5 hover:text-black dark:hover:bg-white/10 dark:hover:text-white"
            >
              {item.name}
            </Link>
          ))}
          <div ref={langRef} className="relative">
            <button
              type="button"
              onClick={() => setIsLangOpen((open) => !open)}
              className="flex items-center gap-2 rounded-full px-3 py-2 text-black/70 transition-colors hover:bg-black/5 hover:text-black dark:text-white/70 dark:hover:bg-white/10 dark:hover:text-white"
              aria-haspopup="listbox"
              aria-expanded={isLangOpen}
            >
              <span className="text-xs uppercase">{selectedLang.slice(0, 2)}</span>
              <ChevronUp className={`h-4 w-4 transition-transform ${isLangOpen ? 'rotate-0' : 'rotate-180'}`} />
            </button>
            {isLangOpen && (
              <div className="absolute bottom-full right-0 mb-2 w-96 rounded-2xl border border-black/10 bg-white/95 p-2 text-sm text-black shadow-lg z-50 dark:border-white/10 dark:bg-black/90 dark:text-white">
                <div className="grid grid-cols-3 gap-1">
                  {languages.map((lang) => (
                    <button
                      key={lang.code}
                      type="button"
                      onClick={() => void handleSelect(lang.code)}
                      className={`flex w-full items-center justify-between rounded-lg px-3 py-2 text-left transition-colors ${
                        lang.code === selectedLang
                          ? 'bg-black/5 text-black dark:bg-white/10 dark:text-white'
                          : 'text-black/80 hover:bg-black/5 dark:text-white/80 dark:hover:bg-white/10'
                      }`}
                      role="option"
                      aria-selected={lang.code === selectedLang}
                    >
                      <span className="truncate">{lang.name}</span>
                      <span className="text-xs uppercase text-black/60 dark:text-white/60">
                        {lang.code.slice(0, 2)}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
          <button
            onClick={toggleTheme}
            className="rounded-full p-2 transition-colors hover:bg-black/5 dark:hover:bg-white/10"
            aria-label="Toggle theme"
          >
            {theme === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </button>
        </div>
      </div>
    </footer>
  )
}
