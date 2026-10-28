use super::*;
use crate::minecraft::plan::{
    InstallPlan, PlannedArtifact, asset_index_cache_path, library_cache_path,
};
use crate::minecraft::{Artifact, VersionMetadata};
use uuid::Uuid;

fn fixture(id: &str, os: &str) -> (AppPaths, PathBuf, InstalledLock, Platform, AccountInfo) {
    let bytes = match id {
        "26.3" => include_bytes!("../fixtures/26.3.json").as_slice(),
        _ => include_bytes!("../fixtures/1.21.1.json").as_slice(),
    };
    let platform = Platform::x64(os, "10.0.26100").unwrap_or_else(|_| panic!("platform"));
    let version = serde_json::from_slice::<VersionMetadata>(bytes)
        .unwrap_or_else(|_| panic!("metadata"))
        .resolve(&platform)
        .unwrap_or_else(|_| panic!("resolve"));
    let runtime_root = format!(
        "runtime/java-{}-{}",
        version.java.major_version,
        platform.runtime_key().unwrap_or_default()
    );
    let java_executable = format!(
        "{runtime_root}/bin/{}",
        if os == "windows" { "java.exe" } else { "java" }
    );
    let mut artifacts: Vec<_> = version
        .classpath
        .iter()
        .map(|artifact| PlannedArtifact {
            destination: library_cache_path(artifact).unwrap_or_else(|_| panic!("library")),
            artifact: artifact.clone(),
            executable: false,
        })
        .collect();
    artifacts.extend([
        PlannedArtifact {
            destination: format!("versions/{0}/{0}-{1}.jar", version.id, version.client.sha1),
            artifact: version.client.clone(),
            executable: false,
        },
        PlannedArtifact {
            destination: asset_index_cache_path(&version.assets)
                .unwrap_or_else(|_| panic!("assets")),
            artifact: version.assets.artifact.clone(),
            executable: false,
        },
        PlannedArtifact {
            destination: java_executable.clone(),
            artifact: Artifact {
                url: "https://piston-data.mojang.com/java".into(),
                sha1: "a".repeat(40),
                size: 1,
                path: None,
            },
            executable: true,
        },
    ]);
    if let Some(logging) = &version.logging {
        artifacts.push(PlannedArtifact {
            destination: format!(
                "assets/log_configs/{}-{}",
                logging.file.artifact.sha1, logging.file.id
            ),
            artifact: logging.file.artifact.clone(),
            executable: false,
        });
    }
    // A synthetic UNC path lets Windows hosts test Linux ':' separation without
    // a drive-letter colon. The command builder never accesses the filesystem.
    let root = if cfg!(windows) && os == "linux" {
        PathBuf::from(r"\\fixture\share\Panita launch path with spaces")
    } else {
        std::env::temp_dir().join("Panita launch path with spaces")
    };
    let paths = AppPaths::new(root).unwrap_or_else(|_| panic!("root"));
    let directory = paths.root().join("instances/vanilla-test/minecraft");
    let installed = InstalledLock {
        schema_version: 2,
        instance_id: Uuid::new_v4(),
        operation: Uuid::new_v4(),
        plan: InstallPlan {
            schema_version: 1,
            version,
            platform: os.into(),
            architecture: "amd64".into(),
            runtime_root,
            java_executable,
            documents: vec![],
            artifacts,
            loader: None,
        },
        native_files: vec![],
        pack: None,
        native_directory: None,
    };
    let account = AccountInfo {
        account_id: Uuid::new_v4(),
        kind: AccountKind::Offline,
        username: "Fixture_Player".into(),
        profile_id: Uuid::new_v4(),
        oauth_client_id: None,
    };
    (paths, directory, installed, platform, account)
}

fn value_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .unwrap_or_else(|| panic!("missing {flag}"));
    &args[index + 1]
}

