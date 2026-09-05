fn main() {
    if std::path::Path::new("assets/icon.ico").exists() {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Rustic Backup");
        res.set("FileDescription", "Rustic 备份管理工具");
        res.set("CompanyName", "rustic-backup");
        res.set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=winres compile failed: {}", e);
        }
    } else {
        eprintln!("cargo:warning=icon.ico not found, skipping icon embedding");
    }
}
