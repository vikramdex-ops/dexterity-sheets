//! Windows only: the icon and version information of `DexSheetsServerManager.exe`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GRIDCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/dexterity-sheets.ico")
        .set("ProductName", "Dexterity Sheets Server")
        .set("FileDescription", "Dexterity Sheets Server Manager")
        .set("CompanyName", "Dexterity Design Services Pvt. Ltd.")
        .set(
            "LegalCopyright",
            "Copyright (c) Dexterity Design Services Pvt. Ltd. Based on GridCraft, copyright (c) the GridCraft authors, MIT OR Apache-2.0.",
        )
        .set("OriginalFilename", "DexSheetsServerManager.exe")
        .set("InternalName", "DexSheetsServerManager");
    if let Err(e) = res.compile() {
        if std::env::var_os("GRIDCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=DexSheetsServerManager.exe built without icon/version resources: {e}");
    }
}
