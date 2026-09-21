pub type PlaybackUpdate = (f64, f64, bool, bool, usize, usize, String);

#[cfg(windows)]
mod native {
    use std::{
        ffi::c_void,
        io::{Read, Write},
        mem::{size_of, zeroed},
        net::TcpStream,
        ptr::{null, null_mut},
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    use tokio_util::sync::CancellationToken;
    use windows_sys::Win32::{
        Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
        Graphics::Gdi::{
            BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
            CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET,
            DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
            DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, Ellipse,
            EndPaint, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, FillRect, GetMonitorInfoW,
            InvalidateRect, LineTo, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            MoveToEx, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SRCCOPY, SelectObject, SetBkMode,
            SetTextColor, TRANSPARENT,
        },
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            LibraryLoader::GetModuleHandleW,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        },
        UI::{
            Controls::Dialogs::{
                GetOpenFileNameW, OFN_EXPLORER, OFN_NOVALIDATE, OFN_PATHMUSTEXIST, OPENFILENAMEW,
            },
            Input::KeyboardAndMouse::{
                ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
            },
            Shell::{
                DragAcceptFiles, DragFinish, DragQueryFileW, HDROP, NIF_ICON, NIF_MESSAGE,
                NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT,
                NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateIcon,
                CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
                DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect, GetCursorPos,
                GetMessageW, GetWindowLongPtrW, HICON, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_HAND,
                KillTimer, LoadCursorW, MF_DISABLED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
                PostQuitMessage, RegisterClassW, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE,
                SWP_NOACTIVATE, SendMessageW, SetCursor, SetForegroundWindow, SetTimer,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
                TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE,
                WM_DESTROY, WM_DROPFILES, WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
                WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONUP, WM_SETICON, WM_TIMER,
                WNDCLASSW, WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
                WS_SYSMENU,
            },
        },
    };

    const TRAY_MESSAGE: u32 = WM_APP + 1;
    const NOTIFICATION_MESSAGE: u32 = WM_APP + 2;
    const PLAYBACK_MESSAGE: u32 = WM_APP + 3;
    const WM_MOUSE_LEAVE: u32 = 0x02a3;
    const TIMER_HIDE: usize = 2;
    const TIMER_MANAGER: usize = 3;
    const CMD_OPEN: usize = 100;
    const CMD_COPY: usize = 101;
    const CMD_HIDE: usize = 102;
    const CMD_STOP: usize = 103;
    const CMD_MANAGER: usize = 104;
    const CMD_LOG: usize = 105;
    const MANAGER_COPY: i32 = 1;
    const MANAGER_PLAY: i32 = 2;
    const MANAGER_OVERLAY: i32 = 3;
    const MANAGER_STOP: i32 = 4;
    const MANAGER_EXIT: i32 = 5;
    const MANAGER_NEXT: i32 = 7;
    const MANAGER_SELECT: i32 = 8;
    const CF_UNICODETEXT: u32 = 13;
    const HWND_TOPMOST: HWND = -1isize as HWND;
    static POPUP_TEXT: OnceLock<Mutex<Vec<u16>>> = OnceLock::new();
    static POPUP_HOVER: AtomicU8 = AtomicU8::new(0);
    static POPUP_ERROR: AtomicBool = AtomicBool::new(false);
    static MANAGER_PATH: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    static MANAGER_STATUS: OnceLock<Mutex<String>> = OnceLock::new();
    static MANAGER_HOVER: AtomicU8 = AtomicU8::new(0);
    static MANAGER_PRESSED: AtomicU8 = AtomicU8::new(0);
    static PLAYBACK_TIME: AtomicU64 = AtomicU64::new(0);
    static PLAYBACK_DURATION: AtomicU64 = AtomicU64::new(0);
    static PLAYBACK_PLAYING: AtomicBool = AtomicBool::new(false);
    static FOLDER_PLAYING: AtomicBool = AtomicBool::new(false);
    static FOLDER_INDEX: AtomicUsize = AtomicUsize::new(0);
    static FOLDER_TOTAL: AtomicUsize = AtomicUsize::new(0);
    static PLAYBACK_FILE: OnceLock<Mutex<String>> = OnceLock::new();
    static SCRUB_PREVIEW: AtomicU64 = AtomicU64::new(u64::MAX);

    struct Context {
        base: String,
        cancel: CancellationToken,
        popup: HWND,
        manager: HWND,
        icon: HICON,
    }
    unsafe impl Send for Context {}

