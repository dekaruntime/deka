# Emitted-output controls (deka#592)

Run one command from this directory:

```sh
DEKA=/path/to/target/release/cli ./run.sh
```

The script transpiles every `.ds` case, runs the emitted JavaScript and its
hand-written neighboring `.js` control with Node, and prints the median of
five `/usr/bin/time -p` wall-clock samples. Compilation is outside the timed
interval. Each pair performs the same 2,000,000-operation workload and prints
a sink so the work cannot be discarded.

The cases correspond, in issue order, to struct construction, `implMut`
guards, newtype boxing, enum namespace tables, JSX static-props object
construction, string/array builtins, and the `__deka_type_of` cache. The JSX
case isolates the props-object construction currently emitted around JSX; it
does not claim to benchmark renderer or DOM work. The control deliberately
uses ordinary JavaScript representations, including a frozen enum table.

The result is a representation-tax comparison: generated seconds divided by
control seconds. A result near 1.00x is parity; ranking is by generated minus
control time, not by an earlier survey's guessed cost.
