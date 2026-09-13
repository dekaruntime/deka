# deka-bench phase 2 results

**PRELIMINARY — pending one clean idle-host rerun.** Numbers were measured on a shared host with the contention guard (**10 waits / 8 discards**). The guard mitigates compiler contention; it does not make this an isolated run. Ava schedules the clean idle-host rerun before homepage use. No benchmark rerun was performed for this review fix.

Generated: 2026-09-13T20:50:09.106Z

Machine: macOS 26.6.1, x86_64, Intel(R) Core(TM) i9-10910 CPU @ 3.60GHz, 128 GiB, Node v26.3.1.

Toolchains (exact output and binary hashes in JSON):

```json
{
  "note": "Release build with dev-server from this PR, including app-router Content-Type and server-component reload fixes; not a released CLI.",
  "deka": "deka [version 0.52.0]\ngit_sha: 2a8ed1bf6a4c\nbuild_unix: 1789331313\ntarget: x86_64-apple-darwin\nruntime_abi: deka-runtime-phpx-v1\nreact: 19.1.1\n\nto check for updates run: deka --update",
  "dsc": "dsc [version 0.52.2]",
  "chrome": "Google Chrome 152.0.7977.83",
  "node": "v26.3.1",
  "installed": {
    "vite-blog": {
      "name": "deka-bench-vite-blog",
      "dependencies": {
        "@types/react-dom": {
          "version": "19.1.6",
          "resolved": "https://registry.npmjs.org/@types/react-dom/-/react-dom-19.1.6.tgz",
          "overridden": false
        },
        "@types/react": {
          "version": "19.1.8",
          "resolved": "https://registry.npmjs.org/@types/react/-/react-19.1.8.tgz",
          "overridden": false
        },
        "@vitejs/plugin-react": {
          "version": "4.5.2",
          "resolved": "https://registry.npmjs.org/@vitejs/plugin-react/-/plugin-react-4.5.2.tgz",
          "overridden": false
        },
        "react-dom": {
          "version": "19.1.1",
          "resolved": "https://registry.npmjs.org/react-dom/-/react-dom-19.1.1.tgz",
          "overridden": false
        },
        "react-router-dom": {
          "version": "7.6.2",
          "resolved": "https://registry.npmjs.org/react-router-dom/-/react-router-dom-7.6.2.tgz",
          "overridden": false
        },
        "react": {
          "version": "19.1.1",
          "resolved": "https://registry.npmjs.org/react/-/react-19.1.1.tgz",
          "overridden": false
        },
        "typescript": {
          "version": "5.8.3",
          "resolved": "https://registry.npmjs.org/typescript/-/typescript-5.8.3.tgz",
          "overridden": false
        },
        "vite": {
          "version": "6.3.5",
          "resolved": "https://registry.npmjs.org/vite/-/vite-6.3.5.tgz",
          "overridden": false
        }
      }
    },
    "next-blog": {
      "name": "deka-bench-next-blog",
      "dependencies": {
        "@types/node": {
          "version": "22.15.30",
          "resolved": "https://registry.npmjs.org/@types/node/-/node-22.15.30.tgz",
          "overridden": false
        },
        "@types/react-dom": {
          "version": "19.1.6",
          "resolved": "https://registry.npmjs.org/@types/react-dom/-/react-dom-19.1.6.tgz",
          "overridden": false
        },
        "@types/react": {
          "version": "19.1.8",
          "resolved": "https://registry.npmjs.org/@types/react/-/react-19.1.8.tgz",
          "overridden": false
        },
        "next": {
          "version": "16.3.5",
          "resolved": "https://registry.npmjs.org/next/-/next-16.3.5.tgz",
          "overridden": false
        },
        "react-dom": {
          "version": "19.3.0",
          "resolved": "https://registry.npmjs.org/react-dom/-/react-dom-19.3.0.tgz",
          "overridden": false
        },
        "react": {
          "version": "19.3.0",
          "resolved": "https://registry.npmjs.org/react/-/react-19.3.0.tgz",
          "overridden": false
        },
        "typescript": {
          "version": "5.8.3",
          "resolved": "https://registry.npmjs.org/typescript/-/typescript-5.8.3.tgz",
          "overridden": false
        }
      }
    }
  },
  "vite": {
    "name": "deka-bench-vite-blog",
    "private": true,
    "type": "module",
    "scripts": {
      "dev": "vite",
      "build": "vite build",
      "preview": "vite preview --host 127.0.0.1 --port 4173"
    },
    "dependencies": {
      "react": "19.1.1",
      "react-dom": "19.1.1",
      "react-router-dom": "7.6.2"
    },
    "devDependencies": {
      "@types/react": "19.1.8",
      "@types/react-dom": "19.1.6",
      "@vitejs/plugin-react": "4.5.2",
      "typescript": "5.8.3",
      "vite": "6.3.5"
    }
  },
  "next": {
    "name": "deka-bench-next-blog",
    "private": true,
    "type": "module",
    "scripts": {
      "dev": "next dev",
      "build": "next build",
      "start": "next start"
    },
    "dependencies": {
      "next": "16.3.5",
      "react": "19.3.0",
      "react-dom": "19.3.0"
    },
    "devDependencies": {
      "typescript": "5.8.3",
      "@types/node": "22.15.30",
      "@types/react": "19.1.8",
      "@types/react-dom": "19.1.6"
    }
  },
  "dekaSha256": "21daa7f9af9174b390fdee268453122ba34ea4eb43878f58e57fab49fbbd1bb3",
  "dscSha256": "eba359385e0c4bdc8275cd04f151bdc66abe336bed9549047588119b6109ca2a",
  "sourceCommit": "b7802aeb3aa63ee66fbc842099ced4a1a4c9eb0f"
}
```

