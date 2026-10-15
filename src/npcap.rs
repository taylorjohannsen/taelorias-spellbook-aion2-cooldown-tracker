//! Npcap, loaded at runtime so building doesn't need its SDK. Npcap keeps
//! wpcap.dll in System32\Npcap; WinPcap-compatible mode also puts one on the path.
//! Captures are read-only copies, so this runs fine next to other meters.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;
use std::sync::Arc;

#[repr(C)]
struct PcapIf {
    next: *mut PcapIf,
    name: *mut c_char,
    description: *mut c_char,
    addresses: *mut c_void,
    flags: u32,
}

// Windows' timeval has 32-bit fields
#[repr(C)]
struct PacketHeader {
    tv_sec: i32,
    tv_usec: i32,
    caplen: u32,
    len: u32,
}

#[repr(C)]
struct BpfProgram {
    bf_len: u32,
    bf_insns: *mut c_void,
}

type Pcap = *mut c_void;
type SetInt = unsafe extern "C" fn(Pcap, c_int) -> c_int;

const ERRBUF: usize = 256;
const NETMASK_UNKNOWN: u32 = 0xFFFF_FFFF;
const SNAPLEN: c_int = 65_535;
const READ_TIMEOUT_MS: c_int = 100;
const KERNEL_BUFFER: c_int = 16 * 1024 * 1024;

struct Api {
    _lib: libloading::Library,
    findalldevs: unsafe extern "C" fn(*mut *mut PcapIf, *mut c_char) -> c_int,
    freealldevs: unsafe extern "C" fn(*mut PcapIf),
    create: unsafe extern "C" fn(*const c_char, *mut c_char) -> Pcap,
    open_offline: unsafe extern "C" fn(*const c_char, *mut c_char) -> Pcap,
    set_snaplen: SetInt,
    set_promisc: SetInt,
    set_timeout: SetInt,
    set_immediate_mode: SetInt,
    set_buffer_size: SetInt,
    activate: unsafe extern "C" fn(Pcap) -> c_int,
    compile: unsafe extern "C" fn(Pcap, *mut BpfProgram, *const c_char, c_int, u32) -> c_int,
    setfilter: unsafe extern "C" fn(Pcap, *mut BpfProgram) -> c_int,
    freecode: unsafe extern "C" fn(*mut BpfProgram),
    next_ex: unsafe extern "C" fn(Pcap, *mut *mut PacketHeader, *mut *const u8) -> c_int,
    datalink: unsafe extern "C" fn(Pcap) -> c_int,
    geterr: unsafe extern "C" fn(Pcap) -> *mut c_char,
    close: unsafe extern "C" fn(Pcap),
}

pub struct Npcap(Arc<Api>);

pub struct Adapter {
    pub name: String,
    pub description: String,
}

pub struct Capture {
    api: Arc<Api>,
    p: Pcap,
    pub link: i32,
}

// a capture handle is only ever used from the thread that reads it
unsafe impl Send for Capture {}

pub enum Next<'a> {
    Packet(&'a [u8]),
    Timeout,
    End,
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        *$lib.get(concat!($name, "\0").as_bytes()).map_err(|_| format!("Npcap is missing {}", $name))?
    };
}

impl Npcap {
    pub fn load() -> Result<Self, String> {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let path = format!(r"{root}\System32\Npcap\wpcap.dll");
        let lib = unsafe {
            libloading::os::windows::Library::load_with_flags(&path, libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH)
                .map(libloading::Library::from)
                .or_else(|_| libloading::Library::new("wpcap.dll"))
        }
        .map_err(|_| "Npcap isn't installed".to_string())?;
        let api = unsafe {
            Api {
                findalldevs: sym!(lib, "pcap_findalldevs"),
                freealldevs: sym!(lib, "pcap_freealldevs"),
                create: sym!(lib, "pcap_create"),
                open_offline: sym!(lib, "pcap_open_offline"),
                set_snaplen: sym!(lib, "pcap_set_snaplen"),
                set_promisc: sym!(lib, "pcap_set_promisc"),
                set_timeout: sym!(lib, "pcap_set_timeout"),
                set_immediate_mode: sym!(lib, "pcap_set_immediate_mode"),
                set_buffer_size: sym!(lib, "pcap_set_buffer_size"),
                activate: sym!(lib, "pcap_activate"),
                compile: sym!(lib, "pcap_compile"),
                setfilter: sym!(lib, "pcap_setfilter"),
                freecode: sym!(lib, "pcap_freecode"),
                next_ex: sym!(lib, "pcap_next_ex"),
                datalink: sym!(lib, "pcap_datalink"),
                geterr: sym!(lib, "pcap_geterr"),
                close: sym!(lib, "pcap_close"),
                _lib: lib,
            }
        };
        Ok(Self(Arc::new(api)))
    }

