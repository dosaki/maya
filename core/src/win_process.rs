//! Process facts on Windows, from documented APIs only: the running
//! processes (a ToolHelp snapshot) and which processes have a file open
//! (the Restart Manager, which installers use to name the apps to close).

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};

/// `(pid, executable name)` of every running process.
pub fn list() -> Vec<(u32, String)> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    // SAFETY: a snapshot walked with a correctly sized entry, then closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push((e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..len])));
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// `(pid, parent pid)` of every running process.
pub fn tree() -> Vec<(u32, u32)> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    // SAFETY: as in `list`: a snapshot walked with a correctly sized entry, then closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            out.push((e.th32ProcessID, e.th32ParentProcessID));
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Pids of the processes that have `path` open, per the Restart Manager.
pub fn holders(path: &std::path::Path) -> Vec<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::RestartManager::{RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY, RM_PROCESS_INFO};
    const ERROR_MORE_DATA: u32 = 234;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut session = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    // SAFETY: Restart Manager calls with buffers of the sizes given; the session is ended.
    unsafe {
        if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
            return vec![];
        }
        let files = [wide.as_ptr()];
        let mut out = Vec::new();
        if RmRegisterResources(session, 1, files.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null()) == 0 {
            let mut infos: Vec<RM_PROCESS_INFO> = vec![std::mem::zeroed(); 8];
            for _ in 0..3 {
                let (mut needed, mut count, mut reasons) = (0u32, infos.len() as u32, 0u32);
                match RmGetList(session, &mut needed, &mut count, infos.as_mut_ptr(), &mut reasons) {
                    0 => {
                        out = infos[..count as usize].iter().map(|i| i.Process.dwProcessId).collect();
                        break;
                    }
                    ERROR_MORE_DATA => infos = vec![std::mem::zeroed(); needed as usize + 4],
                    _ => break,
                }
            }
        }
        RmEndSession(session);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_listed() {
        let me = std::process::id();
        assert!(list().iter().any(|(pid, name)| *pid == me && name.to_lowercase().ends_with(".exe")));
    }

    #[test]
    fn a_file_held_open_names_its_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("held.lock");
        let file = std::fs::File::create(&path).unwrap();
        assert!(holders(&path).contains(&std::process::id()));
        drop(file);
        assert!(!holders(&path).contains(&std::process::id()));
        assert!(holders(&dir.path().join("missing.lock")).is_empty());
    }
}
