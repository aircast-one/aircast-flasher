fn main() {
    // The telemetry key is baked in with option_env!, so a cached build from
    // before the secret existed would otherwise ship without a sink.
    println!("cargo:rerun-if-env-changed=POSTHOG_KEY");
    tauri_build::build()
}
