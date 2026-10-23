// Gives the Windows exe its icon and version info (what Explorer shows under Properties > Details).
// Needs a resource compiler: rc.exe with the MSVC toolchain, windres with MinGW. Without one the
// build still works, the exe just has the generic icon.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u32> = version.split(['.', '-']).filter_map(|p| p.parse().ok()).collect();
    parts.resize(4, 0);
    let numeric = parts.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let icon = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("icon.ico");
    let rc = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "BeForce1"
      VALUE "FileDescription", "Don't Forget"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "DontForget"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "DontForget.exe"
      VALUE "ProductName", "Don't Forget"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/"),
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app.rc");
    fs::write(&out, rc).unwrap();
    use embed_resource::CompilationResult as R;
    match embed_resource::compile(&out, embed_resource::NONE) {
        R::NotAttempted(why) | R::Failed(why) => println!("cargo:warning=exe icon and version info skipped: {why}"),
        R::NotWindows | R::Ok => {}
    }
}
