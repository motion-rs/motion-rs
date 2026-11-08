//! Build script that enables the benchmark engine cfg and fingerprints benchmarked sources.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(directory).expect("benchmark source directory") {
        let entry = entry.expect("benchmark source entry");
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some("target" | "reference" | "archive")
            ) {
                collect(root, &path, files);
            }
        } else if entry.file_type().unwrap().is_file()
            && (path.starts_with(root.join("gallery/assets"))
                || path.file_name().is_some_and(|name| name == "Cargo.toml")
                || matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("rs" | "wgsl" | "h" | "cjs")
                ))
        {
            files.insert(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                fingerprint(&fs::read(path).unwrap()),
            );
        }
    }
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(motion_bench_engine)");
    println!("cargo:rustc-cfg=motion_bench_engine");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = if manifest.join("../../crates/motion").is_dir() {
        manifest.join("../..")
    } else {
        manifest.join("..")
    }
    .canonicalize()
    .unwrap();
    let mut files = BTreeMap::new();
    for folder in ["crates", "benches", "gallery/src", "gallery/assets"] {
        println!("cargo:rerun-if-changed={}", root.join(folder).display());
        collect(&root, &root.join(folder), &mut files);
    }
    for file in ["Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", root.join(file).display());
        files.insert(
            file.into(),
            fingerprint(&fs::read(root.join(file)).unwrap()),
        );
    }
    for file in files.keys() {
        println!("cargo:rerun-if-changed={}", root.join(file).display());
    }
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    let compiler = Command::new(env::var_os("RUSTC").unwrap())
        .arg("-Vv")
        .output()
        .expect("rustc identity");
    assert!(compiler.status.success());
    let mut generated = format!(
        "pub const RUSTC: &str = {:?};\n",
        String::from_utf8(compiler.stdout).unwrap().trim()
    );
    for (name, variable) in [
        ("TARGET", "TARGET"),
        ("PROFILE", "PROFILE"),
        ("OPT_LEVEL", "OPT_LEVEL"),
        ("DEBUG", "DEBUG"),
        ("FLAGS", "CARGO_ENCODED_RUSTFLAGS"),
    ] {
        generated.push_str(&format!(
            "pub const {name}: &str = {:?};\n",
            env::var(variable).unwrap_or_default()
        ));
    }
    let mut features = env::vars()
        .filter_map(|(name, value)| {
            name.strip_prefix("CARGO_FEATURE_")
                .filter(|_| value == "1")
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    features.sort();
    generated.push_str(&format!("pub const FEATURES: &[&str] = &{:?};\n", features));
    generated.push_str("pub const SOURCE_FILES: &[(&str,&str)] = &[\n");
    for (name, hash) in files {
        generated.push_str(&format!("({name:?},{hash:?}),\n"));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("motion_benchmark_build.rs"),
        generated,
    )
    .unwrap();
}