    pub struct Tray {
        hwnd: Arc<Mutex<isize>>,
    }
    impl Drop for Tray {
        fn drop(&mut self) {
            let hwnd = *self.hwnd.lock().unwrap();
            if hwnd != 0 {
                unsafe {
                    PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
                }
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn fill<const N: usize>(target: &mut [u16; N], value: &str) {
        for (slot, ch) in target.iter_mut().zip(value.encode_utf16()) {
            *slot = ch;
        }
    }
    unsafe fn create_app_icon(instance: *mut c_void) -> HICON {
        const SIZE: usize = 32;
        let and_mask = [0u8; SIZE * SIZE / 8];
        let mut pixels = [0u8; SIZE * SIZE * 4];
        for y in 0..SIZE {
            for x in 0..SIZE {
                let source_y = SIZE - 1 - y;
                let offset = (source_y * SIZE + x) * 4;
                let edge = !(2..=29).contains(&x) || !(2..=29).contains(&y);
                let corner = (!(6..=25).contains(&x) && !(6..=25).contains(&y))
                    && ((x as isize - if x < 16 { 6 } else { 25 }).pow(2)
                        + (y as isize - if y < 16 { 6 } else { 25 }).pow(2)
                        > 16);
                let in_play = (9..=23).contains(&x)
                    && (7..=24).contains(&y)
                    && (y as isize - 16).unsigned_abs() <= (23 - x) / 2;
                let (red, green, blue, alpha) = if edge || corner {
                    (0, 0, 0, 0)
                } else if in_play {
                    (255, 255, 255, 255)
                } else {
                    (0, 120, 212, 255)
                };
                pixels[offset..offset + 4].copy_from_slice(&[blue, green, red, alpha]);
            }
        }
        unsafe {
            CreateIcon(
                instance,
                SIZE as i32,
                SIZE as i32,
                1,
                32,
                and_mask.as_ptr(),
                pixels.as_ptr(),
            )
        }
    }
    fn http_get(base: &str, path: &str) -> Option<String> {
        let address = base.strip_prefix("http://")?;
        let mut stream = TcpStream::connect(address).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
    }
    unsafe fn context(hwnd: HWND) -> Option<&'static mut Context> {
        unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context).as_mut() }
    }
    unsafe fn copy_text(hwnd: HWND, text: &str) {
        let data = wide(text);
        if unsafe { OpenClipboard(hwnd) } == 0 {
            return;
        }
        unsafe {
            EmptyClipboard();
        }
        let handle: HGLOBAL = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len() * 2) };
        if !handle.is_null() {
            let memory = unsafe { GlobalLock(handle) } as *mut u16;
            if !memory.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), memory, data.len());
                    GlobalUnlock(handle);
                    SetClipboardData(CF_UNICODETEXT, handle as _);
                }
            }
        }
        unsafe {
            CloseClipboard();
        }
    }
    unsafe fn open_url(url: &str) {
        let verb = wide("open");
        let url = wide(url);
        unsafe {
            ShellExecuteW(null_mut(), verb.as_ptr(), url.as_ptr(), null(), null(), 1);
        }
    }
    unsafe fn show_popup(hwnd: HWND, message: &str, error: bool) {
        POPUP_HOVER.store(0, Ordering::Relaxed);
        POPUP_ERROR.store(error, Ordering::Relaxed);
        *POPUP_TEXT
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap() = message.encode_utf16().chain(Some(0)).collect();
        let mut cursor: POINT = unsafe { zeroed() };
        unsafe {
            GetCursorPos(&mut cursor);
        }
        let mut info: MONITORINFO = unsafe { zeroed() };
        info.cbSize = size_of::<MONITORINFO>() as u32;
        unsafe {
            GetMonitorInfoW(
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
                &mut info,
            );
        }
        let (width, height) = (420, 128);
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                info.rcWork.right - width - 20,
                info.rcWork.bottom - height - 20,
                width,
                height,
                SWP_NOACTIVATE,
            );
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            InvalidateRect(hwnd, null(), 1);
            SetTimer(hwnd, TIMER_HIDE, 6000, None);
        }
    }
    unsafe extern "system" fn popup_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_PAINT => {
                let mut paint: PAINTSTRUCT = unsafe { zeroed() };
                let dc = unsafe { BeginPaint(hwnd, &mut paint) };
                let brush = unsafe { CreateSolidBrush(0x00242120) };
                unsafe {
                    FillRect(dc, &paint.rcPaint, brush);
                    DeleteObject(brush);
                    let accent = CreateSolidBrush(if POPUP_ERROR.load(Ordering::Relaxed) {
                        0x003834d1
                    } else {
                        0x00d47800
                    });
                    FillRect(
                        dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: 4,
                            bottom: 128,
                        },
                        accent,
                    );
                    DeleteObject(accent);
                    let hover = POPUP_HOVER.load(Ordering::Relaxed);
                    if hover != 0 {
                        let hover_brush = CreateSolidBrush(0x003a3836);
                        let hover_rect = if hover == 1 {
                            RECT {
                                left: 340,
                                top: 5,
                                right: 378,
                                bottom: 45,
                            }
                        } else {
                            RECT {
                                left: 378,
                                top: 5,
                                right: 418,
                                bottom: 45,
                            }
                        };
                        FillRect(dc, &hover_rect, hover_brush);
                        DeleteObject(hover_brush);
                    }
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(dc, 0x00ffffff);
                }
                let face = wide("Segoe UI");
                let title = wide("obs-video-trigger");
                let mut title_rect = RECT {
                    left: 20,
                    top: 17,
                    right: 326,
                    bottom: 42,
                };
                let title_font = unsafe {
                    CreateFontW(
                        -16,
                        0,
                        0,
                        0,
                        FW_SEMIBOLD as i32,
                        0,
                        0,
                        0,
                        DEFAULT_CHARSET as u32,
                        OUT_DEFAULT_PRECIS as u32,
                        CLIP_DEFAULT_PRECIS as u32,
                        CLEARTYPE_QUALITY as u32,
                        (DEFAULT_PITCH | FF_DONTCARE) as u32,
                        face.as_ptr(),
                    )
                };
                unsafe {
                    let old_font = SelectObject(dc, title_font);
                    DrawTextW(
                        dc,
                        title.as_ptr(),
                        -1,
                        &mut title_rect,
                        DT_LEFT | DT_NOPREFIX,
                    );
                    SelectObject(dc, old_font);
                    DeleteObject(title_font);
                }
                if let Some(text) = POPUP_TEXT.get() {
                    let text = text.lock().unwrap();
                    let mut body = RECT {
                        left: 20,
                        top: 51,
                        right: 398,
                        bottom: 113,
                    };
                    let body_font = unsafe {
                        CreateFontW(
                            -17,
                            0,
                            0,
                            0,
                            FW_NORMAL as i32,
                            0,
                            0,
                            0,
                            DEFAULT_CHARSET as u32,
                            OUT_DEFAULT_PRECIS as u32,
                            CLIP_DEFAULT_PRECIS as u32,
                            CLEARTYPE_QUALITY as u32,
                            (DEFAULT_PITCH | FF_DONTCARE) as u32,
                            face.as_ptr(),
                        )
                    };
                    unsafe {
                        let old_font = SelectObject(dc, body_font);
                        DrawTextW(
                            dc,
                            text.as_ptr(),
                            -1,
                            &mut body,
                            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                        );
                        SelectObject(dc, old_font);
                        DeleteObject(body_font);
                    }
                }
                let pen = unsafe { CreatePen(PS_SOLID, 2, 0x00d8d8d8) };
                unsafe {
                    let old_pen = SelectObject(dc, pen);
                    MoveToEx(dc, 387, 19, null_mut());
                    LineTo(dc, 399, 31);
                    MoveToEx(dc, 399, 19, null_mut());
                    LineTo(dc, 387, 31);
                    MoveToEx(dc, 351, 21, null_mut());
                    LineTo(dc, 363, 21);
                    LineTo(dc, 363, 33);
                    LineTo(dc, 351, 33);
                    LineTo(dc, 351, 21);
                    MoveToEx(dc, 355, 17, null_mut());
                    LineTo(dc, 367, 17);
                    LineTo(dc, 367, 29);
                    SelectObject(dc, old_pen);
                    DeleteObject(pen);
                }
                unsafe {
                    EndPaint(hwnd, &paint);
                }
                0
            }
            WM_MOUSEMOVE => {
                let x = (lparam as u32 & 0xffff) as i16 as i32;
                let y = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
                let hover = if (340..=377).contains(&x) && (0..=45).contains(&y) {
                    1
                } else if (378..=419).contains(&x) && (0..=45).contains(&y) {
                    2
                } else {
                    0
                };
                let previous = POPUP_HOVER.swap(hover, Ordering::Relaxed);
                unsafe {
                    SetCursor(LoadCursorW(
                        null_mut(),
                        if hover == 0 { IDC_ARROW } else { IDC_HAND },
                    ));
                    if hover != previous {
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                let x = (lparam as u32 & 0xffff) as i16 as i32;
                let y = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
                if (378..=419).contains(&x) && (0..=45).contains(&y) {
                    unsafe {
                        KillTimer(hwnd, TIMER_HIDE);
                        ShowWindow(hwnd, SW_HIDE);
                    }
                } else if (340..=377).contains(&x)
                    && (0..=45).contains(&y)
                    && let Some(text) = POPUP_TEXT.get()
                {
                    let text = String::from_utf16_lossy(&text.lock().unwrap());
                    unsafe {
                        copy_text(hwnd, text.trim_end_matches('\0'));
                    }
                }
                0
            }
            WM_TIMER if wparam == TIMER_HIDE => {
                unsafe {
                    ShowWindow(hwnd, SW_HIDE);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }
    fn manager_url(ctx: &Context) -> Option<String> {
        let path = MANAGER_PATH
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap();
        let path = path.as_ref()?;
        let encoded: String = url::form_urlencoded::byte_serialize(path.as_bytes()).collect();
        let endpoint = if std::path::Path::new(path).is_dir() {
            "play-folder?folder"
        } else {
            "play?file"
        };
        Some(format!("{}/{endpoint}={encoded}", ctx.base))
    }

    unsafe fn select_path(hwnd: HWND) -> Option<String> {
        let mut buffer = vec![0u16; 32768];
        let folder_choice = "Select this folder";
        let initial = wide(folder_choice);
        buffer[..initial.len()].copy_from_slice(&initial);
        let filter = wide("Video files\0*.mp4;*.webm;*.mov;*.m4v;*.mkv;*.ogv\0All files\0*.*\0");
        let title = wide("Select a video file or the current folder");
        let mut dialog: OPENFILENAMEW = unsafe { zeroed() };
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = buffer.len() as u32;
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_EXPLORER | OFN_NOVALIDATE | OFN_PATHMUSTEXIST;
        if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
            return None;
        }
        let length = buffer.iter().position(|&ch| ch == 0).unwrap_or(0);
        let selected = String::from_utf16_lossy(&buffer[..length]);
        let path = std::path::Path::new(&selected);
        if path.is_file() || path.is_dir() {
            return Some(selected);
        }
        if path.file_name().and_then(|name| name.to_str()) == Some(folder_choice) {
            return path
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned());
        }
        None
    }

    fn apply_selected_path(hwnd: HWND, selected: String) {
        let is_file = std::path::Path::new(&selected).is_file();
        *MANAGER_PATH
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(selected.clone());
        let active = FOLDER_PLAYING.load(Ordering::Relaxed)
            || PLAYBACK_DURATION.load(Ordering::Relaxed) != 0;
        if is_file && active {
            if let Some(ctx) = unsafe { context(hwnd) }
                && let Some(url) = manager_url(ctx)
                && let Some(endpoint) = url.strip_prefix(&ctx.base)
            {
                http_get(&ctx.base, endpoint);
                crate::diagnostics::log(&format!("manager selection auto-play file={selected}"));
                set_manager_status("Selected video is now playing.");
            }
        } else {
            set_manager_status("Selection ready — trigger URL created.");
        }
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }

    fn breakable_text(value: &str) -> String {
        let mut output = String::with_capacity(value.len() + value.len() / 3);
        let mut escaped = 0;
        let mut column = 0;
        for ch in value.chars() {
            output.push(ch);
            column += 1;
            if escaped > 0 {
                escaped -= 1;
            } else if ch == '%' {
                escaped = 2;
            }
            if column >= 76 && escaped == 0 {
                output.push('\n');
                column = 0;
            }
        }
        output
    }

    fn manager_action(x: i32, y: i32) -> u8 {
        if (710..=840).contains(&x) && (20..=62).contains(&y) {
            return MANAGER_SELECT as u8;
        }
        if (710..=840).contains(&x) && (170..=212).contains(&y) {
            return MANAGER_COPY as u8;
        }
        if (24..=840).contains(&x) && (238..=278).contains(&y) {
            return 6;
        }
        if !(357..=399).contains(&y) {
            return 0;
        }
        if (24..=164).contains(&x) {
            MANAGER_PLAY as u8
        } else if (174..=314).contains(&x) {
            MANAGER_STOP as u8
        } else if FOLDER_PLAYING.load(Ordering::Relaxed) && (324..=464).contains(&x) {
            MANAGER_NEXT as u8
        } else if (600..=730).contains(&x) {
            MANAGER_OVERLAY as u8
        } else if (760..=840).contains(&x) {
            MANAGER_EXIT as u8
        } else {
            0
        }
    }

    fn set_manager_status(message: &str) {
        *MANAGER_STATUS
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap() = message.into();
    }

    unsafe fn show_manager(hwnd: HWND) {
        let mut cursor: POINT = unsafe { zeroed() };
        unsafe {
            GetCursorPos(&mut cursor);
        }
        let mut info: MONITORINFO = unsafe { zeroed() };
        info.cbSize = size_of::<MONITORINFO>() as u32;
        unsafe {
            GetMonitorInfoW(
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
                &mut info,
            );
            SetWindowPos(
                hwnd,
                null_mut(),
                info.rcWork.left + (info.rcWork.right - info.rcWork.left - 880) / 2,
                info.rcWork.top + (info.rcWork.bottom - info.rcWork.top - 470) / 2,
                880,
                470,
                0,
            );
            ShowWindow(hwnd, SW_SHOW);
            SetTimer(hwnd, TIMER_MANAGER, 33, None);
            SetForegroundWindow(hwnd);
        }
    }

    unsafe fn manager_button(dc: *mut c_void, id: u8, rect: RECT, label: &str) {
        let hover = MANAGER_HOVER.load(Ordering::Relaxed) == id;
        let pressed = MANAGER_PRESSED.load(Ordering::Relaxed) == id;
        let primary = id == MANAGER_PLAY as u8;
        let color = match (primary, hover, pressed) {
            (_, _, true) => 0x002d2c2b,
            (true, true, false) => 0x00e58816,
            (true, false, false) => 0x00d47800,
            (false, true, false) => 0x00504e4c,
            (false, false, false) => 0x00403e3c,
        };
        let brush = unsafe { CreateSolidBrush(color) };
        unsafe {
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
        }
        let label = wide(label);
        let mut text_rect = rect;
        unsafe {
            DrawTextW(
                dc,
                label.as_ptr(),
                -1,
                &mut text_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }

    unsafe extern "system" fn manager_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_CREATE => {
                let create = lparam as *const CREATESTRUCTW;
                let dark = 1i32;
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
                    DragAcceptFiles(hwnd, 1);
                    DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                        &dark as *const i32 as *const c_void,
                        size_of::<i32>() as u32,
                    );
                }
                0
            }
            WM_DROPFILES => {
                let drop = wparam as HDROP;
                let length = unsafe { DragQueryFileW(drop, 0, null_mut(), 0) };
                if length > 0 {
                    let mut path = vec![0u16; length as usize + 1];
                    unsafe {
                        DragQueryFileW(drop, 0, path.as_mut_ptr(), path.len() as u32);
                    }
                    let dropped = String::from_utf16_lossy(&path[..length as usize]);
                    apply_selected_path(hwnd, dropped);
                }
                unsafe {
                    DragFinish(drop);
                }
                0
            }
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut paint: PAINTSTRUCT = unsafe { zeroed() };
                let screen_dc = unsafe { BeginPaint(hwnd, &mut paint) };
                let mut client: RECT = unsafe { zeroed() };
                unsafe { GetClientRect(hwnd, &mut client) };
                let dc = unsafe { CreateCompatibleDC(screen_dc) };
                let bitmap =
                    unsafe { CreateCompatibleBitmap(screen_dc, client.right, client.bottom) };
                let old_bitmap = unsafe { SelectObject(dc, bitmap) };
                let background = unsafe { CreateSolidBrush(0x00242120) };
                unsafe {
                    FillRect(dc, &client, background);
                    DeleteObject(background);
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(dc, 0x00ffffff);
                }
                let face = wide("Segoe UI");
                let font = unsafe {
                    CreateFontW(
                        -17,
                        0,
                        0,
                        0,
                        FW_NORMAL as i32,
                        0,
                        0,
                        0,
                        DEFAULT_CHARSET as u32,
                        OUT_DEFAULT_PRECIS as u32,
                        CLIP_DEFAULT_PRECIS as u32,
                        CLEARTYPE_QUALITY as u32,
                        (DEFAULT_PITCH | FF_DONTCARE) as u32,
                        face.as_ptr(),
                    )
                };
                let old_font = unsafe { SelectObject(dc, font) };
                let heading_font = unsafe {
                    CreateFontW(
                        -22,
                        0,
                        0,
                        0,
                        FW_SEMIBOLD as i32,
                        0,
                        0,
                        0,
                        DEFAULT_CHARSET as u32,
                        OUT_DEFAULT_PRECIS as u32,
                        CLIP_DEFAULT_PRECIS as u32,
                        CLEARTYPE_QUALITY as u32,
                        (DEFAULT_PITCH | FF_DONTCARE) as u32,
                        face.as_ptr(),
                    )
                };
                let heading = wide("Drop a video here");
                let mut heading_rect = RECT {
                    left: 24,
                    top: 22,
                    right: 840,
                    bottom: 52,
                };
                let help = wide(
                    "The manager uses the real Windows file path to create a reusable trigger URL.",
                );
                let mut help_rect = RECT {
                    left: 24,
                    top: 54,
                    right: 840,
                    bottom: 82,
                };
                unsafe {
                    SelectObject(dc, heading_font);
                    DrawTextW(
                        dc,
                        heading.as_ptr(),
                        -1,
                        &mut heading_rect,
                        DT_LEFT | DT_NOPREFIX,
                    );
                    SelectObject(dc, font);
                    manager_button(
                        dc,
                        MANAGER_SELECT as u8,
                        RECT {
                            left: 710,
                            top: 20,
                            right: 840,
                            bottom: 62,
                        },
                        "＋  Select…",
                    );
                    SelectObject(dc, heading_font);
                    let current = f64::from_bits(PLAYBACK_TIME.load(Ordering::Relaxed));
                    let duration = f64::from_bits(PLAYBACK_DURATION.load(Ordering::Relaxed));
                    let track = RECT {
                        left: 24,
                        top: 245,
                        right: 840,
                        bottom: 251,
                    };
                    let track_brush = CreateSolidBrush(0x00504e4c);
                    FillRect(dc, &track, track_brush);
                    DeleteObject(track_brush);
                    if duration > 0.0 {
                        let preview = SCRUB_PREVIEW.load(Ordering::Relaxed);
                        let handle_time = if preview == u64::MAX {
                            current
                        } else {
                            f64::from_bits(preview)
                        };
                        let handle_x =
                            24 + ((816.0 * handle_time / duration).clamp(0.0, 816.0) as i32);
                        let progress = RECT {
                            left: 24,
                            top: 245,
                            right: 24 + ((816.0 * current / duration).clamp(0.0, 816.0) as i32),
                            bottom: 251,
                        };
                        let progress_brush = CreateSolidBrush(0x00d47800);
                        FillRect(dc, &progress, progress_brush);
                        let handle_brush = CreateSolidBrush(0x00ffffff);
                        let old_brush = SelectObject(dc, handle_brush);
                        Ellipse(dc, handle_x - 7, 241, handle_x + 7, 255);
                        SelectObject(dc, old_brush);
                        DeleteObject(handle_brush);
                        DeleteObject(progress_brush);
                    }
                    let timing = wide(&format!(
                        "{}:{:02} / {}:{:02}",
                        (current / 60.0) as u64,
                        current as u64 % 60,
                        (duration / 60.0) as u64,
                        duration as u64 % 60
                    ));
                    let mut timing_rect = RECT {
                        left: 24,
                        top: 258,
                        right: 840,
                        bottom: 288,
                    };
                    SetTextColor(dc, 0x00b0b0b0);
                    DrawTextW(
                        dc,
                        timing.as_ptr(),
                        -1,
                        &mut timing_rect,
                        DT_RIGHT | DT_NOPREFIX,
                    );
                    let folder_index = FOLDER_INDEX.load(Ordering::Relaxed);
                    let folder_total = FOLDER_TOTAL.load(Ordering::Relaxed);
                    if folder_index > 0 && folder_total > 0 {
                        let position = wide(&format!("Video {folder_index}/{folder_total}"));
                        DrawTextW(
                            dc,
                            position.as_ptr(),
                            -1,
                            &mut timing_rect,
                            DT_LEFT | DT_NOPREFIX,
                        );
                    }
                    let playback_file = PLAYBACK_FILE
                        .get_or_init(|| Mutex::new(String::new()))
                        .lock()
                        .unwrap()
                        .clone();
                    if !playback_file.is_empty() {
                        let filename = std::path::Path::new(&playback_file)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        let filename = wide(&filename);
                        let mut filename_rect = RECT {
                            left: 190,
                            top: 258,
                            right: 674,
                            bottom: 288,
                        };
                        SelectObject(dc, font);
                        SetTextColor(dc, 0x00d8d8d8);
                        DrawTextW(
                            dc,
                            filename.as_ptr(),
                            -1,
                            &mut filename_rect,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                        );
                    }
                    SelectObject(dc, font);
                    DeleteObject(heading_font);
                    SetTextColor(dc, 0x00c8c8c8);
                    DrawTextW(dc, help.as_ptr(), -1, &mut help_rect, DT_LEFT | DT_NOPREFIX);
                }
                let path = MANAGER_PATH
                    .get_or_init(|| Mutex::new(None))
                    .lock()
                    .unwrap()
                    .clone();
                let (path_text, url_text) = if let Some(path) = path {
                    let url = unsafe { context(hwnd) }.and_then(|ctx| manager_url(ctx));
                    (path, url.unwrap_or_default())
                } else {
                    (
                        "No video selected yet.".into(),
                        "Drop a file anywhere in this window.".into(),
                    )
                };
                let file_label = wide("VIDEO FILE");
                let url_label = wide("TRIGGER URL");
                let path_text = wide(&breakable_text(&path_text));
                let url_text = wide(&breakable_text(&url_text));
                let mut file_label_rect = RECT {
                    left: 24,
                    top: 88,
                    right: 840,
                    bottom: 108,
                };
                let mut path_rect = RECT {
                    left: 24,
                    top: 111,
                    right: 840,
                    bottom: 145,
                };
                let mut url_label_rect = RECT {
                    left: 24,
                    top: 150,
                    right: 690,
                    bottom: 170,
                };
                let mut url_rect = RECT {
                    left: 24,
                    top: 173,
                    right: 690,
                    bottom: 220,
                };
                unsafe {
                    SetTextColor(dc, 0x00b0b0b0);
                    DrawTextW(
                        dc,
                        file_label.as_ptr(),
                        -1,
                        &mut file_label_rect,
                        DT_LEFT | DT_NOPREFIX,
                    );
                    SetTextColor(dc, 0x00ffffff);
                    DrawTextW(
                        dc,
                        path_text.as_ptr(),
                        -1,
                        &mut path_rect,
                        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                    );
                    SetTextColor(dc, 0x00b0b0b0);
                    DrawTextW(
                        dc,
                        url_label.as_ptr(),
                        -1,
                        &mut url_label_rect,
                        DT_LEFT | DT_NOPREFIX,
                    );
                    SetTextColor(dc, 0x00d8d8d8);
                    DrawTextW(
                        dc,
                        url_text.as_ptr(),
                        -1,
                        &mut url_rect,
                        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                    );
                    SetTextColor(dc, 0x00ffffff);
                    manager_button(
                        dc,
                        MANAGER_COPY as u8,
                        RECT {
                            left: 710,
                            top: 170,
                            right: 840,
                            bottom: 212,
                        },
                        "⧉  Copy URL",
                    );
                    manager_button(
                        dc,
                        MANAGER_PLAY as u8,
                        RECT {
                            left: 24,
                            top: 357,
                            right: 164,
                            bottom: 399,
                        },
                        if PLAYBACK_DURATION.load(Ordering::Relaxed) != 0 {
                            if PLAYBACK_PLAYING.load(Ordering::Relaxed) {
                                "Ⅱ  Pause"
                            } else {
                                "▶  Resume"
                            }
                        } else {
                            "▶  Play"
                        },
                    );
                    manager_button(
                        dc,
                        MANAGER_STOP as u8,
                        RECT {
                            left: 174,
                            top: 357,
                            right: 314,
                            bottom: 399,
                        },
                        "■  Stop",
                    );
                    if FOLDER_PLAYING.load(Ordering::Relaxed) {
                        manager_button(
                            dc,
                            MANAGER_NEXT as u8,
                            RECT {
                                left: 324,
                                top: 357,
                                right: 464,
                                bottom: 399,
                            },
                            "▶│  Next",
                        );
                    }
                    manager_button(
                        dc,
                        MANAGER_OVERLAY as u8,
                        RECT {
                            left: 600,
                            top: 357,
                            right: 730,
                            bottom: 399,
                        },
                        "↗  Overlay",
                    );
                    manager_button(
                        dc,
                        MANAGER_EXIT as u8,
                        RECT {
                            left: 760,
                            top: 357,
                            right: 840,
                            bottom: 399,
                        },
                        "⏻  Exit",
                    );
                    let status = MANAGER_STATUS
                        .get_or_init(|| Mutex::new(String::new()))
                        .lock()
                        .unwrap()
                        .clone();
                    if !status.is_empty() {
                        let status = wide(&status);
                        let mut status_rect = RECT {
                            left: 24,
                            top: 294,
                            right: 840,
                            bottom: 320,
                        };
                        SetTextColor(dc, 0x00e7a66a);
                        DrawTextW(
                            dc,
                            status.as_ptr(),
                            -1,
                            &mut status_rect,
                            DT_LEFT | DT_NOPREFIX,
                        );
                    }
                    let sections = [("VIDEO", 24, 464), ("TOOLS", 600, 730), ("APP", 760, 840)];
                    SetTextColor(dc, 0x00a0a0a0);
                    for (label, left, right) in sections {
                        let label = wide(label);
                        let mut rect = RECT {
                            left,
                            top: 334,
                            right,
                            bottom: 354,
                        };
                        DrawTextW(dc, label.as_ptr(), -1, &mut rect, DT_LEFT | DT_NOPREFIX);
                    }
                    SelectObject(dc, old_font);
                    DeleteObject(font);
                    BitBlt(
                        screen_dc,
                        paint.rcPaint.left,
                        paint.rcPaint.top,
                        paint.rcPaint.right - paint.rcPaint.left,
                        paint.rcPaint.bottom - paint.rcPaint.top,
                        dc,
                        paint.rcPaint.left,
                        paint.rcPaint.top,
                        SRCCOPY,
                    );
                    SelectObject(dc, old_bitmap);
                    DeleteObject(bitmap);
                    DeleteDC(dc);
                    EndPaint(hwnd, &paint);
                }
                0
            }
            WM_MOUSEMOVE => {
                let x = (lparam as u32 & 0xffff) as i16 as i32;
                let y = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
                let action = manager_action(x, y);
                if MANAGER_PRESSED.load(Ordering::Relaxed) == 6 {
                    let duration = f64::from_bits(PLAYBACK_DURATION.load(Ordering::Relaxed));
                    let time = duration * ((x - 24) as f64 / 816.0).clamp(0.0, 1.0);
                    SCRUB_PREVIEW.store(time.to_bits(), Ordering::Relaxed);
                    unsafe {
                        InvalidateRect(
                            hwnd,
                            &RECT {
                                left: 16,
                                top: 232,
                                right: 848,
                                bottom: 289,
                            },
                            0,
                        );
                    }
                }
                let previous = MANAGER_HOVER.swap(action, Ordering::Relaxed);
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    TrackMouseEvent(&mut tracking);
                    SetCursor(LoadCursorW(
                        null_mut(),
                        if action == 0 { IDC_ARROW } else { IDC_HAND },
                    ));
                    if action != previous {
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_MOUSE_LEAVE => {
                MANAGER_HOVER.store(0, Ordering::Relaxed);
                unsafe { InvalidateRect(hwnd, null(), 0) };
                0
            }
            WM_LBUTTONDOWN => {
                let x = (lparam as u32 & 0xffff) as i16 as i32;
                let y = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
                let action = manager_action(x, y);
                MANAGER_PRESSED.store(action, Ordering::Relaxed);
                if action == 6 {
                    let duration = f64::from_bits(PLAYBACK_DURATION.load(Ordering::Relaxed));
                    let time = duration * ((x - 24) as f64 / 816.0).clamp(0.0, 1.0);
                    SCRUB_PREVIEW.store(time.to_bits(), Ordering::Relaxed);
                }
                if action != 0 {
                    unsafe {
                        SetCapture(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                let x = (lparam as u32 & 0xffff) as i16 as i32;
                let y = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
                let released = manager_action(x, y);
                let pressed = MANAGER_PRESSED.swap(0, Ordering::Relaxed);
                let action = if pressed == 6 {
                    6
                } else if released == pressed {
                    released as i32
                } else {
                    0
                };
                unsafe {
                    ReleaseCapture();
                    InvalidateRect(hwnd, null(), 0);
                }
                if let Some(ctx) = unsafe { context(hwnd) } {
                    match action {
                        MANAGER_SELECT => {
                            if let Some(selected) = unsafe { select_path(hwnd) } {
                                apply_selected_path(hwnd, selected);
                            }
                        }
                        6 => {
                            let duration =
                                f64::from_bits(PLAYBACK_DURATION.load(Ordering::Relaxed));
                            let time = duration * ((x - 24) as f64 / 816.0).clamp(0.0, 1.0);
                            PLAYBACK_TIME.store(time.to_bits(), Ordering::Relaxed);
                            SCRUB_PREVIEW.store(u64::MAX, Ordering::Relaxed);
                            http_get(&ctx.base, &format!("/seek?time={time}"));
                            set_manager_status("Playback position updated.");
                        }
                        MANAGER_COPY => {
                            if let Some(url) = manager_url(ctx) {
                                unsafe { copy_text(hwnd, &url) };
                                set_manager_status("Trigger URL copied to the clipboard.");
                            }
                        }
                        MANAGER_PLAY => {
                            if f64::from_bits(PLAYBACK_DURATION.load(Ordering::Relaxed)) > 0.0 {
                                let pause = PLAYBACK_PLAYING.load(Ordering::Relaxed);
                                http_get(&ctx.base, &format!("/pause?paused={pause}"));
                                set_manager_status(if pause {
                                    "Playback paused."
                                } else {
                                    "Playback resumed."
                                });
                            } else if let Some(url) = manager_url(ctx)
                                && let Some(path) = url.strip_prefix(&ctx.base)
                            {
                                http_get(&ctx.base, path);
                                set_manager_status("Play request sent.");
                            }
                        }
                        MANAGER_OVERLAY => {
                            unsafe { open_url(&format!("{}/overlay", ctx.base)) };
                            set_manager_status("Overlay opened in the default browser.");
                        }
                        MANAGER_STOP => {
                            http_get(&ctx.base, "/stop");
                            set_manager_status("Playback stopped.");
                        }
                        MANAGER_NEXT => {
                            http_get(&ctx.base, "/next");
                            set_manager_status("Skipped to the next folder video.");
                        }
                        MANAGER_EXIT => ctx.cancel.cancel(),
                        _ => {}
                    }
                }
                0
            }
            WM_TIMER if wparam == TIMER_MANAGER => {
                if PLAYBACK_PLAYING.load(Ordering::Relaxed)
                    && MANAGER_PRESSED.load(Ordering::Relaxed) != 6
                {
                    let duration = f64::from_bits(PLAYBACK_DURATION.load(Ordering::Relaxed));
                    let current = f64::from_bits(PLAYBACK_TIME.load(Ordering::Relaxed));
                    PLAYBACK_TIME
                        .store((current + 0.033).min(duration).to_bits(), Ordering::Relaxed);
                    unsafe {
                        InvalidateRect(
                            hwnd,
                            &RECT {
                                left: 16,
                                top: 232,
                                right: 848,
                                bottom: 289,
                            },
                            0,
                        );
                    }
                }
                0
            }
            WM_CLOSE => {
                unsafe {
                    KillTimer(hwnd, TIMER_MANAGER);
                    ShowWindow(hwnd, SW_HIDE);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    unsafe extern "system" fn tray_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_CREATE => {
                let create = lparam as *const CREATESTRUCTW;
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
                }
                0
            }
            TRAY_MESSAGE => {
                let notification = lparam as u32 & 0xffff;
                if matches!(notification, WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT)
                    && let Some(ctx) = unsafe { context(hwnd) }
                {
                    unsafe {
                        show_manager(ctx.manager);
                    }
                }
                if notification == WM_RBUTTONUP {
                    let menu = unsafe { CreatePopupMenu() };
                    unsafe {
                        AppendMenuW(
                            menu,
                            MF_STRING | MF_DISABLED,
                            0,
                            wide("OBS Video Overlay").as_ptr(),
                        );
                        AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        AppendMenuW(menu, MF_STRING, CMD_MANAGER, wide("Open manager").as_ptr());
                        AppendMenuW(
                            menu,
                            MF_STRING,
                            CMD_OPEN,
                            wide("Open overlay page").as_ptr(),
                        );
                        AppendMenuW(menu, MF_STRING, CMD_COPY, wide("Copy overlay URL").as_ptr());
                        AppendMenuW(
                            menu,
                            MF_STRING,
                            CMD_LOG,
                            wide("Open diagnostics log").as_ptr(),
                        );
                        AppendMenuW(
                            menu,
                            MF_STRING,
                            CMD_HIDE,
                            wide("Hide the playing clip").as_ptr(),
                        );
                        AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        AppendMenuW(menu, MF_STRING, CMD_STOP, wide("Exit").as_ptr());
                        let mut point: POINT = zeroed();
                        GetCursorPos(&mut point);
                        SetForegroundWindow(hwnd);
                        TrackPopupMenu(
                            menu,
                            TPM_LEFTALIGN | TPM_BOTTOMALIGN,
                            point.x,
                            point.y,
                            0,
                            hwnd,
                            null(),
                        );
                        DestroyMenu(menu);
                    }
                }
                0
            }
            WM_COMMAND => {
                if let Some(ctx) = unsafe { context(hwnd) } {
                    match wparam & 0xffff {
                        CMD_MANAGER => unsafe { show_manager(ctx.manager) },
                        CMD_OPEN => unsafe { open_url(&format!("{}/overlay", ctx.base)) },
                        CMD_COPY => unsafe { copy_text(hwnd, &format!("{}/overlay", ctx.base)) },
                        CMD_LOG => unsafe {
                            open_url(&crate::diagnostics::path().to_string_lossy())
                        },
                        CMD_HIDE => {
                            http_get(&ctx.base, "/stop");
                        }
                        CMD_STOP => ctx.cancel.cancel(),
                        _ => {}
                    }
                }
                0
            }
            NOTIFICATION_MESSAGE => {
                let notification = unsafe { Box::from_raw(lparam as *mut (String, bool)) };
                if let Some(ctx) = unsafe { context(hwnd) } {
                    unsafe {
                        show_popup(ctx.popup, &notification.0, notification.1);
                    }
                }
                0
            }
            PLAYBACK_MESSAGE => {
                let playback = unsafe { Box::from_raw(lparam as *mut super::PlaybackUpdate) };
                let old_duration = PLAYBACK_DURATION.load(Ordering::Relaxed);
                let old_playing = PLAYBACK_PLAYING.load(Ordering::Relaxed);
                let old_folder = FOLDER_PLAYING.load(Ordering::Relaxed);
                let old_file = PLAYBACK_FILE
                    .get_or_init(|| Mutex::new(String::new()))
                    .lock()
                    .unwrap()
                    .clone();
                PLAYBACK_TIME.store(playback.0.to_bits(), Ordering::Relaxed);
                PLAYBACK_DURATION.store(playback.1.to_bits(), Ordering::Relaxed);
                PLAYBACK_PLAYING.store(playback.2, Ordering::Relaxed);
                FOLDER_PLAYING.store(playback.3, Ordering::Relaxed);
                FOLDER_INDEX.store(playback.4, Ordering::Relaxed);
                FOLDER_TOTAL.store(playback.5, Ordering::Relaxed);
                *PLAYBACK_FILE
                    .get_or_init(|| Mutex::new(String::new()))
                    .lock()
                    .unwrap() = playback.6.clone();
                if !playback.3 && !playback.6.is_empty() {
                    *MANAGER_PATH
                        .get_or_init(|| Mutex::new(None))
                        .lock()
                        .unwrap() = Some(playback.6.clone());
                }
                if let Some(ctx) = unsafe { context(hwnd) } {
                    unsafe {
                        if old_duration != playback.1.to_bits()
                            || old_playing != playback.2
                            || old_folder != playback.3
                            || old_file != playback.6
                        {
                            InvalidateRect(ctx.manager, null(), 0);
                        } else {
                            InvalidateRect(
                                ctx.manager,
                                &RECT {
                                    left: 16,
                                    top: 232,
                                    right: 848,
                                    bottom: 289,
                                },
                                0,
                            );
                        }
                    }
                }
                0
            }
            WM_CLOSE => {
                unsafe {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                let mut icon: NOTIFYICONDATAW = unsafe { zeroed() };
                icon.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                icon.hWnd = hwnd;
                icon.uID = 1;
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &icon);
                    if let Some(ctx) = context(hwnd) {
                        if !ctx.manager.is_null() {
                            DestroyWindow(ctx.manager);
                            ctx.manager = null_mut();
                        }
                        if !ctx.popup.is_null() {
                            DestroyWindow(ctx.popup);
                            ctx.popup = null_mut();
                        }
                        if !ctx.icon.is_null() {
                            DestroyIcon(ctx.icon);
                            ctx.icon = null_mut();
                        }
                    }
                    PostQuitMessage(0);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }
    pub fn start(
        base: String,
        cancel: CancellationToken,
        notifications: mpsc::Receiver<(String, bool)>,
        playback: mpsc::Receiver<super::PlaybackUpdate>,
    ) -> Option<Tray> {
        let shared = Arc::new(Mutex::new(0isize));
        let output = shared.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        thread::Builder::new()
            .name("tray".into())
            .spawn(move || unsafe {
                let instance = GetModuleHandleW(null());
                let tray_class = wide("ObsVideoTriggerTray");
                let popup_class = wide("ObsVideoTriggerPopup");
                let manager_class = wide("ObsVideoTriggerManager");
                RegisterClassW(&WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(tray_proc),
                    hInstance: instance,
                    lpszClassName: tray_class.as_ptr(),
                    ..zeroed()
                });
                RegisterClassW(&WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(manager_proc),
                    hInstance: instance,
                    hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                    lpszClassName: manager_class.as_ptr(),
                    ..zeroed()
                });
                RegisterClassW(&WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(popup_proc),
                    hInstance: instance,
                    hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                    lpszClassName: popup_class.as_ptr(),
                    ..zeroed()
                });
                let popup = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    popup_class.as_ptr(),
                    wide("").as_ptr(),
                    WS_POPUP,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    380,
                    104,
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                let mut context = Box::new(Context {
                    base,
                    cancel,
                    popup,
                    manager: null_mut(),
                    icon: null_mut(),
                });
                let hwnd = CreateWindowExW(
                    0,
                    tray_class.as_ptr(),
                    wide("").as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    instance,
                    context.as_mut() as *mut Context as *const c_void,
                );
                if hwnd.is_null() {
                    let _ = ready_tx.send(Err("could not create the tray window".into()));
                    return;
                }
                context.manager = CreateWindowExW(
                    0,
                    manager_class.as_ptr(),
                    wide("OBS Video Overlay Manager").as_ptr(),
                    WS_CAPTION | WS_SYSMENU,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    880,
                    470,
                    null_mut(),
                    null_mut(),
                    instance,
                    context.as_mut() as *mut Context as *const c_void,
                );
                if context.manager.is_null() {
                    let _ = ready_tx.send(Err("could not create the manager window".into()));
                    DestroyWindow(hwnd);
                    return;
                }
                *output.lock().unwrap() = hwnd as isize;
                let mut icon: NOTIFYICONDATAW = zeroed();
                icon.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                icon.hWnd = hwnd;
                icon.uID = 1;
                icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
                icon.uCallbackMessage = TRAY_MESSAGE;
                icon.hIcon = create_app_icon(instance);
                if icon.hIcon.is_null() {
                    let _ = ready_tx.send(Err("could not load the tray icon".into()));
                    DestroyWindow(hwnd);
                    return;
                }
                context.icon = icon.hIcon;
                SendMessageW(
                    context.manager,
                    WM_SETICON,
                    ICON_BIG as usize,
                    icon.hIcon as LPARAM,
                );
                SendMessageW(
                    context.manager,
                    WM_SETICON,
                    ICON_SMALL as usize,
                    icon.hIcon as LPARAM,
                );
                fill(&mut icon.szTip, "OBS Video Overlay");
                if Shell_NotifyIconW(NIM_ADD, &icon) == 0 {
                    let _ =
                        ready_tx.send(Err("Windows rejected the tray icon registration".into()));
                    DestroyWindow(hwnd);
                    return;
                }
                icon.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &icon);
                let _ = ready_tx.send(Ok(()));
                let notification_hwnd = hwnd as isize;
                thread::Builder::new()
                    .name("tray-notifications".into())
                    .spawn(move || {
                        for notification in notifications {
                            let notification = Box::into_raw(Box::new(notification));
                            if PostMessageW(
                                notification_hwnd as HWND,
                                NOTIFICATION_MESSAGE,
                                0,
                                notification as LPARAM,
                            ) == 0
                            {
                                drop(Box::from_raw(notification));
                                break;
                            }
                        }
                    })
                    .ok();
                let playback_hwnd = hwnd as isize;
                thread::Builder::new()
                    .name("tray-playback".into())
                    .spawn(move || {
                        for state in playback {
                            let state = Box::into_raw(Box::new(state));
                            if PostMessageW(
                                playback_hwnd as HWND,
                                PLAYBACK_MESSAGE,
                                0,
                                state as LPARAM,
                            ) == 0
                            {
                                drop(Box::from_raw(state));
                                break;
                            }
                        }
                    })
                    .ok();
                let mut message: MSG = zeroed();
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                drop(context);
            })
            .ok()?;
        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                crate::console::err(&format!("Tray unavailable: {message}."));
                return None;
            }
            Err(_) => {
                crate::console::err("Tray unavailable: startup timed out.");
                return None;
            }
        }
        Some(Tray { hwnd: shared })
    }
}

#[cfg(windows)]
pub use native::start;
#[cfg(not(windows))]
pub struct Tray;
#[cfg(not(windows))]
pub fn start(
    _: String,
    _: tokio_util::sync::CancellationToken,
    _: std::sync::mpsc::Receiver<(String, bool)>,
    _: std::sync::mpsc::Receiver<PlaybackUpdate>,
) -> Option<Tray> {
    None
}
