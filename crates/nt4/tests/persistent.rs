//! Persistent file cases ported from ntcore's NetworkServerTest, and tests
//! of saving.

// Ported from allwpilib ntcore/src/test/native/cpp/NetworkServerTest.cpp
// at commit 4ccfe967ba1903b1f409b04b99db0922dd3b7318.
// Copyright (c) FIRST and other WPILib contributors.
// The test cases come from that file and stay under the WPILib BSD license;
// see tests/LICENSE-WPILIB.md. The Rust test code was written by Claude.
//
// Upstream subscribes on the instance before StartServer so the server's
// own client sees the loaded topic; here `Server::subscribe` after start
// gets the loaded value straight away.

#![cfg(feature = "server")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use nt4::{NtType, Properties, Server, ServerConfig, SubscriptionOptions, Value};

/// Valid persistent JSON containing a single persistent integer topic.
const PERSISTENT_JSON: &str = r#"[
  {
    "name": "/test/persistent_value",
    "type": "int",
    "value": 42,
    "properties": {"persistent": true}
  }
]"#;

/// A temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("nt4_test_{tag}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("test_persistent.json")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

async fn start(file: &Path) -> Server {
    Server::start(ServerConfig {
        listen_address: "127.0.0.1".parse().unwrap(),
        port: 0,
        persist_file: Some(file.to_owned()),
    })
    .await
    .unwrap()
}

fn persistent_value(server: &Server) -> Option<i64> {
    server
        .subscribe::<i64>("/test/persistent_value", SubscriptionOptions::default())
        .get()
}

// upstream: "NetworkServerPersistentTest LoadPersistentRestoresFromBackupWhenOriginalMissing"
#[tokio::test]
async fn load_persistent_restores_from_backup_when_original_missing() {
    // Set up "interrupted" state: only .bck file exists, no original.
    let dir = TempDir::new("restore");
    let file = dir.file();
    let backup = with_suffix(&file, ".bck");
    std::fs::write(&backup, PERSISTENT_JSON).unwrap();
    assert!(!file.exists());

    let server = start(&file).await;
    assert_eq!(persistent_value(&server), Some(42));
    // The .bck should have been renamed to the original filename.
    assert!(file.exists());
}

// upstream: "NetworkServerPersistentTest LoadPersistentNormalLoad"
#[tokio::test]
async fn load_persistent_normal_load() {
    let dir = TempDir::new("normal");
    let file = dir.file();
    std::fs::write(&file, PERSISTENT_JSON).unwrap();

    let server = start(&file).await;
    assert_eq!(persistent_value(&server), Some(42));
    assert!(server.persistent_load_errors().is_empty());
}

// upstream: "NetworkServerPersistentTest LoadPersistentPrefersOriginalOverBackup"
#[tokio::test]
async fn load_persistent_prefers_original_over_backup() {
    let dir = TempDir::new("prefer");
    let file = dir.file();
    std::fs::write(&file, PERSISTENT_JSON.replace("42", "100")).unwrap();
    std::fs::write(with_suffix(&file, ".bck"), PERSISTENT_JSON).unwrap();

    let server = start(&file).await;
    // The value should come from the original (100), not the backup (42).
    assert_eq!(persistent_value(&server), Some(100));
}

// upstream: "NetworkServerPersistentTest LoadPersistentNoFile"
#[tokio::test]
async fn load_persistent_no_file() {
    let dir = TempDir::new("nofile");
    let file = dir.file();
    let server = start(&file).await;
    assert_eq!(persistent_value(&server), None);
    assert!(server.persistent_load_errors().is_empty());
}

// The rest of this file was written by Claude, not ported.

#[tokio::test]
async fn missing_file_is_created_empty() {
    let dir = TempDir::new("create");
    let file = dir.file();
    let _server = start(&file).await;
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.trim_end(), "[]");
}

#[tokio::test]
async fn bad_entries_are_reported_and_skipped() {
    let dir = TempDir::new("bad");
    let file = dir.file();
    std::fs::write(
        &file,
        r#"[{"name": "/a", "type": "int", "value": "x", "properties": {"persistent": true}},
            {"name": "/test/persistent_value", "type": "int", "value": 7, "properties": {"persistent": true}}]"#,
    )
    .unwrap();
    let server = start(&file).await;
    assert_eq!(
        server.persistent_load_errors(),
        ["0: value type mismatch, expected int"]
    );
    assert_eq!(persistent_value(&server), Some(7));
}

fn persistent() -> Properties {
    let mut p = Properties::new();
    p.insert("persistent".into(), true.into());
    p
}

#[tokio::test]
async fn changes_are_saved_within_a_second_and_load_again() {
    let dir = TempDir::new("save");
    let file = dir.file();
    {
        let server = start(&file).await;
        let p = server.publish_generic("/gain", NtType::Double, persistent());
        p.set(Value::Double(0.75)).unwrap();
        let mut text = String::new();
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            text = std::fs::read_to_string(&file).unwrap_or_default();
            if text.contains("0.75") {
                break;
            }
        }
        assert!(text.contains(r#""value": 0.75"#), "{text}");
        // ntcore keeps the previous file as .bck.
        assert!(with_suffix(&file, ".bck").exists());
        // Unpublished, the topic stays (persistent topics outlive their
        // publishers).
        drop(p);
    }
    let server = start(&file).await;
    let gain = server.subscribe::<f64>("/gain", SubscriptionOptions::default());
    assert_eq!(gain.get(), Some(0.75));
}

#[tokio::test]
async fn dropping_the_server_saves_the_last_change() {
    let dir = TempDir::new("drop");
    let file = dir.file();
    {
        let server = start(&file).await;
        let p = server.publish_generic("/last", NtType::Int, persistent());
        p.set(Value::Int(5)).unwrap();
        // Dropped well before the 1 s save timer.
    }
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(r#""value": 5"#), "{text}");
}

#[tokio::test]
async fn an_unreadable_file_is_reported_and_the_server_still_starts() {
    let dir = TempDir::new("unreadable");
    // A directory where the file should be: it exists but can't be read.
    let file = dir.file();
    std::fs::create_dir(&file).unwrap();
    let server = start(&file).await;
    let errors = server.persistent_load_errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].starts_with("could not open persistent file '"),
        "{errors:?}"
    );
    assert_eq!(persistent_value(&server), None);
}

#[tokio::test]
async fn a_failed_save_is_reported_as_a_warning() {
    let dir = TempDir::new("unwritable");
    let file = dir.file();
    let server = start(&file).await;
    let mut events = server.events();
    // Take the directory away so the save can't write its temporary file.
    std::fs::remove_dir_all(&dir.0).unwrap();
    let p = server.publish_generic("/gain", NtType::Double, persistent());
    p.set(Value::Double(0.5)).unwrap();

    let warning = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(nt4::ServerEvent::Warning(w)) => return w,
                Some(_) => {}
                None => panic!("events ended"),
            }
        }
    })
    .await
    .expect("no warning within 5 s");
    assert!(
        warning.starts_with("could not write persistent file '"),
        "{warning}"
    );
}
