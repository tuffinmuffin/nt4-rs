//! Builds the ntcore oracle (oracle/oracle.cpp) when `NT4_NTCORE_PYTHON`
//! names a Python with pyntcore installed, and links it to that wheel's
//! libntcore. Without it, nt4-fuzz builds with only its own checks.
//!
//! Linux only for now: the wheel's libraries are found by path and loaded
//! through an rpath.
//!
//! Written by Claude.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_ntcore)");
    println!("cargo::rerun-if-env-changed=NT4_NTCORE_PYTHON");
    println!("cargo::rerun-if-changed=oracle");
    let Ok(python) = std::env::var("NT4_NTCORE_PYTHON") else {
        return;
    };
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        println!("cargo::warning=the ntcore oracle is only built on Linux");
        return;
    }
    // pyntcore keeps the C++ libraries and headers in site-packages/native.
    let out = Command::new(&python)
        .args([
            "-I",
            "-c",
            "import ntcore, pathlib; print(pathlib.Path(ntcore.__file__).parent.parent / 'native')",
        ])
        .output()
        .expect("run NT4_NTCORE_PYTHON");
    assert!(
        out.status.success(),
        "NT4_NTCORE_PYTHON can't import ntcore: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let native = PathBuf::from(String::from_utf8(out.stdout).unwrap().trim());
    let libs = ["ntcore", "wpinet", "wpiutil"];
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .file("oracle/oracle.cpp")
        .include("oracle/ntcore-v2026.2.2")
        .warnings(false);
    for lib in libs {
        build.include(native.join(lib).join("include"));
    }
    build.compile("nt4_oracle");

    let dirs: Vec<String> = libs
        .iter()
        .map(|l| native.join(l).join("lib").display().to_string())
        .collect();
    for d in &dirs {
        println!("cargo::rustc-link-search=native={d}");
    }
    // libntcore finds wpiutil and wpinet through its own runpath, which
    // doesn't hold outside the wheel's layout; load them first instead.
    println!("cargo::rustc-link-arg=-Wl,--no-as-needed");
    for lib in ["wpiutil", "wpinet", "ntcore"] {
        println!("cargo::rustc-link-arg=-l{lib}");
    }
    println!(
        "cargo::rustc-link-arg=-Wl,--disable-new-dtags,-rpath,{}",
        dirs.join(":")
    );
    println!("cargo::rustc-cfg=has_ntcore");
}