#[test]
fn loader_commands_merge_child_libraries_and_keep_module_arguments_platform_specific()
-> Result<(), Box<dyn std::error::Error>> {
    let staged_paths = AppPaths::new(
        std::env::temp_dir().join(format!("panita-loader-command-{}", Uuid::new_v4())),
    )?;
    let loader = crate::neoforge::generation::tests::published(&staged_paths)?;
    for os in ["windows", "linux"] {
        let (paths, directory, mut installed, platform, account) = fixture("1.21.1", os);
        installed.plan.schema_version = 2;
        installed.plan.loader = Some(loader.clone().into());
        let command = build_command(
            &paths,
            &directory,
            &installed,
            &platform,
            &GlobalSettings::default(),
            &InstanceSettings::default(),
            LaunchIdentity {
                account: &account,
                minecraft_token: None,
                xuid: None,
            },
        )?;
        let separator = if os == "windows" { ';' } else { ':' };
        let cp = value_after(&command.args, "-cp");
        let entries: Vec<_> = cp.split(separator).collect();
        let guava = entries
            .iter()
            .filter(|entry| entry.contains("guava-32.1.2-jre.jar"))
            .count();
        assert_eq!(guava, 1);
        assert!(entries.iter().all(|entry| !entry.ends_with(&format!(
            "1.21.1-{}.jar",
            installed.plan.version.client.sha1
        ))));
        assert!(
            value_after(&command.args, "-p")
                .contains(&format!("bootstraplauncher-2.0.2.jar{separator}"))
        );
        assert_eq!(
            value_after(&command.args, "--fml.neoForgeVersion"),
            "21.1.264"
        );
        assert_eq!(value_after(&command.args, "--fml.mcVersion"), "1.21.1");
        assert_eq!(value_after(&command.args, "--version"), "neoforge-21.1.264");
        let main = command
            .args
            .iter()
            .position(|arg| arg == "cpw.mods.bootstraplauncher.BootstrapLauncher")
            .ok_or("main")?;
        let module = command
            .args
            .iter()
            .position(|arg| arg == "-p")
            .ok_or("module")?;
        assert!(module < main);
        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| arg.starts_with("-Xmx"))
                .count(),
            1
        );
        for option in [
            "-DlibraryDirectory=C:/unverified",
            "-Djdk.module.path=C:/unverified",
        ] {
            let settings = GlobalSettings {
                extra_jvm_args: vec![option.into()],
                ..GlobalSettings::default()
            };
            assert!(
                build_command(
                    &paths,
                    &directory,
                    &installed,
                    &platform,
                    &settings,
                    &InstanceSettings::default(),
                    LaunchIdentity {
                        account: &account,
                        minecraft_token: None,
                        xuid: None
                    }
                )
                .is_err()
            );
        }
        installed.plan.schema_version = 1;
        assert!(installed.plan.validate().is_err());
        installed.plan.schema_version = 2;
        installed.plan.loader = None;
        assert!(installed.plan.validate().is_err());
    }
    Ok(())
}

#[test]
fn launch_requires_current_flat_installation_schema() {
    let (paths, directory, mut installed, platform, account) = fixture("1.21.1", "windows");
    for schema in [2, 0, 1, 3] {
        installed.schema_version = schema;
        let result = build_command(
            &paths,
            &directory,
            &installed,
            &platform,
            &GlobalSettings::default(),
            &InstanceSettings::default(),
            LaunchIdentity {
                account: &account,
                minecraft_token: None,
                xuid: None,
            },
        );
        if schema == 2 {
            assert!(result.is_ok());
        } else {
            assert_eq!(
                result.err().map(|error| error.code()),
                Some(ErrorCode::InvalidData)
            );
        }
    }
}

