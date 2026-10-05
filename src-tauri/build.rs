use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf};
fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .canonicalize()
        .unwrap();
    let mut entries = String::from("pub static EMBEDDED: &[(&str,&str,bool,&[u8])] = &[\n");
    let catalog_path = root.join("assets/models.json");
    println!("cargo:rerun-if-changed={}", catalog_path.display());
    let catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(catalog_path).unwrap()).unwrap();
    for pack in catalog
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["distribution"] == "bundled")
    {
        for file in pack["files"].as_array().unwrap() {
            let name = format!(
                "models/{}/{}",
                pack["id"].as_str().unwrap(),
                file["name"].as_str().unwrap()
            );
            let path = root.join("assets").join(&name);
            println!("cargo:rerun-if-changed={}", path.display());
            let bytes = fs::read(&path).expect(
                "Bundled detector is missing; run scripts/bootstrap.py --only bundled-models",
            );
            let hash = hex::encode(Sha256::digest(&bytes));
            assert_eq!(
                hash,
                file["sha256"].as_str().unwrap(),
                "Bundled model checksum mismatch: {}",
                path.display()
            );
            assert_eq!(
                bytes.len() as u64,
                file["bytes"].as_u64().unwrap(),
                "Bundled model size mismatch"
            );
            let compressed = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(format!(
                "{}-{}.gz",
                pack["id"].as_str().unwrap(),
                file["name"].as_str().unwrap()
            ));
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            encoder.write_all(&bytes).unwrap();
            fs::write(&compressed, encoder.finish().unwrap()).unwrap();
            entries.push_str(&format!(
                "({name:?},{hash:?},true,include_bytes!({compressed:?})),\n"
            ));
        }
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("assets/runtime-manifest.json"))
            .expect("Run scripts/bootstrap.py first"),
    )
    .unwrap();
    println!(
        "cargo:rerun-if-changed={}",
        root.join("assets/runtime-manifest.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join("assets/licenses").display()
    );
    for record in manifest.as_array().unwrap() {
        let name = record["name"].as_str().unwrap();
        let path = root.join(format!("assets/runtime/{name}.gz"));
        println!("cargo:rerun-if-changed={}", path.display());
        entries.push_str(&format!(
            "({:?},{:?},true,include_bytes!({:?})),\n",
            format!("runtime/{name}"),
            record["sha256"].as_str().unwrap(),
            path
        ));
    }
    for dir in ["fonts", "licenses"] {
        for entry in fs::read_dir(root.join("assets").join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if !path.is_file() {
                continue;
            }
            let hash = hex::encode(Sha256::digest(fs::read(&path).unwrap()));
            println!("cargo:rerun-if-changed={}", path.display());
            entries.push_str(&format!(
                "({:?},{:?},false,include_bytes!({:?})),\n",
                format!("{dir}/{}", path.file_name().unwrap().to_str().unwrap()),
                hash,
                path
            ));
        }
    }
    entries.push_str("];\n");
    fs::write(
        PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded.rs"),
        entries,
    )
    .unwrap();
    tauri_build::build()
}
