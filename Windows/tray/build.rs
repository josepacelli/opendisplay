//! Embeds `assets/app.ico` into `tray.exe` as resource ID 1, so the tray
//! icon (`main.rs`'s `create_message_window`/`add_tray_icon`) is the real
//! OpenDisplay icon instead of the generic `IDI_APPLICATION` stand-in.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/app.rc");
        println!("cargo:rerun-if-changed=assets/app.ico");
        embed_resource::compile("assets/app.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed assets/app.rc into tray.exe");
    }
}
