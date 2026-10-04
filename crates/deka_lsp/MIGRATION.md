# Migration source

This crate carries the Rust language-server analysis and stdio lifecycle from
`dekaruntime/dsc`, commit `af322e8`. The native integration replaces that
version's separate JS-oriented acceptance path and compiler bridge with the
same Deka compiler/host catalog used by the CLI. Source syntax analysis,
completion contexts, hover, definitions, reference/rename helpers and quick
fixes remain Rust code from that implementation.
