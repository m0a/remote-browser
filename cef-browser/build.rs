use std::{env, fs, path::Path};

fn main() {
    // Get CEF directory from wew's build.rs via `links = "wew"`
    let cef_dir = match env::var("DEP_WEW_CEF_DIR") {
        Ok(dir) => dir,
        Err(_) => {
            eprintln!("Warning: DEP_WEW_CEF_DIR not set, skipping CEF file bundling");
            return;
        }
    };

    // Determine the target profile directory (where the binary will be placed)
    // OUT_DIR = target/<profile>/build/cef-browser-<hash>/out/
    let out_dir = env::var("OUT_DIR").unwrap();
    let target_dir = Path::new(&out_dir)
        .ancestors()
        .nth(3) // out/ → cef-browser-<hash>/ → build/ → <profile>/
        .expect("Could not determine target directory");

    let cef_path = Path::new(&cef_dir);
    let release_dir = cef_path.join("Release");
    let resources_dir = cef_path.join("Resources");

    // Copy shared libraries (.so*)
    copy_files_with_ext(&release_dir, target_dir, &["so"]);
    // Copy .bin files (GPU shader cache, etc.)
    copy_files_with_ext(&release_dir, target_dir, &["bin"]);

    // Copy resource files (.pak, .dat)
    copy_files_with_ext(&resources_dir, target_dir, &["pak", "dat"]);

    // Copy locales directory
    let src_locales = resources_dir.join("locales");
    let dst_locales = target_dir.join("locales");
    if src_locales.exists() {
        fs::create_dir_all(&dst_locales).ok();
        if let Ok(entries) = fs::read_dir(&src_locales) {
            for entry in entries.flatten() {
                let src = entry.path();
                if src.extension().is_some_and(|e| e == "pak") {
                    let dst = dst_locales.join(entry.file_name());
                    copy_if_newer(&src, &dst);
                }
            }
        }
    }

    // Copy public/ directory next to binary for standalone use
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_public = Path::new(&manifest_dir).join("public");
    let dst_public = target_dir.join("public");
    if src_public.exists() {
        fs::create_dir_all(&dst_public).ok();
        if let Ok(entries) = fs::read_dir(&src_public) {
            for entry in entries.flatten() {
                let src = entry.path();
                if src.is_file() {
                    let dst = dst_public.join(entry.file_name());
                    copy_if_newer(&src, &dst);
                }
            }
        }
    }

    // Set rpath=$ORIGIN so the binary finds .so files in its own directory
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");

    println!("cargo:rerun-if-changed=public");
    eprintln!("CEF runtime files bundled to {:?}", target_dir);
}

fn copy_files_with_ext(src_dir: &Path, dst_dir: &Path, extensions: &[&str]) {
    let Ok(entries) = fs::read_dir(src_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let src = entry.path();
        let matches = if let Some(name) = src.file_name().and_then(|n| n.to_str()) {
            extensions.iter().any(|ext| {
                name.ends_with(&format!(".{}", ext))
            })
        } else {
            false
        };
        if matches && src.is_file() {
            let dst = dst_dir.join(entry.file_name());
            copy_if_newer(&src, &dst);
        }
    }
}

fn copy_if_newer(src: &Path, dst: &Path) {
    let should_copy = match (fs::metadata(src), fs::metadata(dst)) {
        (Ok(src_meta), Ok(dst_meta)) => {
            src_meta.modified().ok() > dst_meta.modified().ok()
        }
        (Ok(_), Err(_)) => true,
        _ => false,
    };
    if should_copy {
        fs::copy(src, dst).ok();
    }
}
