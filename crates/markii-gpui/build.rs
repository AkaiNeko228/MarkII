fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../src-tauri/icons/icon.ico");
        res.set("ProductName", "Mark II");
        res.set("FileDescription", "Mark II PC monitor");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
