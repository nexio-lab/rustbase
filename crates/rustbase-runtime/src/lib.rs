//! Embedded JS/TS hook runtime for RustBase.
//!
//! Hooks live in `data/hooks/<realm>/<app>/` and execute in a sandboxed
//! QuickJS VM (via `rquickjs`); TypeScript is transpiled at load time
//! via `swc`. Each app's hooks see a curated `$app` global that exposes
//! records, the filter parser, the mailer, the realtime broker, and a
//! fetch client — gated by the JS-capability policy.
