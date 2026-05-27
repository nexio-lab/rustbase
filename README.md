# RustBaas

A multi-tenant Backend-as-a-Service in Rust. One binary, one `data/` folder.

Drop the executable on a server, run the setup wizard, and you have realms,
apps, collections, auth, realtime, file storage, a dashboard, and a REST API.
SQLite under the hood (one file per scope, all under `data/`) for maximum
operational simplicity.

## Status

Early design / scaffolding phase. See [CLAUDE.md](CLAUDE.md) for the
authoritative architecture, mental model, and conventions.

## Quick start (planned)

```sh
./rustbase                # starts the server on :8080
# open http://localhost:8080/_/ to create the master admin
```

## License

Dual-licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
