use std::process::Command;
use std::{fs, time::SystemTime};

fn main() {
    // Turn icon.png into icon.nwi
    println!("cargo:rerun-if-changed=src/icon.png");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    let source_modified = fs::metadata("src/icon.png")
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::now());
    let output_modified = fs::metadata("target/icon.nwi")
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if output_modified >= source_modified {
        return;
    }
    let output = Command::new("nwlink")
        .args(&["png-nwi", "src/icon.png", "target/icon.nwi"])
        .output()
        .expect("Failure to launch process");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
