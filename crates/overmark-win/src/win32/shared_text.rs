//! A little text one process publishes and others read: named shared memory
//! (`CreateFileMappingW`), in this user session only (`Local\`). The tray publishes its hotkeys'
//! state there for the Settings window, which is a process of its own.
//!
//! The memory holds a sequence number, the text's length and the text (UTF-8). The writer makes
//! the number odd while it writes and even after, so a reader that sees it change, or odd,
//! reads again: it never takes half of one text and half of another.

use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
    MapViewOfFile, OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
};
use windows::core::HSTRING;

/// The sequence number and the length, before the text.
const HEADER: usize = 8;

/// A view of the memory, unmapped and closed when dropped.
struct View {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    size: usize,
}

impl View {
    fn seq(&self) -> &AtomicU32 {
        // SAFETY: the view is at least HEADER bytes, page-aligned, and lives as long as `self`.
        unsafe { &*(self.view.Value as *const AtomicU32) }
    }

    fn len(&self) -> &AtomicU32 {
        // SAFETY: as `seq`, four bytes on.
        unsafe { &*((self.view.Value as *const u8).add(4) as *const AtomicU32) }
    }

    fn text_ptr(&self) -> *mut u8 {
        // SAFETY: HEADER is within the view.
        unsafe { (self.view.Value as *mut u8).add(HEADER) }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        // SAFETY: our own view and handle, released once.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

// SAFETY: the view is shared memory, written only through `&mut SharedText` and read with the
// sequence number; the handle may be used from any thread.
unsafe impl Send for View {}

/// The writer's side: the memory exists while this lives.
pub struct SharedText {
    view: View,
}

impl SharedText {
    /// Makes the memory `name` (`Local\Overmark.State`) with room for `capacity` bytes of text.
    pub fn create(name: &str, capacity: usize) -> windows::core::Result<SharedText> {
        let size = HEADER + capacity;
        // SAFETY: memory backed by the paging file, named; the view is checked before use.
        unsafe {
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size as u32,
                &HSTRING::from(name),
            )?;
            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size);
            if view.Value.is_null() {
                let error = windows::core::Error::from_thread();
                let _ = CloseHandle(handle);
                return Err(error);
            }
            Ok(SharedText {
                view: View { handle, view, size },
            })
        }
    }

    /// Publishes `text`, cut at a character boundary if it's longer than the room.
    pub fn write(&mut self, text: &str) {
        let room = self.view.size - HEADER;
        let mut end = text.len().min(room);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let seq = self.view.seq();
        let start = seq.load(Ordering::Relaxed);
        let writing = start | 1;
        seq.store(writing, Ordering::Release);
        // SAFETY: `end` bytes fit after the header; the source is a live string.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), self.view.text_ptr(), end) };
        self.view.len().store(end as u32, Ordering::Release);
        seq.store(writing.wrapping_add(1), Ordering::Release);
    }
}

/// Reads the text another process published as `name`, or `None` if nothing is published
/// (its writer isn't running).
pub fn read(name: &str) -> Option<String> {
    // SAFETY: the mapping is opened read-only and its view checked; both are released by View.
    let view = unsafe {
        let handle = OpenFileMappingW(FILE_MAP_READ.0, false, &HSTRING::from(name)).ok()?;
        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            let _ = CloseHandle(handle);
            return None;
        }
        View {
            handle,
            view,
            size: 0,
        }
    };
    for _ in 0..1000 {
        let before = view.seq().load(Ordering::Acquire);
        if before & 1 == 1 {
            std::hint::spin_loop();
            continue;
        }
        let len = view.len().load(Ordering::Acquire) as usize;
        let mut bytes = vec![0u8; len];
        // SAFETY: the writer never publishes a length beyond its room.
        unsafe { std::ptr::copy_nonoverlapping(view.text_ptr(), bytes.as_mut_ptr(), len) };
        if view.seq().load(Ordering::Acquire) == before {
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(what: &str) -> String {
        format!(r"Local\OvermarkTest-{}-{what}", std::process::id())
    }

    #[test]
    fn what_is_written_is_read_and_nothing_once_gone() {
        let n = name("shared");
        assert_eq!(read(&n), None);
        let mut shared = SharedText::create(&n, 64).unwrap();
        assert_eq!(read(&n).as_deref(), Some(""));
        shared.write("gen=1\ngeneral.hotkey_draw=ok\n");
        assert_eq!(read(&n).as_deref(), Some("gen=1\ngeneral.hotkey_draw=ok\n"));
        shared.write("short");
        assert_eq!(read(&n).as_deref(), Some("short"));
        drop(shared);
        assert_eq!(read(&n), None);
    }

    #[test]
    fn a_long_text_is_cut_at_a_character() {
        let n = name("long");
        let mut shared = SharedText::create(&n, 5).unwrap();
        shared.write("abcdé");
        assert_eq!(read(&n).as_deref(), Some("abcd"));
    }

    /// A reader beside a writer that keeps writing only ever sees whole texts.
    #[test]
    fn readers_never_see_half_a_text() {
        let n = name("race");
        let mut shared = SharedText::create(&n, 256).unwrap();
        let texts = ["a".repeat(200), "b".repeat(10)];
        shared.write(&texts[0]);
        let reader = std::thread::spawn({
            let n = n.clone();
            let texts = texts.clone();
            move || {
                for _ in 0..2000 {
                    if let Some(t) = read(&n) {
                        assert!(texts.contains(&t), "a mixed text: {t:?}");
                    }
                }
            }
        });
        for i in 0..2000 {
            shared.write(&texts[i % 2]);
        }
        reader.join().unwrap();
    }
}
