use super::*;

fn root() -> Result<AppPaths, CoreError> {
    let paths =
        AppPaths::new(std::env::temp_dir().join(format!("phase-9-validation-{}", Uuid::new_v4())))?;
    private_fs::PrivateDir::ensure(paths.root()).map_err(map_private_error)?;
    Ok(paths)
}

#[test]
fn normal_profile_roots_and_account_bearing_roots_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let normal = AppPaths::new(
        std::env::temp_dir().join(format!("panita-user-profile-{}", Uuid::new_v4())),
    )?;
    private_fs::PrivateDir::ensure(normal.root())?;
    assert!(guard(&normal).is_err());
    let paths = root()?;
    let directory = private_fs::PrivateDir::open(paths.root())?;
    directory.write_atomic_durable("accounts.json", b"synthetic sensitive sentinel")?;
    assert!(guard(&paths).is_err());
    assert!(!paths.root().join(TARGET).exists());
    assert_eq!(
        std::fs::read(paths.root().join("accounts.json"))?,
        b"synthetic sensitive sentinel"
    );
    Ok(())
}

#[test]
fn future_validation_markers_remain_preserved_and_never_authorize_launch()
-> Result<(), Box<dyn std::error::Error>> {
    let paths = root()?;
    let store = PrivateStore::open(&paths)?;
    let bytes = b"{\"schema\":900,\"type\":\"loader_validation\"}";
    store.transaction(TARGET, |_| Ok((bytes.to_vec(), ())))?;
    assert!(target(&paths).is_err());
    assert!(launch(&paths, &Cancellation::default()).is_err());
    assert_eq!(store.read(TARGET)?, Some(bytes.to_vec()));
    Ok(())
}

#[test]
fn synthetic_catalog_is_disabled_and_unsupported_by_player_gates() -> Result<(), CoreError> {
    let target = ValidationTarget {
        schema: 1,
        kind: "loader_validation".into(),
        id: Uuid::new_v4(),
        minecraft: "1.21.1".into(),
        neoforge: "21.1.248".into(),
    };
    let mut catalog = catalog(&target);
    catalog.validate()?;
    assert_eq!(
        catalog
            .installation_allowed(&semver::Version::new(99, 0, 0))
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::InstanceDisabled)
    );
    catalog.enabled = true;
    assert_eq!(
        catalog
            .installation_allowed(&semver::Version::new(99, 0, 0))
            .err()
            .map(|error| error.code()),
        Some(ErrorCode::UnsupportedInstance)
    );
    Ok(())
}
