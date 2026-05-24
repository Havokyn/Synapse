use crate::{ActionError, ActionResult};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ClipboardFormat {
    Text,
    Unicode,
}

/// Reads text from the system clipboard in the requested text format.
///
/// # Errors
///
/// Returns an [`ActionError`] when the platform clipboard backend is unavailable
/// or the clipboard data cannot be decoded as the requested text format.
pub fn read_text(format: ClipboardFormat) -> ActionResult<String> {
    platform::read_text(format)
}

/// Writes text to the system clipboard in the requested text format.
///
/// # Errors
///
/// Returns an [`ActionError`] when the platform clipboard backend is unavailable,
/// when `CF_TEXT` is requested for non-ASCII text, or when a Windows clipboard API
/// call fails.
pub fn write_text(format: ClipboardFormat, text: &str) -> ActionResult<()> {
    if matches!(format, ClipboardFormat::Text) && !text.is_ascii() {
        return Err(ActionError::BackendUnavailable {
            detail: "CF_TEXT clipboard writes are limited to ASCII in M2; use unicode format"
                .to_owned(),
        });
    }
    platform::write_text(format, text)
}

/// Clears the system clipboard.
///
/// # Errors
///
/// Returns an [`ActionError`] when the platform clipboard backend is unavailable
/// or the platform clear operation fails.
pub fn clear() -> ActionResult<()> {
    platform::clear()
}

#[cfg(not(windows))]
mod platform {
    use super::{ActionError, ActionResult, ClipboardFormat};

    pub fn read_text(_format: ClipboardFormat) -> ActionResult<String> {
        Err(unavailable("read"))
    }

    pub fn write_text(_format: ClipboardFormat, _text: &str) -> ActionResult<()> {
        Err(unavailable("write"))
    }

    pub fn clear() -> ActionResult<()> {
        Err(unavailable("clear"))
    }

