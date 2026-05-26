// SPA mode: every route is client-rendered. We embed a static bundle
// in the Rust binary at /_/, so there's no Node runtime at deploy
// time and no SSR. Disabling prerender keeps the build from trying
// to crawl routes at compile time — auth state lives in localStorage
// and isn't available during prerender.
export const ssr = false;
export const prerender = false;
export const csr = true;
