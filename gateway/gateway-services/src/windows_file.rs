use std::fs::File;
use std::io;
use std::mem::MaybeUninit;
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `file` owns a live Windows handle for the duration of the call,
    // and `information` points to writable storage of the exact API type.
    let result =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: a successful call initializes the complete output structure.
        Ok(unsafe { information.assume_init() })
    }
}

fn identity(file: &File) -> io::Result<FileIdentity> {
    let information = information(file)?;
    Ok(FileIdentity {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

pub(crate) fn has_single_link(file: &File) -> io::Result<bool> {
    Ok(information(file)?.nNumberOfLinks == 1)
}

pub(crate) fn same_file(left: &File, right: &File) -> io::Result<bool> {
    Ok(identity(left)? == identity(right)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_windows_file_identity_uses_open_handles() {
        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first.md");
        let second_path = directory.path().join("second.md");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();

        let first = File::open(&first_path).unwrap();
        let first_reopened = File::open(&first_path).unwrap();
        let second = File::open(&second_path).unwrap();

        assert!(same_file(&first, &first_reopened).unwrap());
        assert!(!same_file(&first, &second).unwrap());
    }

    #[test]
    fn portable_windows_link_count_rejects_hard_links() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.md");
        let alias = directory.path().join("alias.md");
        std::fs::write(&target, b"bounded").unwrap();

        let file = File::open(&target).unwrap();
        assert!(has_single_link(&file).unwrap());

        std::fs::hard_link(&target, alias).unwrap();
        assert!(!has_single_link(&file).unwrap());
    }
}
