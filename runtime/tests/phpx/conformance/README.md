# PHPX Cross-Backend Conformance

This suite is the cross-backend guard for PHPX semantics. It runs the same PHPX corpus
against backend runners and compares process output exactly: exit code, stdout, and
stderr. The current runnable backend is `v8`. The native VM slot is intentionally only
configuration, not a fake runner.

Run the current V8 backend:

```sh
bun tests/phpx/conformance/runner.js
```

Run V8 plus the native VM once a real native runner exists:

```sh
PHPX_NATIVE_BIN=target/release/<native-runner> \
  bun tests/phpx/conformance/runner.js --backends=v8,native
```

Custom backend slots can be registered with `PHPX_<NAME>_BIN` and `PHPX_<NAME>_ARGS`,
then selected with `--backends=v8,<name>`.