## PRELIMINARY measurements — clean idle-host rerun pending

| stack | cold build (median 5) | incremental build (median 5) | warm TTFB (median 25) | component update (median 5) | full reloads |
| --- | ---: | ---: | ---: | ---: | ---: |
| deka | 1016.09 ms | 1032.39 ms | 0.88 ms | 110.43 ms | 5/5 |
| vite | 1096.03 ms | 1078.04 ms | 0.51 ms | 147.26 ms | 0/5 |
| next | 5129.07 ms | 3265.59 ms | 1.00 ms | 52.21 ms | 0/5 |

## Payload story A — no application client islands: Deka zero JS; Next static-page framework JS floor; Vite CSR

Same URL: `/posts/islands-are-a-budget`. Fresh browser/cache; gzip-normalized bodies, not wire compression. RSC includes default Next link prefetches observed through network idle.

| stack | HTML gzip | JS gzip | CSS gzip | RSC gzip | total gzip |
| --- | ---: | ---: | ---: | ---: | ---: |
| deka | 1374 B | 0 B | 1170 B | 0 B | 2544 B |
| vite | 293 B | 83833 B | 1170 B | 0 B | 85296 B |
| next | 3462 B | 136861 B | 1085 B | 14343 B | 155751 B |

## Payload story B — Theme + Newsletter: Deka hydrated islands; Next SSG + client components; Vite CSR

Same URL: `/posts/islands-are-a-budget`. Fresh browser/cache; gzip-normalized bodies, not wire compression. RSC includes default Next link prefetches observed through network idle.

| stack | HTML gzip | JS gzip | CSS gzip | RSC gzip | total gzip |
| --- | ---: | ---: | ---: | ---: | ---: |
| deka | 1466 B | 101400 B | 1170 B | 0 B | 104036 B |
| vite | 293 B | 83833 B | 1170 B | 0 B | 85296 B |
| next | 3377 B | 137463 B | 1085 B | 13648 B | 155573 B |

## Chromium widget checks

- deka: light->dark; Thanks — we will not actually email ava@deka.gg.
- vite: light->dark; Thanks — we will not actually email ava@deka.gg.
- next: light->dark; Thanks — we will not actually email ava@deka.gg.

Contention audit: 10 waits, 8 discarded overlapping attempts (excluded from medians; preserved in JSON). External compiler checks run at sample boundaries and every 500 ms during samples on macOS/Linux.

HMR uses the actual PostCard boundary each framework ships; any full reload is disclosed above. Content-edit HMR remains outside this phase’s component-edit task. Reproduce: `node bench/run.mjs`. See README for clock calibration, cold-cache definition, and serve commands.