    pub fn adapters(&self) -> Result<Vec<Adapter>, String> {
        let mut errbuf = [0 as c_char; ERRBUF];
        let mut list: *mut PcapIf = ptr::null_mut();
        unsafe {
            if (self.0.findalldevs)(&mut list, errbuf.as_mut_ptr()) != 0 {
                return Err(buf_text(&errbuf));
            }
            let mut adapters = Vec::new();
            let mut it = list;
            while !it.is_null() {
                adapters.push(Adapter { name: text((*it).name), description: text((*it).description) });
                it = (*it).next;
            }
            (self.0.freealldevs)(list);
            Ok(adapters)
        }
    }

    pub fn open(&self, adapter: &str, filter: &str) -> Result<Capture, String> {
        let name = CString::new(adapter).map_err(|e| e.to_string())?;
        let mut errbuf = [0 as c_char; ERRBUF];
        let api = &self.0;
        let p = unsafe { (api.create)(name.as_ptr(), errbuf.as_mut_ptr()) };
        if p.is_null() {
            return Err(buf_text(&errbuf));
        }
        let capture = Capture { api: self.0.clone(), p, link: 0 };
        unsafe {
            (api.set_snaplen)(p, SNAPLEN);
            (api.set_promisc)(p, 0);
            (api.set_timeout)(p, READ_TIMEOUT_MS);
            (api.set_immediate_mode)(p, 1);
            (api.set_buffer_size)(p, KERNEL_BUFFER);
            if (api.activate)(p) < 0 {
                return Err(capture.error());
            }
        }
        capture.with_filter(filter)
    }

    pub fn open_file(&self, path: &str, filter: &str) -> Result<Capture, String> {
        let path = CString::new(path).map_err(|e| e.to_string())?;
        let mut errbuf = [0 as c_char; ERRBUF];
        let p = unsafe { (self.0.open_offline)(path.as_ptr(), errbuf.as_mut_ptr()) };
        if p.is_null() {
            return Err(buf_text(&errbuf));
        }
        Capture { api: self.0.clone(), p, link: 0 }.with_filter(filter)
    }
}

impl Capture {
    fn with_filter(mut self, filter: &str) -> Result<Self, String> {
        let filter = CString::new(filter).map_err(|e| e.to_string())?;
        let mut program = BpfProgram { bf_len: 0, bf_insns: ptr::null_mut() };
        unsafe {
            if (self.api.compile)(self.p, &mut program, filter.as_ptr(), 1, NETMASK_UNKNOWN) != 0 {
                return Err(self.error());
            }
            let set = (self.api.setfilter)(self.p, &mut program);
            (self.api.freecode)(&mut program);
            if set != 0 {
                return Err(self.error());
            }
            self.link = (self.api.datalink)(self.p);
        }
        Ok(self)
    }

    pub fn next(&mut self) -> Result<Next<'_>, String> {
        let mut header: *mut PacketHeader = ptr::null_mut();
        let mut data: *const u8 = ptr::null();
        match unsafe { (self.api.next_ex)(self.p, &mut header, &mut data) } {
            1 => Ok(Next::Packet(unsafe { std::slice::from_raw_parts(data, (*header).caplen as usize) })),
            0 => Ok(Next::Timeout),
            -2 => Ok(Next::End),
            _ => Err(self.error()),
        }
    }

    fn error(&self) -> String {
        unsafe { text((self.api.geterr)(self.p)) }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { (self.api.close)(self.p) }
    }
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

fn buf_text(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
