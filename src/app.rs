use crate::{
    clipboard::{self, Command, Event, Worker},
    render::{self, Frame, Renderer},
};
use clipwheel::{
    geometry::{self, PAGE_SIZE},
    model::{Entry, History},
    settings::{self, Hotkey},
};
use std::{
    cell::RefCell,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        Media::{timeBeginPeriod, timeEndPeriod},
        System::{Com::*, DataExchange::*, LibraryLoader::*, Threading::*},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
};

const TRAY: u32 = WM_APP + 1;
const WORKER: u32 = WM_APP + 2;
const FRAME: u32 = WM_APP + 3;
const ANIMATE: usize = 1;
const HOVER: usize = 2;
const PASTE: usize = 3;
const HOTKEY: i32 = 1;
const OPEN: usize = 100;
const PAUSE: usize = 101;
const CLEAR: usize = 102;
const SETTINGS: usize = 103;
const QUIT: usize = 104;
const SAVE: usize = 105;

thread_local! {static APP:RefCell<Option<App>>=const{RefCell::new(None)};}
struct App {
    hwnd: HWND,
    icon: HICON,
    renderer: Option<Renderer>,
    worker: Worker,
    history: History,
    snapshot: Vec<Arc<Entry>>,
    visible: bool,
    paused: bool,
    page: usize,
    hover: Option<usize>,
    preview: bool,
    opened: Instant,
    hovered: Instant,
    scroll: f32,
    target: HWND,
    scale: f32,
    reading: bool,
    dirty: bool,
    writing: bool,
    own_sequence: u32,
    paste_started: Option<Instant>,
    hotkey: Hotkey,
    settings: HWND,
    edit: HWND,
    taskbar_message: u32,
    epoch: u64,
    animation_timer: bool,
    timer_handle: HANDLE,
    section: usize,
    menu_target: HWND,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

impl App {
    unsafe fn animate(&mut self) {
        if !self.animation_timer {
            timeBeginPeriod(1);
            self.animation_timer = true;
            // WM_TIMER is low priority and can starve during continuous mouse input.
            // A thread-pool timer posts ordinary messages, without touching UI state.
            if CreateTimerQueueTimer(
                &mut self.timer_handle,
                None,
                Some(frame_timer),
                Some(self.hwnd.0.cast()),
                16,
                16,
                WT_EXECUTEDEFAULT,
            )
            .is_err()
            {
                SetTimer(self.hwnd, ANIMATE, 16, None);
            }
        }
    }
    unsafe fn stop_animation(&mut self) {
        let _ = KillTimer(self.hwnd, ANIMATE);
        if self.timer_handle != HANDLE::default() {
            let _ = DeleteTimerQueueTimer(None, self.timer_handle, INVALID_HANDLE_VALUE);
            self.timer_handle = HANDLE::default();
        }
        if self.animation_timer {
            timeEndPeriod(1);
            self.animation_timer = false;
        }
    }
    unsafe fn notice(&self, text: &str) {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        for (dst, src) in data.szInfo.iter_mut().take(255).zip(text.encode_utf16()) {
            *dst = src;
        }
        for (dst, src) in data.szInfoTitle.iter_mut().zip("ClipWheel".encode_utf16()) {
            *dst = src;
        }
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
    unsafe fn tray(&self, action: NOTIFY_ICON_MESSAGE) {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        let tip = format!(
            "ClipWheel — {}{}",
            self.hotkey.label(),
            if self.paused { " (paused)" } else { "" }
        );
        for (dst, src) in data.szTip.iter_mut().take(127).zip(tip.encode_utf16()) {
            *dst = src;
        }
        let _ = Shell_NotifyIconW(action, &data);
    }
    unsafe fn capture(&mut self) {
        if self.paused || self.writing || GetClipboardSequenceNumber() == self.own_sequence {
            return;
        }
        if self.reading {
            self.dirty = true;
            return;
        }
        if self.worker.tx.try_send(Command::Read(self.epoch)).is_ok() {
            self.reading = true;
            self.dirty = false;
        }
    }
    unsafe fn dismiss(&mut self) {
        self.stop_animation();
        self.visible = false;
        self.preview = false;
        self.hover = None;
        self.snapshot.clear();
        for timer in [ANIMATE, HOVER] {
            let _ = KillTimer(self.hwnd, timer);
        }
        let _ = ShowWindow(self.hwnd, SW_HIDE);
        if let Some(r) = self.renderer.as_mut() {
            r.clear_images();
        }
        if self.dirty {
            self.capture();
        }
    }
    unsafe fn toggle(&mut self) {
        if self.visible {
            self.dismiss();
            return;
        }
        if self.writing || self.paste_started.is_some() {
            return;
        }
        self.target = if self.menu_target != HWND::default() {
            let target = self.menu_target;
            self.menu_target = HWND::default();
            target
        } else {
            GetForegroundWindow()
        };
        if std::env::var_os("CLIPWHEEL_TRACE").is_some() {
            eprintln!("open target {:?}", self.target);
        }
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        let mut xdpi = 96;
        let mut ydpi = 96;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut xdpi, &mut ydpi);
        self.scale = (xdpi as f32 / 96.)
            .min((mi.rcWork.right - mi.rcWork.left) as f32 / render::WIDTH)
            .min((mi.rcWork.bottom - mi.rcWork.top) as f32 / render::HEIGHT)
            .max(0.25);
        let width = (render::WIDTH * self.scale).ceil() as i32;
        let height = (render::HEIGHT * self.scale).ceil() as i32;
        // The window reserves space to the right for previews; align the wheel, not the window, to the pointer.
        let origin = geometry::place(
            (
                cursor.x + ((render::WIDTH / 2. - render::CENTER.0) * self.scale) as i32,
                cursor.y + ((render::HEIGHT / 2. - render::CENTER.1) * self.scale) as i32,
            ),
            (
                mi.rcWork.left,
                mi.rcWork.top,
                mi.rcWork.right,
                mi.rcWork.bottom,
            ),
            (width, height),
        );
        let _ = SetWindowPos(
            self.hwnd,
            HWND_TOPMOST,
            origin.0,
            origin.1,
            width,
            height,
            SWP_NOACTIVATE,
        );
        if self
            .renderer
            .as_ref()
            .is_none_or(|r| (r.scale() - self.scale).abs() > 0.001)
        {
            self.renderer = None;
            match Renderer::new(self.hwnd, self.scale) {
                Ok(r) => self.renderer = Some(r),
                Err(e) => {
                    self.notice(&format!("Cannot initialize graphics: {e}"));
                    return;
                }
            }
        }
        self.snapshot = self.history.entries.clone();
        self.page = 0;
        self.hover = None;
        self.preview = false;
        self.scroll = 0.;
        self.section = 0;
        self.opened = Instant::now();
        self.visible = true;
        self.draw();
        let _ = ShowWindow(self.hwnd, SW_SHOW);
        let _ = SetForegroundWindow(self.hwnd);
        let _ = SetFocus(self.hwnd);
        self.animate();
    }
    unsafe fn draw(&mut self) {
        if !self.visible {
            return;
        }
        let start = self.page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(self.snapshot.len());
        if let Some(renderer) = self.renderer.as_mut() {
            let frame_start = Instant::now();
            let result = renderer.draw(Frame {
                entries: &self.snapshot[start..end],
                page: self.page,
                total_pages: geometry::pages(self.snapshot.len()),
                hover: self.hover,
                hover_amount: (self.hovered.elapsed().as_secs_f32() / 0.1).min(1.),
                preview: self.preview,
                scroll: self.scroll,
                section: self.section,
                opening: (self.opened.elapsed().as_secs_f32() / 0.16).min(1.),
            });
            self.scroll = self.scroll.min(renderer.max_scroll());
            if std::env::var_os("CLIPWHEEL_TRACE").is_some() {
                eprintln!(
                    "frame {:.3} {:.3}",
                    self.opened.elapsed().as_secs_f64() * 1000.,
                    frame_start.elapsed().as_secs_f64() * 1000.
                );
            }
            if let Err(e) = result {
                self.dismiss();
                self.renderer = None;
                self.notice(&format!("Graphics reset. Reopen the wheel. {e}"));
            }
        }
    }
    unsafe fn move_pointer(&mut self, x: f32, y: f32) {
        if std::env::var_os("CLIPWHEEL_TRACE").is_some() {
            eprintln!(
                "pointer {x} {y} visible={} hover={:?}",
                self.visible, self.hover
            );
        }
        if !self.visible {
            return;
        }
        if self.preview
            && self
                .renderer
                .as_ref()
                .is_some_and(|r| r.preview_contains(x, y))
        {
            return;
        }
        let count = self
            .snapshot
            .len()
            .saturating_sub(self.page * PAGE_SIZE)
            .min(PAGE_SIZE);
        let hover = geometry::hit_segment(x - render::CENTER.0, y - render::CENTER.1, count);
        if hover.is_none()
            && self.preview
            && self
                .renderer
                .as_ref()
                .is_some_and(|r| r.preview_bridge(x, y))
        {
            return;
        }
        if hover != self.hover {
            self.hover = hover;
            self.hovered = Instant::now();
            self.preview = false;
            self.scroll = 0.;
            self.section = 0;
            let _ = KillTimer(self.hwnd, HOVER);
            if hover.is_some() {
                SetTimer(self.hwnd, HOVER, 100, None);
            }
            self.animate();
            self.draw();
        }
    }
    unsafe fn select(&mut self, x: f32, y: f32) {
        if self.preview
            && self
                .renderer
                .as_ref()
                .is_some_and(|r| r.preview_contains(x, y))
        {
            return;
        }
        let count = self
            .snapshot
            .len()
            .saturating_sub(self.page * PAGE_SIZE)
            .min(PAGE_SIZE);
        if let Some(index) =
            geometry::hit_segment(x - render::CENTER.0, y - render::CENTER.1, count)
        {
            if let Some(entry) = self.snapshot.get(self.page * PAGE_SIZE + index) {
                match self
                    .worker
                    .tx
                    .try_send(Command::Write(entry.payload.clone()))
                {
                    Ok(()) => {
                        self.writing = true;
                        self.dismiss();
                    }
                    Err(_) => self.notice("Clipboard is busy. Try selecting again."),
                }
            }
        } else {
            self.dismiss();
        }
    }
    unsafe fn worker_event(&mut self) {
        while let Ok(event) = self.worker.rx.try_recv() {
            match event {
                Event::Captured(epoch, payload) => {
                    self.reading = false;
                    if epoch == self.epoch && !self.paused {
                        if let Some(payload) = payload {
                            if let Err(e) = self.history.insert_pinned(payload, &self.snapshot) {
                                self.notice(e);
                            }
                        }
                    }
                }
                Event::Written(sequence) => {
                    self.writing = false;
                    self.own_sequence = sequence;
                    self.dirty = false;
                    if IsWindow(self.target).as_bool() {
                        let _ = SetForegroundWindow(self.target);
                    }
                    self.paste_started = Some(Instant::now());
                    SetTimer(self.hwnd, PASTE, 15, None);
                }
                Event::Notice(text) => {
                    self.reading = false;
                    self.writing = false;
                    self.notice(&text);
                }
            }
        }
        // Read commands with no supported format still post a completion notification.
        self.reading = false;
        if self.dirty {
            self.capture();
        }
    }
    unsafe fn paste_tick(&mut self) {
        let Some(started) = self.paste_started else {
            return;
        };
        let modifiers = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            .any(|key| GetAsyncKeyState(key.0 as i32) < 0);
        if modifiers && started.elapsed() < Duration::from_millis(750) {
            return;
        }
        let _ = KillTimer(self.hwnd, PASTE);
        self.paste_started = None;
        if GetClipboardSequenceNumber() != self.own_sequence {
            self.notice("Clipboard changed before pasting. Select the item again.");
            return;
        }
        if std::env::var_os("CLIPWHEEL_TRACE").is_some() {
            eprintln!(
                "paste target {:?} foreground {:?} modifiers {modifiers}",
                self.target,
                GetForegroundWindow()
            );
        }
        if modifiers || GetForegroundWindow() != self.target {
            self.notice("Item is on the clipboard. Press Ctrl+V in the destination app.");
            return;
        }
        let key = |vk, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        };
        let input = [
            key(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
            key(VIRTUAL_KEY(0x56), KEYBD_EVENT_FLAGS(0)),
            key(VIRTUAL_KEY(0x56), KEYEVENTF_KEYUP),
            key(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        let sent = SendInput(&input, std::mem::size_of::<INPUT>() as i32);
        if std::env::var_os("CLIPWHEEL_TRACE").is_some() {
            eprintln!("injected {sent}");
        }
        if sent != 4 {
            if sent > 0 {
                let releases = [
                    key(VIRTUAL_KEY(0x56), KEYEVENTF_KEYUP),
                    key(VK_CONTROL, KEYEVENTF_KEYUP),
                ];
                SendInput(&releases, std::mem::size_of::<INPUT>() as i32);
            }
            self.notice("Automatic paste was blocked. Press Ctrl+V to paste manually.");
        }
    }
    unsafe fn menu(&mut self) {
        let previous = if self.visible {
            self.target
        } else {
            GetForegroundWindow()
        };
        self.dismiss();
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };
        for (id, label) in [
            (OPEN, "Open wheel"),
            (
                PAUSE,
                if self.paused {
                    "Resume capture"
                } else {
                    "Pause capture"
                },
            ),
            (CLEAR, "Clear history"),
            (SETTINGS, "Shortcut settings"),
            (QUIT, "Quit"),
        ] {
            let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide(label).as_ptr()));
        }
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(self.hwnd);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            self.hwnd,
            None,
        )
        .0 as usize;
        if id == OPEN {
            self.menu_target = previous;
        }
        let _ = DestroyMenu(menu);
        let _ = PostMessageW(self.hwnd, WM_COMMAND, WPARAM(id), LPARAM(0));
    }
    unsafe fn settings(&mut self) {
        self.dismiss();
        if IsWindow(self.settings).as_bool() {
            let _ = SetForegroundWindow(self.settings);
            return;
        }
        let instance = GetModuleHandleW(None).unwrap_or_default();
        self.settings = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("ClipWheelSettings"),
            w!("ClipWheel · Shortcut"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            430,
            205,
            None,
            None,
            instance,
            None,
        )
        .unwrap_or_default();
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Shortcut (Ctrl / Alt / Shift + a letter or number)"),
            WS_CHILD | WS_VISIBLE,
            20,
            20,
            380,
            24,
            self.settings,
            None,
            instance,
            None,
        );
        self.edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            PCWSTR(wide(&self.hotkey.label()).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            20,
            52,
            370,
            28,
            self.settings,
            None,
            instance,
            None,
        )
        .unwrap_or_default();
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("Save shortcut"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            250,
            104,
            140,
            30,
            self.settings,
            HMENU(SAVE as *mut _),
            instance,
            None,
        );
        let _ = ShowWindow(self.settings, SW_SHOW);
        let _ = SetForegroundWindow(self.settings);
        let _ = SetFocus(self.edit);
    }
    unsafe fn save_settings(&mut self) {
        let mut buf = [0u16; 128];
        let n = GetWindowTextW(self.edit, &mut buf);
        let Some(key) = Hotkey::parse(&String::from_utf16_lossy(&buf[..n as usize])) else {
            self.notice("Use a shortcut such as Ctrl+Alt+V or Ctrl+Shift+Q.");
            return;
        };
        // Register the replacement first so a conflict never disables the current key.
        if key != self.hotkey {
            if RegisterHotKey(
                self.hwnd,
                2,
                HOT_KEY_MODIFIERS(key.modifiers) | MOD_NOREPEAT,
                key.key,
            )
            .is_err()
            {
                self.notice("That shortcut is already used. Choose another.");
                return;
            }
            let _ = UnregisterHotKey(self.hwnd, HOTKEY);
            let _ = UnregisterHotKey(self.hwnd, 2);
            if RegisterHotKey(
                self.hwnd,
                HOTKEY,
                HOT_KEY_MODIFIERS(key.modifiers) | MOD_NOREPEAT,
                key.key,
            )
            .is_err()
            {
                let _ = RegisterHotKey(
                    self.hwnd,
                    HOTKEY,
                    HOT_KEY_MODIFIERS(self.hotkey.modifiers) | MOD_NOREPEAT,
                    self.hotkey.key,
                );
                self.notice("Shortcut registration failed.");
                return;
            }
        }
        self.hotkey = key;
        self.tray(NIM_MODIFY);
        if let Err(e) = settings::save(key) {
            self.notice(&format!(
                "Shortcut works for this session, but could not save settings: {e}"
            ));
        }
        let _ = DestroyWindow(self.settings);
        self.settings = HWND::default();
    }
    unsafe fn command(&mut self, id: usize) {
        match id {
            OPEN => self.toggle(),
            PAUSE => {
                self.epoch += 1;
                self.paused = !self.paused;
                self.tray(NIM_MODIFY);
                if !self.paused {
                    self.capture();
                }
            }
            CLEAR => {
                self.epoch += 1;
                self.dirty = false;
                self.dismiss();
                self.history.clear();
                self.notice("Clipboard history cleared.");
            }
            SETTINGS => self.settings(),
            SAVE => self.save_settings(),
            QUIT => {
                let _ = DestroyWindow(self.hwnd);
                PostQuitMessage(0);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn frame_timer(context: *mut std::ffi::c_void, _: BOOLEAN) {
    let _ = PostMessageW(HWND(context), FRAME, WPARAM(0), LPARAM(0));
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    APP.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        let Some(app) = state.as_mut() else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        if hwnd == app.settings {
            match msg {
                WM_COMMAND => {
                    app.command(w.0 & 0xffff);
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    app.settings = HWND::default();
                    return LRESULT(0);
                }
                _ => return DefWindowProcW(hwnd, msg, w, l),
            }
        }
        if msg == app.taskbar_message {
            app.tray(NIM_ADD);
            return LRESULT(0);
        }
        match msg {
            FRAME => {
                if app.animation_timer {
                    app.draw();
                    if app.opened.elapsed() >= Duration::from_millis(160)
                        && app.hovered.elapsed() >= Duration::from_millis(100)
                    {
                        app.stop_animation();
                    }
                }
            }
            WM_HOTKEY => app.toggle(),
            WM_CLIPBOARDUPDATE => app.capture(),
            WORKER => app.worker_event(),
            WM_COMMAND => app.command(w.0 & 0xffff),
            TRAY => match l.0 as u32 {
                WM_RBUTTONUP => app.menu(),
                WM_LBUTTONUP => app.toggle(),
                _ => {}
            },
            WM_MOUSEMOVE => {
                let x = (l.0 as u16 as i16) as f32 / app.scale;
                let y = ((l.0 >> 16) as u16 as i16) as f32 / app.scale;
                app.move_pointer(x, y);
            }
            WM_LBUTTONUP => {
                let x = (l.0 as u16 as i16) as f32 / app.scale;
                let y = ((l.0 >> 16) as u16 as i16) as f32 / app.scale;
                app.select(x, y);
            }
            WM_MOUSEWHEEL => {
                let delta = ((w.0 >> 16) as u16 as i16) as f32;
                let mut p = POINT {
                    x: l.0 as u16 as i16 as i32,
                    y: (l.0 >> 16) as u16 as i16 as i32,
                };
                let _ = ScreenToClient(hwnd, &mut p);
                if app.preview
                    && app.renderer.as_ref().is_some_and(|r| {
                        r.preview_contains(p.x as f32 / app.scale, p.y as f32 / app.scale)
                    })
                {
                    let next = app.scroll - delta / 120. * 60.;
                    let sections = app
                        .hover
                        .and_then(|i| app.snapshot.get(app.page * PAGE_SIZE + i))
                        .map_or(1, |e| e.section_count());
                    let max_scroll = app.renderer.as_ref().map_or(0., |r| r.max_scroll());
                    if next > max_scroll && app.section + 1 < sections {
                        app.section += 1;
                        app.scroll = 0.;
                    } else if next < 0. && app.section > 0 {
                        app.section -= 1;
                        app.scroll = f32::MAX;
                    } else {
                        app.scroll = next.clamp(0., max_scroll);
                    }
                    if std::env::var_os("CLIPWHEEL_TRACE").is_some() {
                        eprintln!("preview-scroll {}", app.scroll);
                    }
                } else {
                    let pages = geometry::pages(app.snapshot.len());
                    app.page = (app.page as isize + if delta < 0. { 1 } else { -1 })
                        .rem_euclid(pages as isize) as usize;
                    app.hover = None;
                    app.preview = false;
                    app.scroll = 0.;
                    app.section = 0;
                    if let Some(r) = app.renderer.as_mut() {
                        r.clear_images();
                    }
                }
                app.draw();
            }
            WM_KEYDOWN => {
                if w.0 == VK_ESCAPE.0 as usize {
                    app.dismiss();
                }
            }
            WM_ACTIVATE => {
                if (w.0 & 0xffff) == WA_INACTIVE as usize && app.visible {
                    app.dismiss();
                }
            }
            WM_TIMER => match w.0 {
                ANIMATE => {
                    app.draw();
                    if app.opened.elapsed() >= Duration::from_millis(160)
                        && app.hovered.elapsed() >= Duration::from_millis(100)
                    {
                        app.stop_animation();
                    }
                }
                HOVER => {
                    let _ = KillTimer(hwnd, HOVER);
                    app.preview = match (
                        app.hover
                            .and_then(|i| app.snapshot.get(app.page * PAGE_SIZE + i)),
                        app.renderer.as_mut(),
                    ) {
                        (Some(entry), Some(renderer)) => {
                            renderer.needs_preview(entry).unwrap_or(false)
                        }
                        _ => false,
                    };
                    if std::env::var_os("CLIPWHEEL_TRACE").is_some() {
                        eprintln!("preview {}", app.preview);
                    }
                    app.draw();
                }
                PASTE => app.paste_tick(),
                _ => {}
            },
            WM_PAINT => {
                let _ = ValidateRect(hwnd, None);
                app.draw();
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                app.dismiss();
                app.renderer = None;
            }
            WM_CLOSE => app.dismiss(),
            WM_DESTROY => PostQuitMessage(0),
            _ => return DefWindowProcW(hwnd, msg, w, l),
        }
        LRESULT(0)
    })
}

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let mutex = CreateMutexW(None, false, w!("Local\\ClipWheel.Instance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return Ok(());
        }
        let instance = GetModuleHandleW(None)?;
        let app_icon = LoadIconW(instance, PCWSTR(std::ptr::without_provenance(1)))?;
        let tray_icon = HICON(
            LoadImageW(
                instance,
                PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_SHARED,
            )?
            .0,
        );
        for (name, background) in [
            (w!("ClipWheelOverlay"), HBRUSH::default()),
            (w!("ClipWheelSettings"), GetSysColorBrush(COLOR_WINDOW)),
        ] {
            let class = WNDCLASSW {
                hInstance: instance.into(),
                lpszClassName: name,
                lpfnWndProc: Some(wndproc),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hIcon: app_icon,
                hbrBackground: background,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(Error::from_win32());
            }
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
            w!("ClipWheelOverlay"),
            w!("ClipWheel"),
            WS_POPUP,
            0,
            0,
            render::WIDTH as i32,
            render::HEIGHT as i32,
            None,
            None,
            instance,
            None,
        )?;
        let worker = Worker::start(hwnd, WORKER);
        let hotkey = settings::load();
        // Warm the composition device without drawing or scheduling an idle timer.
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut dpi_x = 96;
        let mut dpi_y = 96;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let renderer = Renderer::new(hwnd, dpi_x as f32 / 96.).ok();
        let app = App {
            hwnd,
            icon: tray_icon,
            renderer,
            worker,
            history: History::new(32, 128 * 1024 * 1024, clipboard::ITEM_LIMIT),
            snapshot: vec![],
            visible: false,
            paused: false,
            page: 0,
            hover: None,
            preview: false,
            opened: Instant::now(),
            hovered: Instant::now(),
            scroll: 0.,
            target: HWND::default(),
            scale: 1.,
            reading: false,
            dirty: false,
            writing: false,
            own_sequence: 0,
            paste_started: None,
            hotkey,
            settings: HWND::default(),
            edit: HWND::default(),
            taskbar_message: RegisterWindowMessageW(w!("TaskbarCreated")),
            epoch: 0,
            animation_timer: false,
            timer_handle: HANDLE::default(),
            section: 0,
            menu_target: HWND::default(),
        };
        app.tray(NIM_ADD);
        if RegisterHotKey(
            hwnd,
            HOTKEY,
            HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT,
            hotkey.key,
        )
        .is_err()
        {
            app.notice("Shortcut is already used. Right-click the tray icon to change it.");
        }
        AddClipboardFormatListener(hwnd)?;
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        // --demo is a non-destructive renderer smoke test; it never reads or writes the system clipboard.
        if std::env::args().any(|a| a == "--demo") {
            APP.with(|cell| {
                let mut s = cell.borrow_mut();
                let app = s.as_mut().unwrap();
                app.paused = true;
                for text in [
                    "Ready",
                    "Design with intention.",
                    "cargo build --release",
                    "https://rust-lang.org",
                    "A clipboard, reimagined.",
                    "Meet at 14:30",
                    "Hi",
                    "Compact clipboard preview\n\nOnly overflowing items open this panel.\nScroll to read longer content.\n\n01  Design with intention.\n02  Keep the typography consistent.\n03  Make the selected item clear.\n04  Preserve the clipboard workflow.\n05  Use quiet lavender highlights.\n06  Keep text readable.\n07  Preview only when needed.\n08  Click a slice to paste.\n09  Escape closes the wheel.\n10  Your clipboard stays local.\n11  Longer previews remain scrollable.\n12  Keep each item easy to recognize.\n13  Text stays inside its slice.\n14  Use a consistent reading size.\n15  Let the content set the hierarchy.\n16  Images preserve their aspect ratio.\n17  File paths remain accessible.\n18  Short items need no popup.\n19  Keep the wheel responsive.\n20  End of preview.",
                ] {
                    let _ = app
                        .history
                        .insert(clipwheel::model::Payload::Text(text.into()));
                }
                app.toggle();
            });
        }
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 == 0 {
                break;
            }
            if result.0 < 0 {
                return Err(Error::from_win32());
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        APP.with(|cell| {
            if let Some(mut app) = cell.borrow_mut().take() {
                app.stop_animation();
                app.tray(NIM_DELETE);
                let _ = RemoveClipboardFormatListener(hwnd);
                let _ = UnregisterHotKey(hwnd, HOTKEY);
                let _ = app.worker.tx.try_send(Command::Stop);
            }
        });
        let _ = CloseHandle(mutex);
        CoUninitialize();
        Ok(())
    }
}
