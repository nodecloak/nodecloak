//! AppImage variables can belong to the application that launched NodeCloak.
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(crate) fn owned_image(
    executable: &Path,
    image: Option<&OsStr>,
    directory: Option<&OsStr>,
) -> Option<PathBuf> {
    let image = PathBuf::from(image?);
    let directory = Path::new(directory?);
    if !image.is_absolute() || !directory.is_absolute() || !image.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if image.metadata().ok()?.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    let executable = executable.canonicalize().ok()?;
    let directory = directory.canonicalize().ok()?;
    (directory.is_dir() && executable.is_file() && executable.starts_with(&directory))
        .then_some(image)
}

#[cfg(target_os = "linux")]
pub(crate) unsafe fn sanitize_before_startup() {
    let image = std::env::var_os("APPIMAGE");
    let directory = std::env::var_os("APPDIR");
    if image.is_none() && directory.is_none() {
        return;
    }
    let owned = std::env::current_exe()
        .ok()
        .and_then(|executable| owned_image(&executable, image.as_deref(), directory.as_deref()));
    if owned.is_some() {
        return;
    }
    // A deb started from another AppImage (for example, a Codex terminal) must
    // not register or replace the parent image. Also stop its bundled libraries
    // from leaking into subsequently launched system tools and terminals.
    for name in [
        "APPIMAGE",
        "APPDIR",
        "ARGV0",
        "OWD",
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
    ] {
        // SAFETY: our caller invokes this as the first statement in main(),
        // before creating any threads, runtime, or Tauri environment snapshot.
        #[allow(unused_unsafe)]
        unsafe {
            std::env::remove_var(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "synthetic executable").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    #[test]
    fn own_image_is_retained_but_a_deb_cannot_adopt_its_parent_image() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("Codex Parent.AppImage");
        let directory = root.path().join("parent-mount");
        let parent = directory.join("usr/bin/codex");
        let deb = root.path().join("usr/bin/nodecloak");
        fixture_file(&image);
        fixture_file(&parent);
        fixture_file(&deb);
        assert_eq!(
            owned_image(
                &parent,
                Some(image.as_os_str()),
                Some(directory.as_os_str())
            ),
            Some(image.clone())
        );
        assert!(owned_image(&deb, Some(image.as_os_str()), Some(directory.as_os_str())).is_none());
        assert!(owned_image(&deb, Some(image.as_os_str()), None).is_none());
        assert!(owned_image(&deb, None, Some(directory.as_os_str())).is_none());
        assert!(owned_image(&deb, None, None).is_none());
        assert_eq!(
            std::fs::read_to_string(&image).unwrap(),
            "synthetic executable"
        );
    }
    #[test]
    fn image_ownership_requires_canonical_containment_and_a_real_image() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("NodeCloak.AppImage");
        let directory = root.path().join("nodecloak-mount");
        let executable = directory.join("usr/bin/nodecloak");
        let sibling = root.path().join("nodecloak-mount-other/usr/bin/nodecloak");
        fixture_file(&image);
        fixture_file(&executable);
        fixture_file(&sibling);
        assert_eq!(
            owned_image(
                &executable,
                Some(image.as_os_str()),
                Some(directory.as_os_str())
            ),
            Some(image.clone())
        );
        assert!(owned_image(
            &sibling,
            Some(image.as_os_str()),
            Some(directory.as_os_str())
        )
        .is_none());
        assert!(owned_image(
            &executable,
            Some(OsStr::new("relative.AppImage")),
            Some(directory.as_os_str())
        )
        .is_none());
        std::fs::remove_file(&image).unwrap();
        assert!(owned_image(
            &executable,
            Some(image.as_os_str()),
            Some(directory.as_os_str())
        )
        .is_none());
    }
    #[cfg(unix)]
    #[test]
    fn a_symlink_into_an_unrelated_mount_does_not_establish_ownership() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("NodeCloak.AppImage");
        let directory = root.path().join("mount");
        let external = root.path().join("usr/bin/nodecloak");
        fixture_file(&image);
        fixture_file(&external);
        std::fs::create_dir(&directory).unwrap();
        let alias = directory.join("nodecloak");
        std::os::unix::fs::symlink(&external, &alias).unwrap();
        assert!(
            owned_image(&alias, Some(image.as_os_str()), Some(directory.as_os_str())).is_none()
        );
    }
}