#[test]
fn golden_argument_array_preserves_order_spaces_and_literal_values() {
    use crate::minecraft::{Argument, Arguments};
    let (paths, directory, mut installed, platform, account) = fixture("1.21.1", "windows");
    installed.plan.version.arguments = Some(Arguments {
        jvm: vec![Argument::Literal("-Dfixture.path=${game_directory}".into())],
        game: vec![
            Argument::Literal("--username".into()),
            Argument::Literal("${auth_player_name}".into()),
            Argument::Literal("--gameDir".into()),
            Argument::Literal("${game_directory}".into()),
            Argument::Literal("--accessToken".into()),
            Argument::Literal("${auth_access_token}".into()),
        ],
    });
    installed.plan.version.logging = None;
    let settings = GlobalSettings {
        ram_mib: 2048,
        extra_jvm_args: vec!["-Dfixture.literal=a b;$(literal)".into()],
        ..Default::default()
    };
    let command = build_command(
        &paths,
        &directory,
        &installed,
        &platform,
        &settings,
        &InstanceSettings::default(),
        LaunchIdentity {
            account: &account,
            minecraft_token: None,
            xuid: None,
        },
    )
    .unwrap_or_else(|_| panic!("build"));
    let game_dir = directory.to_str().unwrap_or_default();
    assert_eq!(
        command.args,
        vec![
            format!("-Dfixture.path={game_dir}"),
            "-Dfixture.literal=a b;$(literal)".into(),
            "-Xmx2048M".into(),
            "net.minecraft.client.main.Main".into(),
            "--username".into(),
            "Fixture_Player".into(),
            "--gameDir".into(),
            game_dir.into(),
            "--accessToken".into(),
            "0".into(),
        ]
    );
}

#[test]
fn both_pins_build_direct_arguments_for_both_platforms() {
    for id in ["1.21.1", "26.3"] {
        for os in ["windows", "linux"] {
            let (paths, directory, mut installed, platform, account) = fixture(id, os);
            let native_path = if id == "26.3" {
                let path = format!("natives/{}", installed.operation);
                installed.native_directory = Some(path.clone());
                path
            } else {
                format!("natives-{}", installed.operation)
            };
            let instance = InstanceSettings {
                ram_mib: Some(6144),
                ..Default::default()
            };
            let command = build_command(
                &paths,
                &directory,
                &installed,
                &platform,
                &GlobalSettings::default(),
                &instance,
                LaunchIdentity {
                    account: &account,
                    minecraft_token: None,
                    xuid: None,
                },
            )
            .unwrap_or_else(|e| panic!("{id}/{os}: {e}"));
            assert_eq!(
                command.executable(),
                paths.root().join(&installed.plan.java_executable)
            );
            assert_eq!(command.working_directory(), directory);
            assert_eq!(
                value_after(&command.args, "--gameDir"),
                directory.to_str().unwrap_or_default()
            );
            assert_eq!(value_after(&command.args, "--username"), "Fixture_Player");
            assert_eq!(
                value_after(&command.args, "--uuid"),
                account.profile_id.simple().to_string()
            );
            assert_eq!(value_after(&command.args, "--accessToken"), "0");
            if id == "1.21.1" {
                assert_eq!(value_after(&command.args, "--userType"), "legacy");
            } else {
                assert!(!command.args.contains(&"--userType".into()));
            }
            assert_eq!(value_after(&command.args, "--version"), id);
            assert_eq!(value_after(&command.args, "--versionType"), "release");
            assert_eq!(value_after(&command.args, "--width"), "1280");
            assert_eq!(value_after(&command.args, "--height"), "720");
            assert_eq!(
                value_after(&command.args, "--assetIndex"),
                installed.plan.asset_index_name().unwrap_or_default()
            );
            assert!(command.args.contains(&"-Xmx6144M".into()));
            assert!(command.args.contains(&format!(
                    "-Djava.library.path={}",
                    directory
                        .parent()
                        .unwrap_or(&directory)
                    .join(if id == "26.3" { format!("{native_path}/java") } else { native_path })
                        .display()
                )));
            let separator = if os == "windows" { ";" } else { ":" };
            let classpath = value_after(&command.args, "-cp");
            assert_eq!(
                classpath.split(separator).count(),
                installed.plan.version.classpath.len() + 1
            );
            assert!(
                classpath.ends_with(
                    &path_text(
                        &paths
                            .root()
                            .join(installed.plan.client_path().unwrap_or_default())
                    )
                    .unwrap_or_default()
                )
            );
            assert!(!command.args.iter().any(|arg| arg.contains("${")));
            assert!(
                command
                    .args
                    .iter()
                    .any(|arg| arg.starts_with("-Dlog4j.configurationFile=")
                        && arg.contains("path with spaces"))
            );
        }
    }
}