    fn unavailable(verb: &'static str) -> ActionError {
        ActionError::BackendUnavailable {
            detail: format!("act_clipboard {verb} requires Windows clipboard APIs"),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        ptr, slice, thread,
        time::{Duration, Instant},
    };

    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
                OpenClipboard, SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        },
    };

    use super::{ActionError, ActionResult, ClipboardFormat};

    const CF_TEXT: u32 = 1;
    const CF_UNICODETEXT: u32 = 13;
    const OPEN_CLIPBOARD_RETRY_TIMEOUT: Duration = Duration::from_millis(250);
    const OPEN_CLIPBOARD_RETRY_DELAY: Duration = Duration::from_millis(10);

    pub fn read_text(format: ClipboardFormat) -> ActionResult<String> {
        let _clipboard = ClipboardGuard::open("read")?;
        if !format_available(format) {
            return Ok(String::new());
        }

        let handle = unsafe {
            // SAFETY: The clipboard is open for this thread, and the format code is
            // one of the standard text formats accepted by GetClipboardData.
            GetClipboardData(format_code(format))
        }
        .map_err(|err| windows_error("GetClipboardData", &err))?;
        let hglobal = HGLOBAL(handle.0);
        let byte_len = unsafe {
            // SAFETY: The clipboard owns this HGLOBAL while the clipboard remains open.
            GlobalSize(hglobal)
        };
        let locked = LockedGlobal::lock(hglobal, "read")?;
        match format {
            ClipboardFormat::Unicode => read_unicode(locked.ptr(), byte_len),
            ClipboardFormat::Text => Ok(read_text_bytes(locked.ptr(), byte_len)),
        }
    }

    pub fn write_text(format: ClipboardFormat, text: &str) -> ActionResult<()> {
        let _clipboard = ClipboardGuard::open("write")?;
        let memory = match format {
            ClipboardFormat::Unicode => GlobalMemory::from_bytes(&unicode_clipboard_bytes(text))?,
            ClipboardFormat::Text => GlobalMemory::from_bytes(&text_clipboard_bytes(text))?,
        };
        unsafe {
            // SAFETY: The clipboard is open for this thread.
            EmptyClipboard()
        }
        .map_err(|err| windows_error("EmptyClipboard", &err))?;
        memory.give_to_clipboard(format)
    }

    pub fn clear() -> ActionResult<()> {
        let _clipboard = ClipboardGuard::open("clear")?;
        unsafe {
            // SAFETY: The clipboard is open for this thread.
            EmptyClipboard()
        }
        .map_err(|err| windows_error("EmptyClipboard", &err))
    }

    struct ClipboardGuard;

    impl ClipboardGuard {
        fn open(context: &'static str) -> ActionResult<Self> {
            let started = Instant::now();
            let mut attempts = 0_u32;
            loop {
                attempts += 1;
                let result = unsafe {
                    // SAFETY: Passing None preserves the existing M2 clipboard-owner
                    // behavior; the guard closes the clipboard exactly once on success.
                    OpenClipboard(None)
                };
                match result {
                    Ok(()) => return Ok(Self),
                    Err(err) if started.elapsed() < OPEN_CLIPBOARD_RETRY_TIMEOUT => {
                        tracing::debug!(
                            code = "ACTION_CLIPBOARD_OPEN_RETRY",
                            context,
                            attempts,
                            error = %err,
                            "source_of_truth=windows_clipboard open_retry"
                        );
                        thread::sleep(OPEN_CLIPBOARD_RETRY_DELAY);
                    }
                    Err(err) => {
                        return Err(windows_open_error(context, attempts, started, &err));
                    }
                }
            }
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                // SAFETY: This guard is created only after OpenClipboard succeeds.
                let _ = CloseClipboard();
            }
        }
    }

    struct LockedGlobal {
        handle: HGLOBAL,
        ptr: *mut core::ffi::c_void,
    }

    impl LockedGlobal {
        fn lock(handle: HGLOBAL, context: &'static str) -> ActionResult<Self> {
            let ptr = unsafe {
                // SAFETY: Caller passes an HGLOBAL returned by clipboard/global APIs.
                GlobalLock(handle)
            };
            if ptr.is_null() {
                return Err(ActionError::BackendUnavailable {
                    detail: format!("GlobalLock failed during clipboard {context}"),
                });
            }
            Ok(Self { handle, ptr })
        }

        const fn ptr(&self) -> *const core::ffi::c_void {
            self.ptr.cast_const()
        }
    }

    impl Drop for LockedGlobal {
        fn drop(&mut self) {
            unsafe {
                // SAFETY: This guard is created only after GlobalLock returns non-null.
                let _ = GlobalUnlock(self.handle);
            }
        }
    }

    struct GlobalMemory {
        handle: HGLOBAL,
        owned: bool,
    }

    impl GlobalMemory {
        fn from_bytes(bytes: &[u8]) -> ActionResult<Self> {
            let handle = unsafe {
                // SAFETY: GlobalAlloc is called with GMEM_MOVEABLE as required by
                // SetClipboardData, and the byte length is derived from a slice.
                GlobalAlloc(GMEM_MOVEABLE, bytes.len())
            }
            .map_err(|err| windows_error("GlobalAlloc", &err))?;
            {
                let locked = LockedGlobal::lock(handle, "write")?;
                unsafe {
                    // SAFETY: The destination points to a GlobalAlloc block of
                    // bytes.len() bytes and the source slice is valid for that length.
                    ptr::copy_nonoverlapping(bytes.as_ptr(), locked.ptr.cast::<u8>(), bytes.len());
                }
            }
            Ok(Self {
                handle,
                owned: true,
            })
        }

        fn give_to_clipboard(mut self, format: ClipboardFormat) -> ActionResult<()> {
            unsafe {
                // SAFETY: The clipboard is open and SetClipboardData takes ownership
                // of the movable memory handle on success.
                SetClipboardData(format_code(format), Some(HANDLE(self.handle.0)))
            }
            .map_err(|err| windows_error("SetClipboardData", &err))?;
            self.owned = false;
            Ok(())
        }
    }

    impl Drop for GlobalMemory {
        fn drop(&mut self) {
            if self.owned {
                unsafe {
                    // SAFETY: The handle is still owned by this process when owned=true.
                    let _ = GlobalFree(Some(self.handle));
                }
            }
        }
    }

    fn format_available(format: ClipboardFormat) -> bool {
        unsafe {
            // SAFETY: The clipboard is open for this thread, and the format code is
            // one of the standard clipboard text formats.
            IsClipboardFormatAvailable(format_code(format))
        }
        .is_ok()
    }

    const fn format_code(format: ClipboardFormat) -> u32 {
        match format {
            ClipboardFormat::Text => CF_TEXT,
            ClipboardFormat::Unicode => CF_UNICODETEXT,
        }
    }

    fn unicode_clipboard_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn text_clipboard_bytes(text: &str) -> Vec<u8> {
        text.bytes().chain(std::iter::once(0)).collect()
    }

    fn read_unicode(ptr: *const core::ffi::c_void, byte_len: usize) -> ActionResult<String> {
        let unit_len = byte_len / size_of::<u16>();
        let units = unsafe {
            // SAFETY: The pointer comes from a locked global memory block, and unit_len
            // is bounded by GlobalSize for that block.
            slice::from_raw_parts(ptr.cast::<u16>(), unit_len)
        };
        let nul = units.iter().position(|unit| *unit == 0).unwrap_or(unit_len);
        String::from_utf16(&units[..nul]).map_err(|err| ActionError::BackendUnavailable {
            detail: format!("clipboard unicode text is invalid UTF-16: {err}"),
        })
    }

    fn read_text_bytes(ptr: *const core::ffi::c_void, byte_len: usize) -> String {
        let bytes = unsafe {
            // SAFETY: The pointer comes from a locked global memory block, and byte_len
            // is exactly GlobalSize for that block.
            slice::from_raw_parts(ptr.cast::<u8>(), byte_len)
        };
        let nul = bytes.iter().position(|byte| *byte == 0).unwrap_or(byte_len);
        String::from_utf8_lossy(&bytes[..nul]).into_owned()
    }

    fn windows_error(context: &'static str, err: &windows::core::Error) -> ActionError {
        ActionError::BackendUnavailable {
            detail: format!("{context} failed for Windows clipboard: {err}"),
        }
    }

    fn windows_open_error(
        context: &'static str,
        attempts: u32,
        started: Instant,
        err: &windows::core::Error,
    ) -> ActionError {
        ActionError::BackendUnavailable {
            detail: format!(
                "{context} failed for Windows clipboard after {attempts} open attempts over {} ms: {err}",
                started.elapsed().as_millis()
            ),
        }
    }
}
