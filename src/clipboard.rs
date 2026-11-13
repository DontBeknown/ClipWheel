//! Clipboard handles are borrowed only while open; writes transfer owned HGLOBALs.
use clipwheel::model::{Payload, Prepared};
use std::{
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread,
    time::Duration,
};
use windows::Win32::{
    Foundation::*,
    System::{Com::*, DataExchange::*, Memory::*},
    UI::Shell::*,
};

pub const ITEM_LIMIT: usize = 32 * 1024 * 1024;
pub enum Command {
    Read(u64),
    Write(Arc<Payload>),
    Stop,
}
pub enum Event {
    Captured(u64, Option<Prepared>),
    Written(u32),
    Notice(String),
}
pub struct Worker {
    pub tx: SyncSender<Command>,
    pub rx: Receiver<Event>,
}

impl Worker {
    pub fn start(hwnd: HWND, message: u32) -> Self {
        let (tx, commands) = mpsc::sync_channel(1);
        let (events, rx) = mpsc::sync_channel(1);
        let handle = hwnd.0 as isize;
        thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            while let Ok(command) = commands.recv() {
                let event = match command {
                    Command::Stop => break,
                    Command::Read(epoch) => match read(HWND(handle as *mut _)) {
                        Ok(p) => Some(Event::Captured(epoch, p.map(Prepared::new))),
                        Err(e) => Some(Event::Notice(e)),
                    },
                    Command::Write(payload) => {
                        Some(match write(HWND(handle as *mut _), &payload) {
                            Ok(seq) => Event::Written(seq),
                            Err(e) => Event::Notice(e),
                        })
                    }
                };
                if let Some(event) = event {
                    if events.send(event).is_err() {
                        break;
                    }
                }
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    HWND(handle as *mut _),
                    message,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            CoUninitialize();
        });
        Self { tx, rx }
    }
}

struct Open;
impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
unsafe fn open(hwnd: HWND) -> std::result::Result<Open, String> {
    for _ in 0..6 {
        if OpenClipboard(hwnd).is_ok() {
            return Ok(Open);
        }
        thread::sleep(Duration::from_millis(15));
    }
    Err("Clipboard is busy. Please try again.".into())
}

unsafe fn global_bytes(format: u32) -> std::result::Result<Vec<u8>, String> {
    let handle = GetClipboardData(format).map_err(|e| e.to_string())?;
    let memory = HGLOBAL(handle.0);
    let size = GlobalSize(memory);
    if size > ITEM_LIMIT || size == 0 {
        return Err("Clipboard item exceeds 32 MiB or is empty.".into());
    }
    let ptr = GlobalLock(memory);
    if ptr.is_null() {
        return Err("Cannot read clipboard memory.".into());
    }
    let bytes = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
    let _ = GlobalUnlock(memory);
    Ok(bytes)
}

