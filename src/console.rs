#[cfg(windows)]
fn write(which: u32, message: &str) {
    use std::sync::Once;
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::WriteFile,
        System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle},
    };
    static ATTACH: Once = Once::new();
    let mut handle: HANDLE = unsafe { GetStdHandle(which) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        ATTACH.call_once(|| unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        });
        handle = unsafe { GetStdHandle(which) };
    }
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return;
    }
    let bytes = format!("{message}\r\n");
    let mut written = 0;
    unsafe {
        WriteFile(
            handle,
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(windows)]
pub fn out(message: &str) {
    write(
        windows_sys::Win32::System::Console::STD_OUTPUT_HANDLE,
        message,
    );
}
#[cfg(windows)]
pub fn err(message: &str) {
    write(
        windows_sys::Win32::System::Console::STD_ERROR_HANDLE,
        message,
    );
}
#[cfg(not(windows))]
pub fn out(message: &str) {
    println!("{message}");
}
#[cfg(not(windows))]
pub fn err(message: &str) {
    eprintln!("{message}");
}
