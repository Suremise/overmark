//! Windows' known folders, such as the user's Pictures (which may have been moved, or be on
//! OneDrive: never assume `%USERPROFILE%\Pictures`).

use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

/// The user's Pictures folder, where it really is (`SHGetKnownFolderPath`); `None` if Windows
/// can't say.
pub fn pictures() -> Option<PathBuf> {
    // SAFETY: Windows allocates the path with CoTaskMemAlloc; we copy it and free it once.
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_the_pictures_folder() {
        let p = super::pictures().expect("a Pictures folder");
        assert!(p.is_absolute(), "{}", p.display());
    }
}
