//! Publishing a finished file without replacing anything at its destination.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};

use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
use windows::core::HSTRING;

/// Renames `from` to `to` on the same volume, failing if `to` already exists. Unlike
/// `std::fs::rename`, the existence check and move are one filesystem operation.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    rename_no_replace_with(from, to, |p| std::fs::canonicalize(p))
}

fn rename_no_replace_with(
    from: &Path,
    to: &Path,
    mut canonicalize: impl FnMut(&Path) -> io::Result<PathBuf>,
) -> io::Result<()> {
    // Canonical paths carry Windows' extended-length prefix, as std's file operations do.
    // Some virtual drives cannot resolve final paths by handle: keep saving there by
    // falling back to lexical absolute paths, without requiring either entry to be opened.
    // Resolve only the destination's parent: the destination need not exist, and an existing
    // link there must be treated as a collision, not followed.
    let from = canonicalize(from).or_else(|_| std::path::absolute(from))?;
    let parent = to.parent().filter(|p| !p.as_os_str().is_empty());
    let name = to
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let parent = parent.unwrap_or(Path::new("."));
    let to = canonicalize(parent)
        .or_else(|_| std::path::absolute(parent))?
        .join(name);
    let (from, to) = (move_path(&from)?, move_path(&to)?);
    // SAFETY: both paths are NUL-terminated and live through the call. No REPLACE_EXISTING
    // or COPY_ALLOWED: a collision fails, and a failure leaves the source file intact.
    unsafe { MoveFileExW(&from, &to, MOVE_FILE_FLAGS(0)) }.map_err(|e| {
        // MoveFileExW's error is HRESULT_FROM_WIN32(GetLastError()); recover the OS code so
        // callers can distinguish AlreadyExists from permission and filesystem failures.
        io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
    })
}

/// Prepares an absolute path for the native move. Lexical fallback paths need the extended
/// prefix when long, including the destination's file name; already-verbatim paths stay so.
fn move_path(path: &Path) -> io::Result<HSTRING> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    if wide.len() >= 260
        && let Some(Component::Prefix(prefix)) = path.components().next()
    {
        match prefix.kind() {
            Prefix::Disk(_) => {
                wide.splice(..0, r"\\?\".encode_utf16());
            }
            Prefix::UNC(..) => {
                wide.splice(..2, r"\\?\UNC\".encode_utf16());
            }
            _ => {}
        }
    }
    Ok(HSTRING::from_wide(&wide))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_unc_paths_gain_the_unc_prefix_without_a_network_lookup() {
        let path = PathBuf::from(r"\\server\share").join("a".repeat(260));
        let expected = PathBuf::from(r"\\?\UNC\server\share").join("a".repeat(260));
        assert_eq!(
            move_path(&path).unwrap().to_os_string(),
            expected.as_os_str()
        );
        assert_eq!(
            move_path(&expected).unwrap().to_os_string(),
            expected.as_os_str()
        );
    }

    #[test]
    fn publication_works_when_the_volume_cannot_resolve_final_paths() {
        let dir = tempfile::tempdir().unwrap();
        let mut folder = dir.path().to_owned();
        for _ in 0..5 {
            folder.push("long-directory-".repeat(4));
        }
        std::fs::create_dir_all(&folder).unwrap();
        let from = folder.join("記録.mp4.part");
        let to = folder.join("記録.mp4");
        std::fs::write(&from, b"finished video").unwrap();
        let mut calls = 0;
        rename_no_replace_with(&from, &to, |_| {
            calls += 1;
            Err(io::ErrorKind::Unsupported.into())
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(std::fs::read(&to).unwrap(), b"finished video");
        assert!(!from.exists());
    }
}
