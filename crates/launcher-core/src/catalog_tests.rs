use super::*;
use crate::paths::AppPaths;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;

fn record() -> Value {
    json!({"id":"62d7e99d-fb4e-4167-8732-ce3a334d4a58","slug":"vanilla-actual","name":"Enriquecraft","description":"Versión española, sin cambios.","minecraft_version":"26.3","enabled":true,"restricted":false,"min_launcher_version":null,"icon_url":null,"banner_url":null,"sort_order":10,"updated_at":"2026-10-13T06:22:29Z","source_type":"vanilla","modrinth_project_id":null,"pack_version_id":null,"loader":null})
}

fn list(records: Vec<Value>) -> String {
    json!({"meta":{"count":records.len()},"data":records}).to_string()
}

fn temporary_store() -> (AppPaths, PrivateStore) {
    let paths =
        AppPaths::new(std::env::temp_dir().join(format!("panita-catalog-{}", Uuid::new_v4())))
            .unwrap_or_else(|_| panic!("paths"));
    let store = PrivateStore::open(&paths).unwrap_or_else(|_| panic!("store"));
    (paths, store)
}

fn server(replies: Vec<(u16, String, bool)>) -> (CatalogClient, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("loopback"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|_| panic!("nonblocking"));
    let base = format!(
        "http://{}",
        listener.local_addr().unwrap_or_else(|_| panic!("address"))
    );
    let task = std::thread::spawn(move || {
        for (status, body, conditional) in replies {
            let until = std::time::Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < until, "fixture accept timeout");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => panic!("accept"),
                }
            };
            stream
                .set_nonblocking(false)
                .unwrap_or_else(|_| panic!("blocking"));
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap_or_else(|_| panic!("timeout"));
            let mut bytes = Vec::new();
            let mut chunk = [0; 1024];
            while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).unwrap_or_else(|_| panic!("read"));
                assert!(count > 0 && bytes.len() < 16 * 1024);
                bytes.extend_from_slice(&chunk[..count]);
            }
            let request = String::from_utf8_lossy(&bytes).to_lowercase();
            assert!(!request.contains("authorization:"));
            assert_eq!(
                request.contains("if-none-match: w/\"fixture\""),
                conditional
            );
            let response = format!(
                "HTTP/1.1 {status} Fixture\r\nETag: W/\"fixture\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .unwrap_or_else(|_| panic!("write"));
        }
    });
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| panic!("client"));
    (CatalogClient { client, base }, task)
}

