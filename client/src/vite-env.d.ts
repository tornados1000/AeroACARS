/// <reference types="vite/client" />

// v0.9.0 (#GlitchTip): via vite.config.ts `define` injected
declare const __APP_VERSION__: string;

interface ImportMetaEnv {
  readonly VITE_SENTRY_DSN_CLIENT?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
