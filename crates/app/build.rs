// Embeds the app icon and version details in miru.exe on Windows. The icon gets resource
// ID 1, which is what GPUI loads for the window and taskbar icon; Explorer and installer
// shortcuts use it too.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/miru.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("../../assets/brand/miru.ico", "1")
            .set("ProductName", "Miru")
            .set("FileDescription", "Miru")
            .set("LegalCopyright", "GPL-3.0-or-later");
        if let Err(e) = res.compile() {
            // Without the Windows SDK's resource compiler the app still builds, just with
            // the default icon.
            println!("cargo:warning=could not embed the app icon: {e}");
        }
    }
}
