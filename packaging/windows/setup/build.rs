//! Embeds the icon and version info into DexteritySheets-Setup.exe and re-runs when the MSI it
//! carries changes. The MSI path and version come from package.ps1.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DEXSHEETS_SETUP_MSI");
    println!("cargo:rerun-if-env-changed=DEXSHEETS_SETUP_VERSION");
    println!("cargo:rerun-if-env-changed=DEXSHEETS_SETUP_PRODUCT");
    println!("cargo:rerun-if-env-changed=DEXSHEETS_SETUP_STEM");
    println!("cargo:rerun-if-env-changed=DEXSHEETS_SETUP_EXE");
    if let Ok(msi) = std::env::var("DEXSHEETS_SETUP_MSI") {
        println!("cargo:rerun-if-changed={msi}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("DEXSHEETS_SETUP_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut packed: u64 = 0;
    for (i, part) in version.split(['.', '-']).take(4).enumerate() {
        packed |= u64::from(part.parse::<u16>().unwrap_or(0)) << (48 - 16 * i);
    }
    let product = std::env::var("DEXSHEETS_SETUP_PRODUCT").unwrap_or_else(|_| "Dexterity Sheets".into());
    let exe = std::env::var("DEXSHEETS_SETUP_EXE").unwrap_or_else(|_| "DexteritySheets-Setup".into());
    let description = format!("{product} Setup");
    let original = format!("{exe}.exe");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../../assets/app-icon/dexterity-sheets.ico")
        .set("ProductName", &product)
        .set("FileDescription", &description)
        .set("CompanyName", "Dexterity Design Services Pvt. Ltd.")
        .set("LegalCopyright", "Copyright (c) Dexterity Design Services Pvt. Ltd. Based on GridCraft, copyright (c) the GridCraft authors, MIT OR Apache-2.0.")
        .set("OriginalFilename", &original)
        .set("InternalName", &exe)
        .set("ProductVersion", &version)
        .set("FileVersion", &version)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed)
        .set_version_info(winresource::VersionInfo::FILEVERSION, packed);
    if let Err(e) = res.compile() {
        eprintln!("embedding Windows resources failed: {e}");
        std::process::exit(1);
    }
}
