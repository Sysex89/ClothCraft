//! CPU-only document I/O. Commit bytes in the destination directory before replacement.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_write_with(path, |writer| {
        writer.write_all(bytes).map_err(|e| e.to_string())
    })
}

pub(crate) fn atomic_write_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> Result<(), String>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // The file may inherit a publicly readable mode. Stage it inside a
    // private directory on the same filesystem until the atomic rename.
    #[cfg(unix)]
    let (staging, mut temporary) = {
        use std::os::unix::fs::PermissionsExt;
        let staging = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(parent)
            .map_err(|e| e.to_string())?;
        let temporary = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o666))
            .tempfile_in(staging.path())
            .map_err(|e| e.to_string())?;
        (staging, temporary)
    };
    #[cfg(not(unix))]
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;

    write(temporary.as_file_mut())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match fs::metadata(path) {
            Ok(existing) => temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(
                    existing.permissions().mode() & 0o777,
                ))
                .map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    drop(staging);
    // Directory sync is supported on Unix; Windows replacement is handled by tempfile.
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| {
            format!(
                "Replaced {}, but could not sync its directory: {e}",
                path.display()
            )
        })?;
    Ok(())
}

/// Content-addressed OBJ next to the project. Existing asset names are never overwritten.
/// Copy the project and its `.assets` directory together to relocate it.
pub fn portable_avatar(source: &Path, project: &Path) -> Result<PathBuf, String> {
    let bytes =
        fs::read(source).map_err(|e| format!("Cannot read avatar {}: {e}", source.display()))?;
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    let stem = project
        .file_name()
        .ok_or("Project needs a filename")?
        .to_string_lossy();
    let relative = PathBuf::from(format!("{stem}.assets")).join(format!("avatar-{hash:016x}.obj"));
    let destination = project.parent().unwrap_or(Path::new(".")).join(&relative);
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    if destination.exists() {
        if fs::read(&destination).map_err(|e| e.to_string())? != bytes {
            return Err("Avatar asset hash collision; existing asset was preserved".into());
        }
    } else {
        atomic_write(&destination, &bytes)?;
    }
    Ok(relative)
}
