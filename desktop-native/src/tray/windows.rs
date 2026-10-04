//! Bandeja do Windows: uma janela oculta numa thread própria, com o laço de mensagens dela. Não é janela "só de
//! mensagens": essas não recebem o `TaskbarCreated`, que pede o ícone de volta quando o Explorer reinicia.
use super::TrayEvent;
use std::sync::{OnceLock, atomic::{AtomicU32, Ordering}};
use windows::{core::{w, HSTRING, PCWSTR}, Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW},
        WindowsAndMessaging::{AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
            GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
            TrackPopupMenu, TranslateMessage, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
            WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW}}}};

const CALLBACK: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;
const OPEN: usize = 1;
const QUIT: usize = 2;

static EVENTS: OnceLock<async_channel::Sender<TrayEvent>> = OnceLock::new();
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn send(event: TrayEvent) { if let Some(events) = EVENTS.get() { let _ = events.try_send(event); } }

pub struct Handle { hwnd: isize }

impl Handle {
    pub fn online(&self) -> bool { true }
    pub fn refresh(&self) {}
}

impl Drop for Handle {
    fn drop(&mut self) { unsafe { let _ = PostMessageW(Some(HWND(self.hwnd as _)), WM_CLOSE, WPARAM(0), LPARAM(0)); } }
}

pub fn start(events: async_channel::Sender<TrayEvent>) -> Result<Handle, String> {
    // O canal é o mesmo durante a vida do processo; religar a opção reaproveita o primeiro.
    let _ = EVENTS.set(events);
    let (ready, wait) = std::sync::mpsc::channel::<Result<isize, String>>();
    std::thread::Builder::new().name("tray".into()).spawn(move || match create() {
        Ok(hwnd) => { let _ = ready.send(Ok(hwnd.0 as isize)); pump(); }
        Err(reason) => { let _ = ready.send(Err(reason)); }
    }).map_err(|e| e.to_string())?;
    wait.recv().map_err(|e| e.to_string())?.map(|hwnd| Handle { hwnd })
}

fn create() -> Result<HWND, String> {
    unsafe {
        let module = GetModuleHandleW(PCWSTR::null()).map_err(|e| e.to_string())?;
        let class = w!("HangarTray");
        // Registrar duas vezes (opção religada) falha e não importa: a classe já existe.
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Hangar"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(module.into()), None)
            .map_err(|e| e.to_string())?;
        if !add_icon(hwnd) {
            let _ = DestroyWindow(hwnd);
            return Err("Shell_NotifyIcon".into());
        }
        Ok(hwnd)
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: ICON_ID, ..Default::default() };
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = CALLBACK;
    // Recurso 1 do executável (`assets/brand/icon.rc`).
    unsafe {
        if let Ok(module) = GetModuleHandleW(PCWSTR::null()) {
            if let Ok(icon) = LoadIconW(Some(module.into()), PCWSTR(1 as _)) { data.hIcon = icon; }
        }
    }
    for (slot, unit) in data.szTip.iter_mut().zip("Hangar".encode_utf16()) { *slot = unit; }
    data
}

fn add_icon(hwnd: HWND) -> bool { unsafe { Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd)).as_bool() } }

fn pump() {
    let mut message = MSG::default();
    unsafe {
        // `GetMessageW` devolve -1 em erro: só o positivo segue.
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn menu(hwnd: HWND) {
    let (open, quit) = (HSTRING::from(crate::i18n::tr("tray_open")), HSTRING::from(crate::i18n::tr("tray_quit")));
    let picked = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, OPEN, PCWSTR(open.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, QUIT, PCWSTR(quit.as_ptr()));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // Sem trazer a janela dona para a frente, o menu não fecha ao clicar fora.
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY, at.x, at.y, Some(0), hwnd, None);
        let _ = DestroyMenu(menu);
        picked.0 as usize
    };
    match picked {
        OPEN => send(TrayEvent::Show),
        QUIT => send(TrayEvent::Quit),
        _ => {}
    }
}

unsafe extern "system" fn proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        CALLBACK => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => send(TrayEvent::Toggle),
                WM_RBUTTONUP => menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => { let _ = unsafe { DestroyWindow(hwnd) }; LRESULT(0) }
        WM_DESTROY => {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        // O Explorer reiniciou: a bandeja nova não conhece o ícone.
        other if other != 0 && other == TASKBAR_CREATED.load(Ordering::Relaxed) => { add_icon(hwnd); LRESULT(0) }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