unsafe fn read(hwnd: HWND) -> std::result::Result<Option<Payload>, String> {
    let _guard = open(hwnd)?;
    if IsClipboardFormatAvailable(15).is_ok() {
        let handle = GetClipboardData(15).map_err(|e| e.to_string())?;
        let drop = HDROP(handle.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        if count > 8192 {
            return Err("Too many files in one clipboard item.".into());
        }
        let mut paths = Vec::new();
        let mut bytes = 0;
        for i in 0..count {
            let size = DragQueryFileW(drop, i, None) as usize;
            bytes += size * 2;
            if bytes > ITEM_LIMIT {
                return Err("File list exceeds 32 MiB.".into());
            }
            let mut buf = vec![0u16; size + 1];
            DragQueryFileW(drop, i, Some(&mut buf));
            paths.push(String::from_utf16_lossy(&buf[..size]));
        }
        return Ok(Some(Payload::Files(paths)));
    }
    for format in [17, 8] {
        if IsClipboardFormatAvailable(format).is_ok() {
            let bytes = global_bytes(format)?;
            if let Some(image) = decode_dib(&bytes) {
                return Ok(Some(image));
            }
        }
    }
    if IsClipboardFormatAvailable(13).is_ok() {
        let bytes = global_bytes(13)?;
        let words: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .take_while(|&c| c != 0)
            .collect();
        let text = String::from_utf16_lossy(&words);
        if !text.is_empty() {
            return Ok(Some(Payload::Text(text)));
        }
    }
    Ok(None)
}

// Accept uncompressed 24/32-bit DIBs and conventional BGRA bitfields only.
pub fn decode_dib(data: &[u8]) -> Option<Payload> {
    if data.len() < 40 {
        return None;
    }
    let u32at = |o| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    let header = u32at(0) as usize;
    let width = u32at(4) as i32;
    let signed_height = u32at(8) as i32;
    let bits = u16::from_le_bytes([data[14], data[15]]) as usize;
    let compression = u32at(16);
    if header < 40
        || header > data.len()
        || width <= 0
        || signed_height == 0
        || signed_height == i32::MIN
        || ![24, 32].contains(&bits)
        || ![0, 3].contains(&compression)
    {
        return None;
    }
    let masks = if compression == 3 && header == 40 {
        12
    } else {
        0
    };
    if compression == 3 {
        if bits != 32 || data.len() < 52 {
            return None;
        }
        if [u32at(40), u32at(44), u32at(48)] != [0x00ff0000, 0x0000ff00, 0x000000ff] {
            return None;
        }
    }
    let height = signed_height.unsigned_abs() as usize;
    let width = width as usize;
    let size = width.checked_mul(height)?.checked_mul(4)?;
    if size > ITEM_LIMIT {
        return None;
    }
    let stride = (width.checked_mul(bits)?).div_ceil(32) * 4;
    let offset = header
        .checked_add(masks)?
        .checked_add((u32at(32) as usize).checked_mul(4)?)?;
    if offset.checked_add(stride.checked_mul(height)?)? > data.len() {
        return None;
    }
    let defined_alpha = bits == 32 && header >= 56 && u32at(52) == 0xff000000;
    let mut bgra = vec![0; size];
    for y in 0..height {
        let sy = if signed_height < 0 { y } else { height - 1 - y };
        for x in 0..width {
            let from = offset + sy * stride + x * (bits / 8);
            let to = (y * width + x) * 4;
            bgra[to..to + 3].copy_from_slice(&data[from..from + 3]);
            bgra[to + 3] = if defined_alpha { data[from + 3] } else { 255 };
        }
    }
    Some(Payload::Image {
        width: width as u32,
        height: height as u32,
        bgra,
    })
}

unsafe fn set_bytes(format: u32, bytes: &[u8]) -> std::result::Result<(), String> {
    let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?;
    let ptr = GlobalLock(memory);
    if ptr.is_null() {
        let _ = GlobalFree(memory);
        return Err("Cannot allocate clipboard memory.".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    let _ = GlobalUnlock(memory);
    if let Err(e) = SetClipboardData(format, HANDLE(memory.0)) {
        let _ = GlobalFree(memory);
        return Err(e.to_string());
    }
    Ok(())
}

unsafe fn write(hwnd: HWND, payload: &Payload) -> std::result::Result<u32, String> {
    // Validate references before touching the current clipboard.
    if let Payload::Files(paths) = payload {
        if paths.iter().any(|p| !std::path::Path::new(p).exists()) {
            return Err("A copied file no longer exists. Clipboard unchanged.".into());
        }
    }
    let (format, bytes) = match payload {
        Payload::Text(text) => (
            13,
            text.encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        ),
        Payload::Files(paths) => {
            let mut bytes = vec![0u8; 20];
            bytes[..4].copy_from_slice(&20u32.to_le_bytes());
            bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
            for path in paths {
                bytes.extend(
                    path.encode_utf16()
                        .chain(Some(0))
                        .flat_map(u16::to_le_bytes),
                );
            }
            bytes.extend([0, 0]);
            (15, bytes)
        }
        Payload::Image {
            width,
            height,
            bgra,
        } => {
            let mut bytes = vec![0; 124];
            bytes[..4].copy_from_slice(&124u32.to_le_bytes());
            bytes[4..8].copy_from_slice(&width.to_le_bytes());
            bytes[8..12].copy_from_slice(&(-(*height as i32)).to_le_bytes());
            bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
            bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
            bytes[16..20].copy_from_slice(&3u32.to_le_bytes());
            for (offset, mask) in [
                (40, 0x00ff0000u32),
                (44, 0x0000ff00),
                (48, 0x000000ff),
                (52, 0xff000000),
                (56, 0x73524742),
            ] {
                bytes[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
            }
            bytes.extend(bgra);
            (17, bytes)
        }
    };
    let _guard = open(hwnd)?;
    EmptyClipboard().map_err(|e| e.to_string())?;
    set_bytes(format, &bytes)?;
    Ok(GetClipboardSequenceNumber())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dib(width: i32, height: i32, bits: u16, pixels: &[u8]) -> Vec<u8> {
        let mut b = vec![0; 40];
        b[..4].copy_from_slice(&40u32.to_le_bytes());
        b[4..8].copy_from_slice(&width.to_le_bytes());
        b[8..12].copy_from_slice(&height.to_le_bytes());
        b[12..14].copy_from_slice(&1u16.to_le_bytes());
        b[14..16].copy_from_slice(&bits.to_le_bytes());
        b.extend(pixels);
        b
    }
    #[test]
    fn converts_bottom_up_24_bit_rows_with_padding() {
        let image = decode_dib(&dib(1, 2, 24, &[1, 2, 3, 0, 4, 5, 6, 0])).unwrap();
        assert_eq!(
            image,
            Payload::Image {
                width: 1,
                height: 2,
                bgra: vec![4, 5, 6, 255, 1, 2, 3, 255]
            }
        );
    }
    #[test]
    fn accepts_top_down_32_bit_and_normalizes_undefined_alpha() {
        assert_eq!(
            decode_dib(&dib(1, -1, 32, &[1, 2, 3, 0])),
            Some(Payload::Image {
                width: 1,
                height: 1,
                bgra: vec![1, 2, 3, 255]
            })
        );
    }
    #[test]
    fn rejects_truncated_and_hostile_headers() {
        assert!(decode_dib(&[0; 12]).is_none());
        assert!(decode_dib(&dib(100, 100, 32, &[0; 4])).is_none());
        assert!(decode_dib(&dib(i32::MAX, i32::MAX, 32, &[])).is_none());
        assert!(decode_dib(&dib(1, i32::MIN, 32, &[])).is_none());
        assert!(decode_dib(&dib(-1, 1, 32, &[])).is_none());
    }
    #[test]
    fn preserves_explicit_v5_alpha() {
        let mut b = vec![0; 124];
        b[..4].copy_from_slice(&124u32.to_le_bytes());
        b[4..8].copy_from_slice(&1i32.to_le_bytes());
        b[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        b[12..14].copy_from_slice(&1u16.to_le_bytes());
        b[14..16].copy_from_slice(&32u16.to_le_bytes());
        b[16..20].copy_from_slice(&3u32.to_le_bytes());
        for (offset, mask) in [
            (40, 0x00ff0000u32),
            (44, 0x0000ff00),
            (48, 0x000000ff),
            (52, 0xff000000),
        ] {
            b[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
        }
        b.extend([10, 20, 30, 64]);
        assert_eq!(
            decode_dib(&b),
            Some(Payload::Image {
                width: 1,
                height: 1,
                bgra: vec![10, 20, 30, 64]
            })
        );
    }

    #[test]
    #[ignore = "Uses the real clipboard; run scripts/integration.ps1 to back up and restore its content"]
    fn native_worker_roundtrips_text_images_and_file_references() {
        use windows::{core::w, Win32::UI::WindowsAndMessaging::*};
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Clipboard test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let worker = Worker::start(hwnd, WM_APP + 90);
            let receive = || {
                let start = std::time::Instant::now();
                loop {
                    if let Ok(event) = worker.rx.try_recv() {
                        break event;
                    }
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    assert!(
                        start.elapsed() < Duration::from_secs(3),
                        "Clipboard worker timed out"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
            };
            let file =
                std::env::temp_dir().join(format!("clipwheel-test-{}.txt", std::process::id()));
            std::fs::write(&file, "fixture").unwrap();
            let samples = [
                Payload::Text("Round trip 🦀 ไทย\nSecond line".into()),
                Payload::Image {
                    width: 2,
                    height: 1,
                    bgra: vec![1, 2, 3, 255, 20, 30, 40, 64],
                },
                Payload::Files(vec![file.to_string_lossy().into_owned()]),
            ];
            for payload in samples {
                worker
                    .tx
                    .send(Command::Write(Arc::new(payload.clone())))
                    .unwrap();
                assert!(matches!(receive(), Event::Written(_)));
                worker.tx.send(Command::Read(17)).unwrap();
                match receive() {
                    Event::Captured(17, Some(p)) => assert_eq!(p.payload, payload),
                    _ => panic!("Expected captured clipboard data"),
                }
            }
            std::fs::remove_file(&file).unwrap();
            worker
                .tx
                .send(Command::Write(Arc::new(Payload::Files(vec![file
                    .to_string_lossy()
                    .into_owned()]))))
                .unwrap();
            assert!(matches!(receive(), Event::Notice(_)));
            worker.tx.send(Command::Stop).unwrap();
            let _ = DestroyWindow(hwnd);
        }
    }
}
