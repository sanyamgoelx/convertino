fn main() {
    // Error reports go to this webhook (a GitHub Actions secret; see report-setup.cmd).
    println!("cargo:rerun-if-env-changed=CONVERTINO_REPORT_WEBHOOK");
    tauri_build::build()
}
