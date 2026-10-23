//! Fixed, read-only support locations; callers never supply arbitrary paths.
use std::path::PathBuf;

use cap_fs_ext::DirExt;
use uuid::Uuid;

use crate::{error::CoreError, paths::AppPaths, storage::map_private_error};

pub fn crash_report_directory(
    paths: &AppPaths,
    instance: Uuid,
) -> Result<Option<PathBuf>, CoreError> {
    let root = paths.instance(instance)?;
    let private = private_fs::PrivateDir::open(&root).map_err(map_private_error)?;
    let directory = super::world_backup::capability(&private)?;
    let game = directory
        .open_dir_nofollow("minecraft")
        .map_err(super::download::io)?;
    match game.open_dir_nofollow("crash-reports") {
        Ok(_reports) => Ok(Some(root.join("minecraft/crash-reports"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(super::download::io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_folder_is_optional_fixed_and_requires_a_real_directory() {
        let (paths, accounts, id) = super::super::play::tests::fixture();
        drop(accounts);
        let game = paths.game_directory(id).unwrap_or_else(|_| panic!("game"));
        assert_eq!(crash_report_directory(&paths, id).ok(), Some(None));
        let reports = game.join("crash-reports");
        std::fs::write(&reports, b"not a directory").unwrap_or_else(|_| panic!("file"));
        assert!(crash_report_directory(&paths, id).is_err());
        std::fs::remove_file(&reports).unwrap_or_else(|_| panic!("remove file"));
        std::fs::create_dir(&reports).unwrap_or_else(|_| panic!("folder"));
        assert_eq!(crash_report_directory(&paths, id).ok(), Some(Some(reports)));
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }
}
