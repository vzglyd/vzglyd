use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let slide_dir = manifest_dir.join("loading-slide");

    // Rebuild whenever loading-slide source or assets change.
    println!("cargo:rerun-if-changed=loading-slide/src");
    println!("cargo:rerun-if-changed=loading-slide/assets");
    println!("cargo:rerun-if-changed=loading-slide/manifest.json");
    println!("cargo:rerun-if-changed=loading-slide/Cargo.toml");

    // ── 1. Compile loading-slide to wasm32-wasip1 ─────────────────────────────
    let status = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args([
            "build",
            "--manifest-path",
            slide_dir.join("Cargo.toml").to_str().unwrap(),
            "--target",
            "wasm32-wasip1",
            "--release",
            "--quiet",
        ])
        .status()
        .expect("failed to invoke cargo for loading-slide");

    if !status.success() {
        panic!("loading-slide wasm build failed");
    }

    // Cargo places cross-compiled artifacts under the workspace target dir.
    let wasm_src = manifest_dir
        .join("target/wasm32-wasip1/release/loading_slide.wasm");

    if !wasm_src.exists() {
        panic!(
            "expected loading slide wasm at {} — build succeeded but file is missing",
            wasm_src.display()
        );
    }

    // ── 2. Pack loading.vzglyd (zip) ──────────────────────────────────────────
    let vzglyd_path = out_dir.join("loading.vzglyd");
    pack_vzglyd(&vzglyd_path, &wasm_src, &slide_dir)
        .expect("failed to pack loading.vzglyd");

    println!("cargo:rerun-if-changed=build.rs");
}

fn pack_vzglyd(
    out: &Path,
    wasm: &Path,
    slide_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::BufWriter;

    let file = fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(BufWriter::new(file));
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    // manifest.json
    zip.start_file("manifest.json", stored)?;
    zip.write_all(&fs::read(slide_dir.join("manifest.json"))?)?;

    // slide.wasm
    zip.start_file("slide.wasm", stored)?;
    zip.write_all(&fs::read(wasm)?)?;

    // assets/
    let assets_dir = slide_dir.join("assets");
    if assets_dir.is_dir() {
        for entry in fs::read_dir(&assets_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry.file_name();
                zip.start_file(
                    format!("assets/{}", name.to_string_lossy()),
                    stored,
                )?;
                zip.write_all(&fs::read(entry.path())?)?;
            }
        }
    }

    zip.finish()?;
    Ok(())
}
