use super::*;
use crate::paths::AppPaths;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;

pub(crate) fn record() -> Value {
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

pub(crate) fn server(
    replies: Vec<(u16, String, bool)>,
) -> (CatalogClient, std::thread::JoinHandle<()>) {
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
    (
        CatalogClient {
            client,
            base,
            auth: None,
        },
        task,
    )
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
fn custom_catalog_pins_round_trip_and_only_published_neoforge_packs_install() {
    let mut raw = record();
    raw["source_type"] = json!("custom");
    raw["loader"] = json!("neoforge");
    raw["current_version_id"] = json!("04ac0877-11a5-4de2-9a41-a41dbb44f1ee");
    raw["current_version_label"] = Value::Null;
    raw["current_version_published_at"] = json!("2026-10-25T20:54:25.192Z");
    let parsed =
        parse_list(list(vec![raw.clone()]).as_bytes()).unwrap_or_else(|_| panic!("custom catalog"));
    assert!(matches!(
        parsed[0].source,
        CatalogSource::Custom { label: None, .. }
    ));
    assert!(parsed[0].validate().is_ok());
    // A published NeoForge pack is installable; every other loader stays unsupported.
    assert!(
        parsed[0]
            .installation_allowed(&Version::new(0, 2, 0))
            .is_ok()
    );
    assert!(parsed[0].source.installable());
    for loader in ["fabric", "forge", "quilt"] {
        let mut other = raw.clone();
        other["loader"] = json!(loader);
        let parsed = parse_list(list(vec![other]).as_bytes()).unwrap_or_else(|_| panic!("loader"));
        assert_eq!(
            parsed[0]
                .installation_allowed(&Version::new(0, 2, 0))
                .err()
                .map(|error| error.code()),
            Some(ErrorCode::UnsupportedInstance),
            "{loader}"
        );
    }
    let encoded = serde_json::to_vec(&parsed[0]).unwrap_or_else(|_| panic!("encode"));
    let restored: CatalogInstance =
        serde_json::from_slice(&encoded).unwrap_or_else(|_| panic!("decode"));
    assert_eq!(restored, parsed[0]);
    assert!(restored.validate().is_ok());
    for (field, value) in [
        ("minecraft_version", Value::Null),
        ("loader", Value::Null),
        ("pack_version_id", json!("unexpected")),
        ("current_version_id", Value::Null),
        ("current_version_id", json!(Uuid::nil())),
        ("current_version_label", json!("x".repeat(33))),
        ("current_version_published_at", json!("invalid")),
    ] {
        let mut bad = raw.clone();
        bad[field] = value;
        assert!(parse_list(list(vec![bad]).as_bytes()).is_err(), "{field}");
    }
    raw["enabled"] = json!(false);
    raw["current_version_id"] = Value::Null;
    raw["current_version_published_at"] = Value::Null;
    assert!(parse_list(list(vec![raw.clone()]).as_bytes()).is_ok());
    raw["enabled"] = json!(true);
    assert!(parse_list(list(vec![raw]).as_bytes()).is_err());
}

#[test]
fn future_loader_is_retained_but_never_installed_as_vanilla() {
    let mut raw = record();
    raw["source_type"] = json!("modrinth");
    raw["modrinth_project_id"] = json!("project");
    raw["pack_version_id"] = json!("pinned");
    raw["loader"] = json!("future-loader");
    let parsed = parse_list(list(vec![raw]).as_bytes()).unwrap_or_else(|_| panic!("catalog"));
    assert!(parsed[0].validate().is_ok());
    assert!(
        parsed[0]
            .installation_allowed(&Version::new(0, 2, 0))
            .is_err()
    );
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
    let pack = parse_list(list(vec![raw.clone()]).as_bytes())
        .unwrap_or_else(|_| panic!("catalog"))
        .remove(0);
    assert!(matches!(pack.source, CatalogSource::Modrinth { .. }));
    assert!(pack.installation_allowed(&Version::new(9, 0, 0)).is_err());
    // Pinned NeoForge packs install; a missing loader leaves the decision to the index.
    for loader in [json!("neoforge"), Value::Null] {
        let mut raw = raw.clone();
        raw["loader"] = loader;
        let pack = parse_list(list(vec![raw]).as_bytes())
            .unwrap_or_else(|_| panic!("catalog"))
            .remove(0);
        assert!(pack.installation_allowed(&Version::new(9, 0, 0)).is_ok());
    }
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
        .unwrap_or_else(|| panic!("cache object"))
        .remove("schema");
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
    assert_eq!(upgraded["schema"], json!(3));
    assert!(upgraded.get("schema_version").is_none());
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

/// A catalog client for a signed-in member, served by the activation fixture.
fn member_server(
    replies: Vec<crate::panita_api::tests::Canned>,
    identity: u64,
) -> (CatalogClient, std::thread::JoinHandle<Vec<String>>) {
    use crate::panita_api::tests as panita;
    let (api, task) = panita::server(replies);
    let (client, base) = api.test_transport();
    let mut user = crate::panita_session::tests::user();
    user.id = identity;
    let session =
        crate::panita_session::PanitaSession::new(format!("ps_member-{identity}-token"), user, 1)
            .unwrap_or_else(|_| panic!("session"));
    (
        CatalogClient {
            client,
            base,
            auth: Some(panita::credentials().with_session(&session)),
        },
        task,
    )
}

fn restricted_record() -> Value {
    let mut raw = record();
    raw["restricted"] = json!(true);
    raw
}

const RECORD_ID: &str = "62d7e99d-fb4e-4167-8732-ce3a334d4a58";

#[tokio::test]
async fn member_requests_carry_the_key_and_session_while_anonymous_ones_carry_neither() {
    use crate::panita_api::tests::canned;
    let (client, task) = member_server(
        vec![
            canned(200, list(vec![restricted_record()])),
            canned(200, json!({"data":restricted_record()}).to_string()),
        ],
        42,
    );
    let (paths, store) = temporary_store();
    let catalog = client
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("member list"));
    assert!(catalog.instances[0].restricted);
    let id = Uuid::parse_str(RECORD_ID).unwrap_or_else(|_| panic!("id"));
    assert!(client.detail(id).await.is_ok());
    let seen = task.join().unwrap_or_else(|_| panic!("fixture"));
    assert!(seen[0].starts_with("get /v1/instances http/1.1"));
    assert!(seen[1].starts_with(&format!("get /v1/instances/{RECORD_ID} http/1.1")));
    for request in &seen {
        assert!(request.contains("x-api-key: test-key"));
        assert!(request.contains("authorization: bearer ps_member-42-token"));
    }
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());

    // The public fixture asserts that no anonymous request carries a bearer token.
    let (anonymous, task) = server(vec![(200, list(vec![record()]), false)]);
    let (paths, store) = temporary_store();
    assert!(anonymous.refresh(&store).await.is_ok());
    assert!(task.join().is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn unauthorized_answers_are_never_an_outage_and_keep_the_cache() {
    use crate::panita_api::tests::{canned, failure_json};
    let (client, task) = member_server(
        vec![
            canned(200, list(vec![restricted_record()])),
            canned(401, failure_json("invalid_token")),
            canned(401, failure_json("invalid_key")),
            canned(403, String::new()),
            canned(401, "<html>proxy</html>".to_owned()),
            canned(401, failure_json("invalid_token")),
            canned(401, failure_json("invalid_key")),
            canned(403, String::new()),
        ],
        42,
    );
    let (paths, store) = temporary_store();
    assert!(client.refresh(&store).await.is_ok());
    let before = store.read(CACHE_FILE).unwrap_or_default();
    for expected in [
        ErrorCode::SessionRejected,
        ErrorCode::ApiKeyRejected,
        ErrorCode::ApiKeyRejected,
        ErrorCode::InvalidData,
    ] {
        // A cached copy exists, yet none of these may fall back to it as stale.
        let error = client.refresh(&store).await.err();
        assert_eq!(error.map(|e| e.code()), Some(expected));
    }
    assert_eq!(store.read(CACHE_FILE).unwrap_or_default(), before);
    let id = Uuid::parse_str(RECORD_ID).unwrap_or_else(|_| panic!("id"));
    for expected in [
        ErrorCode::SessionRejected,
        ErrorCode::ApiKeyRejected,
        ErrorCode::ApiKeyRejected,
    ] {
        assert_eq!(
            client.detail(id).await.err().map(|e| e.code()),
            Some(expected)
        );
    }
    assert!(task.join().is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[tokio::test]
async fn a_cached_member_catalog_is_never_served_to_another_identity() {
    use crate::panita_api::tests::canned;
    let (paths, store) = temporary_store();
    let (first, task) = member_server(
        vec![
            canned(200, list(vec![restricted_record()])),
            canned(503, String::new()),
            canned(503, String::new()),
            canned(503, String::new()),
        ],
        42,
    );
    assert!(first.refresh(&store).await.is_ok());
    // The cache fingerprint includes the base URL, so every client shares this server.
    let session = |identity: u64| {
        let mut user = crate::panita_session::tests::user();
        user.id = identity;
        crate::panita_session::PanitaSession::new(format!("ps_member-{identity}-token"), user, 1)
            .unwrap_or_else(|_| panic!("session"))
    };
    let sharing = |auth: Option<PanitaCredentials>| CatalogClient {
        client: first.client.clone(),
        base: first.base.clone(),
        auth,
    };
    // The same member can still read their own cache during an outage.
    let own = sharing(Some(
        crate::panita_api::tests::credentials().with_session(&session(42)),
    ));
    let stale = own
        .refresh(&store)
        .await
        .unwrap_or_else(|_| panic!("own stale cache"));
    assert!(stale.stale && stale.instances[0].restricted);
    // Another member, or an anonymous launcher, gets nothing from it.
    let other = sharing(Some(
        crate::panita_api::tests::credentials().with_session(&session(43)),
    ));
    assert!(other.refresh(&store).await.is_err());
    assert!(sharing(None).refresh(&store).await.is_err());
    assert!(task.join().is_ok());
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
}

#[test]
fn a_restricted_entry_is_installable_because_only_members_receive_it() {
    let client = Version::new(9, 0, 0);
    let parsed = parse_list(list(vec![restricted_record()]).as_bytes())
        .unwrap_or_else(|_| panic!("restricted catalog"));
    assert!(parsed[0].restricted);
    assert!(parsed[0].installation_allowed(&client).is_ok());
    // Restriction never widens what else is blocked.
    let mut disabled = restricted_record();
    disabled["enabled"] = json!(false);
    let mut future = restricted_record();
    future["source_type"] = json!("something_new");
    for raw in [disabled, future] {
        let parsed = parse_list(list(vec![raw]).as_bytes()).unwrap_or_else(|_| panic!("catalog"));
        assert!(parsed[0].installation_allowed(&client).is_err());
    }
}

#[test]
fn purging_removes_every_cache_spelling_and_tolerates_absence() {
    let (paths, store) = temporary_store();
    assert!(purge_cache(&store).is_ok());
    for name in [CACHE_FILE, PREVIOUS_CACHE_FILE, LEGACY_CACHE_FILE] {
        assert!(
            store
                .transaction(name, |_| Ok((b"{}".to_vec(), ())))
                .is_ok()
        );
    }
    assert!(purge_cache(&store).is_ok());
    for name in [CACHE_FILE, PREVIOUS_CACHE_FILE, LEGACY_CACHE_FILE] {
        assert!(!paths.root().join(name).exists());
    }
    drop(store);
    assert!(std::fs::remove_dir_all(paths.root()).is_ok());
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
