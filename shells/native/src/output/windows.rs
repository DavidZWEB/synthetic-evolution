//! Windows handle identities for output-alias checks, including hard links.
//!
//! Canonical paths alone do not identify hard-linked files. This small OS boundary
//! uses the same volume/index identity as Unix device/inode checks.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::MaybeUninit;
use std::os::windows::io::AsRawHandle;

use super::FileKey;
use crate::Result;

#[repr(C)]
struct FileInformation {
    attributes: u32,
    creation_time: [u32; 2],
    last_access_time: [u32; 2],
    last_write_time: [u32; 2],
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(handle: *mut c_void, info: *mut FileInformation) -> i32;
    fn GetFileType(handle: *mut c_void) -> u32;
}

pub(super) fn stream_key(file: &File) -> Result<Option<FileKey>> {
    // File owns a valid handle. Consoles and pipes have no disk file identity.
    match unsafe { GetFileType(file.as_raw_handle()) } {
        1 => file_key(file).map(Some),
        2 | 3 => Ok(None),
        _ => Err(io::Error::last_os_error().into()),
    }
}

pub(super) fn file_key(file: &File) -> Result<FileKey> {
    let mut info = MaybeUninit::<FileInformation>::uninit();
    // The live File owns the handle and the repr(C) buffer matches
    // BY_HANDLE_FILE_INFORMATION. Windows initializes it on a nonzero return.
    let success = unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) };
    if success == 0 {
        return Err(io::Error::last_os_error().into());
    }
    // A successful call initialized every field above.
    let info = unsafe { info.assume_init() };
    Ok((
        u64::from(info.volume_serial),
        (u64::from(info.index_high) << 32) | u64::from(info.index_low),
    ))
}
