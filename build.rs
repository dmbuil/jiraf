fn main() {
    // The app icons are embedded at compile time; rebuild when they change.
    println!("cargo:rerun-if-changed=app-icons");
    tauri_build::build()
}