#[test]
fn microsoft_identity_requires_token_and_debug_is_redacted() {
    let (paths, directory, installed, platform, mut account) = fixture("1.21.1", "windows");
    account.kind = AccountKind::Microsoft;
    account.oauth_client_id = Some(crate::accounts::OAUTH_CLIENT_ID.into());
    let build = |token| {
        build_command(
            &paths,
            &directory,
            &installed,
            &platform,
            &GlobalSettings::default(),
            &InstanceSettings::default(),
            LaunchIdentity {
                account: &account,
                minecraft_token: token,
                xuid: Some("1234"),
            },
        )
    };
    assert_eq!(
        build(None).err().map(|e| e.code()),
        Some(ErrorCode::AuthenticationRequired)
    );
    let token =
        Secret::new("fixture-minecraft-access-token".into()).unwrap_or_else(|_| panic!("token"));
    let command = build(Some(&token)).unwrap_or_else(|_| panic!("build"));
    assert_eq!(value_after(&command.args, "--accessToken"), token.expose());
    assert_eq!(value_after(&command.args, "--userType"), "msa");
    assert_eq!(
        value_after(&command.args, "--clientId"),
        crate::accounts::OAUTH_CLIENT_ID
    );
    assert_eq!(value_after(&command.args, "--xuid"), "1234");
    assert_eq!(format!("{command:?}"), "LaunchCommand { <redacted> }");
}

#[test]
fn invalid_settings_runtime_and_placeholders_fail_closed() {
    let (paths, directory, mut installed, platform, account) = fixture("1.21.1", "windows");
    let build = |installed: &InstalledLock, settings: &GlobalSettings| {
        build_command(
            &paths,
            &directory,
            installed,
            &platform,
            settings,
            &InstanceSettings::default(),
            LaunchIdentity {
                account: &account,
                minecraft_token: None,
                xuid: None,
            },
        )
    };
    for arg in [
        "@args.txt",
        "-cp",
        "-jar",
        "-Xmx999G",
        "-XX:MaxHeapSize=999G",
        "-Djava.class.path=other",
        "-XX:VMOptionsFile=other",
        "-javaagent:other",
    ] {
        let settings = GlobalSettings {
            extra_jvm_args: vec![arg.into()],
            ..Default::default()
        };
        assert!(build(&installed, &settings).is_err(), "{arg}");
    }
    assert!(
        build(
            &installed,
            &GlobalSettings {
                ram_mib: 511,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        build(
            &installed,
            &GlobalSettings {
                java_path_override: Some("other-java".into()),
                ..Default::default()
            }
        )
        .is_err()
    );
    let java = installed.plan.java_executable.clone();
    installed
        .plan
        .artifacts
        .retain(|artifact| artifact.destination != java);
    assert!(build(&installed, &GlobalSettings::default()).is_err());
    let (_, _, mut installed, _, _) = fixture("1.21.1", "windows");
    installed
        .plan
        .version
        .arguments
        .as_mut()
        .unwrap_or_else(|| panic!("args"))
        .game
        .push(super::super::Argument::Literal("${unknown}".into()));
    assert!(build(&installed, &GlobalSettings::default()).is_err());
}

#[test]
fn legacy_cache_destinations_remain_in_launch_arguments() {
    let (paths, directory, mut installed, platform, account) = fixture("1.21.1", "windows");
    let client = installed.plan.client_path().unwrap_or_default();
    let legacy = format!(
        "versions/{}/{}/client.jar",
        installed.plan.version.id, installed.plan.version.client.sha1
    );
    for artifact in &mut installed.plan.artifacts {
        if artifact.destination == client {
            artifact.destination = legacy.clone();
        }
    }
    let command = build_command(
        &paths,
        &directory,
        &installed,
        &platform,
        &GlobalSettings::default(),
        &InstanceSettings::default(),
        LaunchIdentity {
            account: &account,
            minecraft_token: None,
            xuid: None,
        },
    )
    .unwrap_or_else(|_| panic!("build"));
    assert!(
        value_after(&command.args, "-cp")
            .ends_with(&path_text(&paths.root().join(legacy)).unwrap_or_default())
    );
}
