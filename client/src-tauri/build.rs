fn main() {
    // v0.9.0 (#GlitchTip): Damit `option_env!("AEROACARS_SENTRY_DSN")` in
    // `src/sentry_init.rs` zur Build-Zeit ausgewertet wird, muss Cargo
    // bei Aenderung des env neu kompilieren. Sonst bleibt der eingebrannte
    // Wert aus dem ersten Build cached.
    println!("cargo:rerun-if-env-changed=AEROACARS_SENTRY_DSN");
    // v0.9.0 (#Discord-RPC): App-ID kommt zur LAUFZEIT vom Server-Endpoint
    // (siehe src/discord_rpc.rs refresh_app_id) — daher KEIN env-changed-Trigger
    // fuer AEROACARS_DISCORD_APP_ID. Bewusst.
    tauri_build::build()
}
