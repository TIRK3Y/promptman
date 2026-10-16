fn main() {
    // Embeds the app icon in the Windows executables.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icons/icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/icon.ico");
        res.compile().expect("failed to embed the Windows icon");
    }
}
