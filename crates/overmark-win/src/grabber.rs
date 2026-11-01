//! Clean grabs (the screen without Overmark's windows, which stay on screen) from a helper
//! process: `overmark.exe --grabber`, started while the overlay is open and gone when it
//! closes.
//!
//! Why a process of its own: the Magnification API, which makes the clean grab
//! ([`crate::win32::magnifier`]), keeps 35-50 MB in the process that used it, and more after
//! each grab, even once every magnifier is gone and the runtime uninitialised (measured: 121 MB
//! private after 16 opens with a freeze each). In a helper, all of it goes when the helper
//! exits, so the resident app stays at its idle footprint.
//!
//! The protocol, on the helper's stdin and stdout (little-endian):
//! - `P` rect hwnds: prepare a magnifier for `rect`, leaving out `hwnds`; answered `K`.
//! - `G` rect hwnds: grab `rect` without `hwnds`; answered width, height, then the BGRA pixels
//!   (width 0: no image).
//!
//! `rect` is four `i32`s (left, top, right, bottom); `hwnds` is a `u32` count and that many
//! `u64`s.

use std::io::{self, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use windows::Win32::Foundation::HWND;

use crate::win32::Rect;
use crate::win32::capture::Frame;

/// The argument that makes the app a grabber.
pub const ARG: &str = "--grabber";

fn write_request(w: &mut impl Write, op: u8, rect: Rect, hwnds: &[HWND]) -> io::Result<()> {
    let mut b = Vec::with_capacity(1 + 16 + 4 + 8 * hwnds.len());
    b.push(op);
    for v in [rect.left, rect.top, rect.right, rect.bottom] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(hwnds.len() as u32).to_le_bytes());
    for h in hwnds {
        b.extend_from_slice(&(h.0 as usize as u64).to_le_bytes());
    }
    w.write_all(&b)?;
    w.flush()
}

fn read_i32(r: &mut impl Read) -> io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

/// A request read by the helper: its op, rect and windows; `None` at the end of input.
fn read_request(r: &mut impl Read) -> io::Result<Option<(u8, Rect, Vec<HWND>)>> {
    let mut op = [0u8; 1];
    match r.read_exact(&mut op) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let rect = Rect::new(read_i32(r)?, read_i32(r)?, read_i32(r)?, read_i32(r)?);
    let n = read_i32(r)? as u32;
    if n > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "too many windows",
        ));
    }
    let mut hwnds = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let mut b = [0u8; 8];
        r.read_exact(&mut b)?;
        hwnds.push(HWND(u64::from_le_bytes(b) as usize as *mut _));
    }
    Ok(Some((op[0], rect, hwnds)))
}

fn write_frame(w: &mut impl Write, frame: Option<&Frame>) -> io::Result<()> {
    match frame {
        Some(f) => {
            w.write_all(&f.width.to_le_bytes())?;
            w.write_all(&f.height.to_le_bytes())?;
            w.write_all(&f.bgra)?;
        }
        None => {
            w.write_all(&0i32.to_le_bytes())?;
            w.write_all(&0i32.to_le_bytes())?;
        }
    }
    w.flush()
}

fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let (w, h) = (read_i32(r)?, read_i32(r)?);
    if w == 0 || h == 0 {
        return Ok(None);
    }
    let len = crate::win32::byte_len(w, h)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a bad image size"))?;
    let mut bgra = vec![0u8; len];
    r.read_exact(&mut bgra)?;
    Ok(Some(Frame {
        width: w,
        height: h,
        bgra,
    }))
}

/// The helper's side: answers requests until its input ends (the app closed the overlay, or
/// quit). Run by `overmark.exe --grabber`.
pub fn serve() -> i32 {
    use crate::win32::magnifier::Magnifier;
    let _ = crate::win32::dpi::enable_per_monitor_v2();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut magnifiers: Vec<(Rect, Magnifier)> = Vec::new();
    loop {
        let (op, rect, hwnds) = match read_request(&mut input) {
            Ok(Some(r)) => r,
            Ok(None) => return 0,
            Err(error) => {
                tracing::warn!(%error, "grabber: a bad request");
                return 1;
            }
        };
        let i = match magnifiers.iter().position(|(r, _)| *r == rect) {
            Some(i) => {
                let _ = magnifiers[i].1.set_excluded(&hwnds);
                Some(i)
            }
            None => match Magnifier::new(rect, &hwnds) {
                Ok(m) => {
                    magnifiers.push((rect, m));
                    Some(magnifiers.len() - 1)
                }
                Err(error) => {
                    tracing::warn!(%error, "grabber: no magnifier");
                    None
                }
            },
        };
        let written = match op {
            b'P' => output.write_all(b"K").and_then(|()| output.flush()),
            b'G' => {
                let frame = i.and_then(|i| magnifiers[i].1.grab(500));
                write_frame(&mut output, frame.as_ref())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "an unknown request",
            )),
        };
        if written.is_err() {
            return 1;
        }
    }
}

/// The app's side: a running helper.
pub struct Grabber {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
}

impl Grabber {
    /// Starts `exe --grabber` (the app's own exe).
    pub fn start(exe: &std::path::Path) -> io::Result<Grabber> {
        let mut child = Command::new(exe)
            .arg(ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        Ok(Grabber {
            child,
            input,
            output,
        })
    }

    /// Asks it to make a magnifier for `rect` now, so a grab later answers quickly.
    pub fn prepare(&mut self, rect: Rect, exclude: &[HWND]) -> io::Result<()> {
        write_request(&mut self.input, b'P', rect, exclude)?;
        let mut k = [0u8; 1];
        self.output.read_exact(&mut k)?;
        if k[0] == b'K' {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the grabber answered oddly",
            ))
        }
    }

    /// `rect` now, without `exclude`; `None` if the helper had no image.
    pub fn grab(&mut self, rect: Rect, exclude: &[HWND]) -> io::Result<Option<Frame>> {
        write_request(&mut self.input, b'G', rect, exclude)?;
        read_frame(&mut self.output)
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        // Closing its input ends it; it has nothing to save.
        let _ = self.input.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_frames_round_trip() {
        let mut b = Vec::new();
        let hwnds = [HWND(0x1234 as *mut _), HWND(0xABCD_0000 as *mut _)];
        write_request(&mut b, b'G', Rect::new(-1920, 0, 0, 1080), &hwnds).unwrap();
        let (op, rect, got) = read_request(&mut b.as_slice()).unwrap().unwrap();
        assert_eq!((op, rect), (b'G', Rect::new(-1920, 0, 0, 1080)));
        assert_eq!(got, hwnds);
        assert!(read_request(&mut [].as_slice()).unwrap().is_none());

        let frame = Frame {
            width: 2,
            height: 1,
            bgra: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let mut b = Vec::new();
        write_frame(&mut b, Some(&frame)).unwrap();
        write_frame(&mut b, None).unwrap();
        let mut r = b.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().unwrap().bgra, frame.bgra);
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn rejects_a_bad_frame_size() {
        let mut b = Vec::new();
        b.extend_from_slice(&(-3i32).to_le_bytes());
        b.extend_from_slice(&5i32.to_le_bytes());
        assert!(read_frame(&mut b.as_slice()).is_err());
    }
}
