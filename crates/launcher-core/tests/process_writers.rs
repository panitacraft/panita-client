//! Exercise the filesystem mutation lock across actual OS processes.
use launcher_core::paths::AppPaths;
use launcher_core::storage::PrivateStore;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[test]
fn writer_child() {
    let Some(root) = std::env::var_os("PANITA_WRITER_TEST_ROOT") else {
        return;
    };
    let paths = AppPaths::new(root).unwrap_or_else(|_| panic!("absolute synthetic test root"));
    let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("protected test store"));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !paths.root().join("start").exists() {
        assert!(Instant::now() < deadline, "parent start signal timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..25 {
        store
            .transaction("writer-counter.json", |previous| {
                let value: u32 = serde_json::from_slice(
                    &previous.unwrap_or_else(|| panic!("initialized counter")),
                )
                .unwrap_or_else(|_| panic!("complete counter document"));
                // Widen the race window so an unlocked read/modify/write loses increments.
                std::thread::sleep(Duration::from_millis(5));
                Ok((
                    serde_json::to_vec(&(value + 1)).unwrap_or_else(|_| panic!("counter JSON")),
                    (),
                ))
            })
            .unwrap_or_else(|_| panic!("serialized durable write"));
    }
}

#[test]
fn separate_processes_preserve_every_mutation() {
    let root = std::env::temp_dir().join(format!("panita-writers-{}", Uuid::new_v4()));
    let paths = AppPaths::new(&root).unwrap_or_else(|_| panic!("absolute temporary root"));
    let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("protected test store"));
    store
        .transaction("writer-counter.json", |_| Ok((b"0".to_vec(), ())))
        .unwrap_or_else(|_| panic!("initialize counter"));
    let executable = std::env::current_exe().unwrap_or_else(|_| panic!("test executable"));
    let mut children = Vec::new();
    for _ in 0..2 {
        children.push(
            Command::new(&executable)
                .args(["--exact", "writer_child", "--nocapture"])
                .env("PANITA_WRITER_TEST_ROOT", &root)
                .stdin(Stdio::null())
                .spawn()
                .unwrap_or_else(|_| panic!("spawn independent writer")),
        );
    }
    std::fs::write(root.join("start"), b"").unwrap_or_else(|_| panic!("release both writers"));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut failed = false;
    for child in &mut children {
        loop {
            if let Some(status) = child.try_wait().unwrap_or_else(|_| panic!("writer status")) {
                failed |= !status.success();
                break;
            }
            if Instant::now() >= deadline {
                child
                    .kill()
                    .unwrap_or_else(|_| panic!("stop timed out writer"));
                child
                    .wait()
                    .unwrap_or_else(|_| panic!("reap timed out writer"));
                failed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let bytes = store
        .read("writer-counter.json")
        .unwrap_or_else(|_| panic!("read counter"));
    drop(store);
    std::fs::remove_dir_all(&root).unwrap_or_else(|_| panic!("clean synthetic temporary root"));
    assert!(!failed, "both writer processes must finish successfully");
    assert_eq!(bytes.as_deref(), Some(b"50".as_slice()));
}
