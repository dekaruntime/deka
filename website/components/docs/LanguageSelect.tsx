'use client'

import { useState } from 'react'
import { ChevronUp } from 'lucide-react'
import { languages } from '@/i18n'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { useLangOnClient } from '@/context/lang'

interface LanguageSelectProps {
  className?: string
}

export function LanguageSelect({ className }: LanguageSelectProps) {
  const [isOpen, setIsOpen] = useState(false)
  const { data: lang, action: setLang } = useLangOnClient()
  const selectedLang = lang ?? (languages[0]?.code ?? 'en')

  const handleSelect = async (code: string) => {
    if (code === selectedLang) {
      setIsOpen(false)
      return
    }
    setLang?.(code)
    setIsOpen(false)
  }

  return (
    <div className={className}>
      <div className="text-xs uppercase tracking-wide text-muted-foreground mb-2">language</div>
      <Popover open={isOpen} onOpenChange={setIsOpen}>
        <PopoverTrigger asChild>
          <button
            type="button"
            className="flex w-full items-center justify-between rounded-lg border border-border/60 bg-background/80 px-3 py-2 text-sm text-foreground shadow-sm transition-colors hover:border-primary/60 focus:outline-none"
            aria-haspopup="listbox"
          >
            <span className="text-xs uppercase tracking-wide">{selectedLang.slice(0, 2)}</span>
            <ChevronUp className="h-4 w-4 text-muted-foreground" />
          </button>
        </PopoverTrigger>
        <PopoverContent
          side="top"
          align="start"
          sideOffset={8}
          className="z-[80] w-[min(24rem,90vw)] rounded-xl border border-border/60 bg-background p-2 text-sm text-foreground shadow-xl"
        >
          <div className="grid grid-cols-3 gap-1" role="listbox">
            {languages.map((lang) => (
              <button
                key={lang.code}
                type="button"
                onClick={() => void handleSelect(lang.code)}
                className={`flex w-full items-center justify-between rounded-lg px-3 py-2 text-left transition-colors ${
                  lang.code === selectedLang
                    ? 'bg-primary/10 text-foreground'
                    : 'text-muted-foreground hover:bg-secondary/50 hover:text-foreground'
                }`}
                role="option"
                aria-selected={lang.code === selectedLang}
                title={lang.name}
              >
                <span className="truncate">{lang.name}</span>
                <span className="text-xs uppercase text-muted-foreground">
                  {lang.code.slice(0, 2)}
                </span>
              </button>
            ))}
          </div>
        </PopoverContent>
      </Popover>
    </div>
  )
}
