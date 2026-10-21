use std::{env, fs, path::PathBuf, process::Command};

struct TemporaryTarget(PathBuf);

impl TemporaryTarget {
    fn new() -> Self {
        let path = env::temp_dir().join(format!(
            "scafra-build-graph-compile-pass-{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary cargo target should be creatable");
        Self(path)
    }
}

impl Drop for TemporaryTarget {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn provider_contract_composes_through_the_public_application_graph() {
    let target = TemporaryTarget::new();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/provider_contract/Cargo.toml");
    let output = Command::new("cargo")
        .args(["test", "--offline", "--manifest-path"])
        .arg(manifest)
        .env("CARGO_TARGET_DIR", &target.0)
        .output()
        .expect("provider contract fixture should run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "provider contract fixture failed:\n{stderr}"
    );
}
