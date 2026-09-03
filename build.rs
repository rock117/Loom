fn main() {
    println!("cargo:rerun-if-changed=resources/windows/loom.rc");
    println!("cargo:rerun-if-changed=assets/icons/loom.ico");
    println!("cargo:rerun-if-changed=assets/icons/loom.svg");

    #[cfg(target_os = "windows")]
    {
        // GPUI loads resource ID 1 as the app/window icon on Windows.
        embed_resource::compile("resources/windows/loom.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("embed Windows icon resource");
    }
}
