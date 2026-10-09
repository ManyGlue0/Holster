fn main() {
    // Tauri embeds the icons at compile time (exe resource and the tray's
    // default window icon) but doesn't watch them, so `pnpm app-icon` alone
    // would leave the old artwork in the binary.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
