# @deka/io

Program output for DekaScript. Not a `console` global.

```
import { echo } from "io"

echo("hello")
echo("" + (1 + 2))
```

## Internals

**No `bridge`.** Native is still V8: the isolate rebinds `console.log` to stdout (`__print`), so Hats `.stdout` keeps matching. The browser already has `console.log`. One call:

```
unsafe { console.log(message) }
```

User `.ds` never names `console`. That lives only in this package. `unsafe { console.log(...) }` in app code stays JS-mode (RFD 21).

No `host.kinds`, no `from "host"` branch, no `io/native` vs `io/browser`.

## API

| Binding | Role |
| --- | --- |
| `echo(message: string)` | One line of program output. Trailing newline comes from the host `console.log`. |

One argument. Non-strings: `echo("" + n)`. No rest params.

## Release

Tag after merge, same as other stdlib repos (`STDLIB.md` in `deka`). First publish is `v0.1.0`. Do not tag until asked.
