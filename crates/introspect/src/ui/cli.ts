/**
 * CLI wrapper for introspect commands
 * This file is called by the Rust CLI to execute introspect subcommands
 */

import { introspectTop, introspectWorkers, introspectInspect, introspectKill } from './introspect.ts'

const args = process.argv.slice(2)
const command = args[0]

if (!command) {
  console.error('No command specified')
  process.exit(1)
}

// Parse options from remaining args
const options: Record<string, any> = {}
const positionals: string[] = []

for (let i = 1; i < args.length; i++) {
  const arg = args[i]
  if (arg.startsWith('--')) {
    const key = arg.slice(2)
    if (key === 'json' || key === 'archive') {
      options[key] = true
    } else if (i + 1 < args.length && !args[i + 1].startsWith('--')) {
      options[key] = args[i + 1]
      i++
    } else {
      options[key] = true
    }
  } else if (arg.startsWith('-') && arg.length === 2) {
    const key = arg.slice(1)
    if (i + 1 < args.length && !args[i + 1].startsWith('-')) {
      options[key] = args[i + 1]
      i++
    } else {
      options[key] = true
    }
  } else {
    positionals.push(arg)
  }
}

// Map short flags to long flags
if (options.r) {
  options.runtime = options.r
  delete options.r
}
if (options.s) {
  options.sort = options.s
  delete options.s
}
if (options.l) {
  options.limit = options.l
  delete options.l
}

async function main() {
  try {
    switch (command) {
      case 'top':
        await introspectTop(options)
        break
      case 'workers':
        await introspectWorkers(options)
        break
      case 'inspect':
        if (positionals.length === 0) {
          console.error('inspect requires a handler argument')
          process.exit(1)
        }
        await introspectInspect(positionals[0], options)
        break
      case 'kill':
        if (positionals.length === 0) {
          console.error('kill requires a handler argument')
          process.exit(1)
        }
        await introspectKill(positionals[0], options)
        break
      default:
        console.error(`Unknown command: ${command}`)
        process.exit(1)
    }
  } catch (error) {
    console.error('Error:', error instanceof Error ? error.message : String(error))
    process.exit(1)
  }
}

main()