#[test]
fn catalog_validation_preserves_text_and_rejects_malformed_records() {
    let parsed = parse_list(list(vec![record()]).as_bytes()).unwrap_or_else(|_| panic!("catalog"));
    assert_eq!(parsed[0].description, "Versión española, sin cambios.");
    for (key, value) in [
        ("id", json!("bad")),
        ("enabled", json!(null)),
        ("minecraft_version", json!("../escape")),
        ("modrinth_project_id", json!("unexpected")),
        ("min_launcher_version", json!("1.0.0-beta.1")),
        ("icon_url", json!("http://unsafe.invalid/a")),
        ("sort_order", json!(-1)),
    ] {
        let mut bad = record();
        bad[key] = value;
        assert!(parse_list(list(vec![bad]).as_bytes()).is_err(), "{key}");
    }
    assert!(parse_list(list(vec![record(), record()]).as_bytes()).is_err());
    assert!(parse_list(br#"{"meta":{"count":1},"data":[]}"#).is_err());
    assert!(parse_list(&vec![b' '; MAX_BODY + 1]).is_err());
}

#[test]
fn catalog_sources_minimum_versions_and_disabled_states_are_not_installable() {
    let mut raw = record();
    raw["min_launcher_version"] = json!("0.2.0");
    let record = parse_list(list(vec![raw.clone()]).as_bytes())
        .unwrap_or_else(|_| panic!("catalog"))
        .remove(0);
    assert!(!record.compatible_with(&Version::new(0, 1, 99)));
    assert!(
        record
            .compatible_with(&Version::parse("0.2.0-beta.1").unwrap_or_else(|_| panic!("version")))
    );
    raw["enabled"] = json!(false);
    let disabled = parse_list(list(vec![raw.clone()]).as_bytes())
        .unwrap_or_else(|_| panic!("catalog"))
        .remove(0);
    assert!(
        matches!(disabled.installation_allowed(&Version::new(9,0,0)), Err(e) if e.code() == ErrorCode::InstanceDisabled)
    );
    raw["source_type"] = json!("future-loader");
    raw["enabled"] = json!(true);
    let unknown = parse_list(list(vec![raw.clone()]).as_bytes())
        .unwrap_or_else(|_| panic!("catalog"))
        .remove(0);
    assert!(matches!(unknown.source, CatalogSource::Unsupported { .. }));
    assert!(
        unknown
            .installation_allowed(&Version::new(9, 0, 0))
            .is_err()
    );
    raw["source_type"] = json!("modrinth");
    assert!(parse_list(list(vec![raw.clone()]).as_bytes()).is_err());
    raw["modrinth_project_id"] = json!("fixture");
    raw["pack_version_id"] = json!("pinned");
    raw["loader"] = json!("fabric");
    let pack = parse_list(list(vec![raw]).as_bytes())
        .unwrap_or_else(|_| panic!("catalog"))
        .remove(0);
    assert!(matches!(pack.source, CatalogSource::Modrinth { .. }));
    assert!(pack.installation_allowed(&Version::new(9, 0, 0)).is_err());
}

#[tokio::test]
async fn catalog_etag_revalidation_and_outage_preserve_disabled_cache() {
    let mut raw = record();
    raw["enabled"] = json!(false);
    let (client, task) = server(vec![
        (200, list(vec![raw]), false),
        (500, String::new(), true),
        (304, String::new(), true),
        (500, "private service detail".to_owned(), true),
    ]);
    let (paths, store) = temporary_store();
    let first = client
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("fresh"));
    assert!(!first.stale && !first.instances[0].enabled && first.cache_warning.is_none());
    // Simulate the pre-formatting cache so revalidation also proves migration.
    let bytes = store
        .read(CACHE_FILE)
        .unwrap_or_default()
        .unwrap_or_default();
    let mut legacy: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("cache"));
    legacy["schema_version"] = json!(1);
    legacy
        .as_object_mut()
        .unwrap_or_else(|| panic!("object"))
        .remove("source_fingerprint");
    legacy["representation"] = json!(client.representation());
    legacy["body"] = json!(legacy["body"].to_string());
    let legacy_bytes = serde_json::to_vec(&legacy).unwrap_or_default();
    assert!(
        store
            .transaction(LEGACY_CACHE_FILE, |_| Ok((legacy_bytes, ())))
            .is_ok()
    );
    assert!(std::fs::remove_file(paths.catalog_cache()).is_ok());
    let legacy_fallback = client
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("legacy fallback"));
    assert!(legacy_fallback.stale && !legacy_fallback.instances[0].enabled);
    assert!(!paths.catalog_cache().exists());
    let second = client
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("304"));
    assert_eq!(first.instances, second.instances);
    assert!(!second.stale);
    let upgraded: Value = serde_json::from_slice(
        &store
            .read(CACHE_FILE)
            .unwrap_or_default()
            .unwrap_or_default(),
    )
    .unwrap_or_else(|_| panic!("upgraded cache"));
    assert_eq!(upgraded["schema_version"], json!(3));
    assert_eq!(
        upgraded["source_fingerprint"],
        json!(source_fingerprint(&client.representation()))
    );
    assert!(upgraded.get("representation").is_none());
    assert!(!upgraded.to_string().contains(&client.base));
    assert!(upgraded["body"].is_object());
    // Once the new cache exists, an obsolete legacy file cannot affect fallback.
    assert!(
        store
            .transaction(LEGACY_CACHE_FILE, |_| Ok((b"corrupt".to_vec(), ())))
            .is_ok()
    );
    let stale = client
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("fallback"));
    assert!(stale.stale && !stale.instances[0].enabled);
    assert!(task.join().is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn catalog_cache_uses_legacy_name_only_when_new_name_is_absent() {
    let (paths, store) = temporary_store();
    assert_eq!(paths.catalog_cache(), paths.root().join("insts-cache.json"));
    assert!(
        store
            .transaction(LEGACY_CACHE_FILE, |_| Ok((b"legacy".to_vec(), ())))
            .is_ok()
    );
    assert_eq!(
        read_cache(&store).unwrap_or_default(),
        Some(b"legacy".to_vec())
    );
    assert!(
        store
            .transaction(PREVIOUS_CACHE_FILE, |_| Ok((b"previous".to_vec(), ())))
            .is_ok()
    );
    assert_eq!(
        read_cache(&store).unwrap_or_default(),
        Some(b"previous".to_vec())
    );
    assert!(
        store
            .transaction(CACHE_FILE, |_| Ok((b"corrupt".to_vec(), ())))
            .is_ok()
    );
    assert_eq!(
        read_cache(&store).unwrap_or_default(),
        Some(b"corrupt".to_vec())
    );
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn catalog_schema_two_url_binding_upgrades_to_a_fingerprint() {
    let key = "https://example.invalid/v1/instances|accept:application/json";
    let legacy = json!({
        "schema_version": 2,
        "representation": key,
        "etag": "\"test\"",
        "fetched_at": 1,
        "body": serde_json::from_str::<Value>(&list(vec![record()])).unwrap_or_else(|_| panic!("body")),
    });
    let bytes = serde_json::to_vec(&legacy).unwrap_or_default();
    let mut cache = CachedCatalog::decode(&bytes, key).unwrap_or_else(|_| panic!("legacy cache"));
    assert!(
        CachedCatalog::decode(
            &bytes,
            "https://other.invalid/v1/instances|accept:application/json"
        )
        .is_err()
    );
    cache.schema_version = 3;
    let upgraded = serde_json::to_vec(&cache).unwrap_or_default();
    assert!(!String::from_utf8_lossy(&upgraded).contains("https://example.invalid"));
    assert!(CachedCatalog::decode(&upgraded, key).is_ok());
    assert!(
        CachedCatalog::decode(
            &upgraded,
            "https://other.invalid/v1/instances|accept:application/json"
        )
        .is_err()
    );
}

#[tokio::test]
async fn catalog_invalid_responses_do_not_replace_last_valid_cache() {
    let (client, task) = server(vec![
        (200, list(vec![record()]), false),
        (200, "malformed".to_owned(), true),
        (200, String::new(), true),
    ]);
    let (paths, store) = temporary_store();
    assert!(client.refresh(&store).await.is_ok());
    let before = store.read(CACHE_FILE).unwrap_or_default();
    assert!(client.refresh(&store).await.is_err());
    assert_eq!(store.read(CACHE_FILE).unwrap_or_default(), before);
    assert!(client.refresh(&store).await.is_err());
    assert_eq!(store.read(CACHE_FILE).unwrap_or_default(), before);
    assert!(task.join().is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn catalog_cache_validator_is_bound_to_the_request_representation() {
    let (paths, store) = temporary_store();
    let (first, first_task) = server(vec![(200, list(vec![record()]), false)]);
    assert!(first.refresh(&store).await.is_ok());
    assert!(first_task.join().is_ok());
    let mut next = record();
    next["name"] = json!("Otra representación");
    let (second, second_task) = server(vec![(200, list(vec![next]), false)]);
    let refreshed = second
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("new representation"));
    assert_eq!(refreshed.instances[0].name, "Otra representación");
    assert!(second_task.join().is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn catalog_304_without_cache_and_corrupt_cache_fail_safely() {
    let (client, task) = server(vec![
        (304, String::new(), false),
        (500, String::new(), false),
    ]);
    let (paths, store) = temporary_store();
    assert!(client.refresh(&store).await.is_err());
    assert!(
        store
            .transaction(CACHE_FILE, |_| Ok((b"corrupt".to_vec(), ())))
            .is_ok()
    );
    assert!(client.refresh(&store).await.is_err());
    assert_eq!(
        store.read(CACHE_FILE).unwrap_or_default(),
        Some(b"corrupt".to_vec())
    );
    assert!(task.join().is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn catalog_detail_distinguishes_missing_disabled_and_identity_mismatch() {
    let mut raw = record();
    raw["enabled"] = json!(false);
    let (client, task) = server(vec![
        (404, String::new(), false),
        (200, json!({"data":raw}).to_string(), false),
        (200, json!({"data":record()}).to_string(), false),
    ]);
    let id =
        Uuid::parse_str("62d7e99d-fb4e-4167-8732-ce3a334d4a58").unwrap_or_else(|_| panic!("id"));
    assert!(matches!(client.detail(id).await,Err(e) if e.code() == ErrorCode::InstanceMissing));
    assert!(
        !client
            .detail(id)
            .await
            .unwrap_or_else(|_| panic!("disabled"))
            .enabled
    );
    assert!(client.detail(Uuid::new_v4()).await.is_err());
    assert!(task.join().is_ok());
}

#[tokio::test]
#[ignore = "explicit read-only live catalog integration"]
async fn live_public_catalog_includes_two_independent_vanilla_records() {
    let (paths, store) = temporary_store();
    let client = CatalogClient::new().unwrap_or_else(|_| panic!("client"));
    let catalog = client
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("live list"));
    let vanilla: Vec<_> = catalog
        .instances
        .iter()
        .filter(|record| matches!(record.source, CatalogSource::Vanilla { .. }))
        .collect();
    assert!(vanilla.len() >= 2);
    assert_ne!(vanilla[0].id, vanilla[1].id);
    for record in vanilla {
        assert_eq!(
            &client
                .detail(record.id)
                .await
                .unwrap_or_else(|_| panic!("detail")),
            record
        );
    }
    assert!(client.refresh(&store).await.is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}
