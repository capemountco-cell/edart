//! The few C functions the USB path needs from macOS, declared by hand:
//! the dynamic loader, a little CoreFoundation, and libxpc.
//!
//! Only public system APIs are declared here, so they link like any other.
//! Apple's private frameworks are never linked: they are opened at run time
//! with [`Library`], and a missing framework or symbol is an error message,
//! not a crash or a build failure.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

use block2::Block;
use objc2::runtime::Bool;

extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
    pub fn free(ptr: *mut c_void);
    /// Nanoseconds since boot, not counting sleep: the clock HID reports
    /// are stamped with.
    pub fn clock_gettime_nsec_np(clock_id: u32) -> u64;
}

pub const CLOCK_UPTIME_RAW: u32 = 8;
const RTLD_NOW: c_int = 0x2;

/// A framework opened at run time. Never closed: frameworks register
/// process-wide callbacks, and unloading under them would crash.
pub struct Library(*mut c_void);

// SAFETY: a dlopen handle is a process-wide token; dlsym is thread-safe.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Library {
    pub fn open(path: &str) -> Result<Library, String> {
        let c_path = CString::new(path).map_err(|e| e.to_string())?;
        // SAFETY: a valid NUL-terminated path; dlopen has no other
        // preconditions.
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            // SAFETY: dlerror returns a C string or null.
            let why = unsafe {
                let e = dlerror();
                if e.is_null() {
                    "unknown error".to_string()
                } else {
                    CStr::from_ptr(e).to_string_lossy().into_owned()
                }
            };
            Err(format!("could not open {path}: {why}"))
        } else {
            Ok(Library(handle))
        }
    }

    /// The address of a symbol, or `None` if this version does not have it.
    pub fn symbol(&self, name: &CStr) -> Option<*mut c_void> {
        // SAFETY: a live handle and a NUL-terminated name.
        let address = unsafe { dlsym(self.0, name.as_ptr()) };
        (!address.is_null()).then_some(address)
    }
}

// --- CoreFoundation ---

pub type CFTypeRef = *const c_void;
pub const UTF8: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFRunLoopDefaultMode: CFTypeRef;
    pub fn CFRelease(object: CFTypeRef);
    pub fn CFGetTypeID(object: CFTypeRef) -> usize;
    pub fn CFStringGetTypeID() -> usize;
    pub fn CFBooleanGetTypeID() -> usize;
    pub fn CFDictionaryGetTypeID() -> usize;
    pub fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
    pub fn CFStringCreateWithCString(
        allocator: CFTypeRef,
        text: *const c_char,
        encoding: u32,
    ) -> CFTypeRef;
    pub fn CFStringGetCString(
        string: CFTypeRef,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> u8;
    pub fn CFArrayGetCount(array: CFTypeRef) -> isize;
    pub fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    pub fn CFDataCreate(allocator: CFTypeRef, bytes: *const u8, length: isize) -> CFTypeRef;
    pub fn CFPropertyListCreateWithData(
        allocator: CFTypeRef,
        data: CFTypeRef,
        options: usize,
        format: *mut isize,
        error: *mut CFTypeRef,
    ) -> CFTypeRef;
    pub fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    pub fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, return_after_source: u8) -> i32;
}

/// An owned CoreFoundation object, released when dropped.
pub struct Cf(pub CFTypeRef);

impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this wrapper owns one reference.
            unsafe { CFRelease(self.0) }
        }
    }
}

impl Cf {
    /// Take ownership of a +1 reference; `None` for null.
    pub fn owned(object: CFTypeRef) -> Option<Cf> {
        (!object.is_null()).then_some(Cf(object))
    }

    pub fn string(text: &str) -> Option<Cf> {
        let c = CString::new(text).ok()?;
        // SAFETY: a valid C string; the default allocator.
        Cf::owned(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) })
    }
}

/// The text of a CFString; `None` if the object is something else.
pub fn cf_to_string(object: CFTypeRef) -> Option<String> {
    if object.is_null() {
        return None;
    }
    // SAFETY: `object` is a live CF object; the buffer is big enough for
    // the size passed.
    unsafe {
        if CFGetTypeID(object) != CFStringGetTypeID() {
            return None;
        }
        let mut buffer = vec![0 as c_char; 2048];
        if CFStringGetCString(object, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) == 0 {
            return None;
        }
        Some(
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// The value of a CFBoolean; `None` if the object is something else.
pub fn cf_to_bool(object: CFTypeRef) -> Option<bool> {
    if object.is_null() {
        return None;
    }
    // SAFETY: `object` is a live CF object.
    unsafe {
        if CFGetTypeID(object) != CFBooleanGetTypeID() {
            return None;
        }
        Some(CFBooleanGetValue(object) != 0)
    }
}

/// Parse a property list (XML or binary) that must be a dictionary.
pub fn plist_dictionary(bytes: &[u8]) -> Option<Cf> {
    // SAFETY: the data object copies `bytes`; null format and error
    // pointers are allowed.
    unsafe {
        let data = Cf::owned(CFDataCreate(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
        ))?;
        let plist = Cf::owned(CFPropertyListCreateWithData(
            std::ptr::null(),
            data.0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ))?;
        (CFGetTypeID(plist.0) == CFDictionaryGetTypeID()).then_some(plist)
    }
}

/// The value under a string key; borrowed from the dictionary.
pub fn dictionary_get(dictionary: &Cf, key: &str) -> CFTypeRef {
    match Cf::string(key) {
        // SAFETY: both are live CF objects of the right types.
        Some(key) => unsafe { CFDictionaryGetValue(dictionary.0, key.0) },
        None => std::ptr::null(),
    }
}

// --- libxpc and libdispatch (public API, in libSystem) ---

pub type XpcObject = *mut c_void;
pub type Handler = Block<dyn Fn(XpcObject)>;
pub type DictionaryApplier = Block<dyn Fn(*const c_char, XpcObject) -> Bool>;

extern "C" {
    pub static _xpc_type_dictionary: c_void;
    pub static _xpc_type_array: c_void;
    pub static _xpc_type_string: c_void;
    pub static _xpc_type_int64: c_void;
    pub static _xpc_type_uint64: c_void;
    pub static _xpc_type_bool: c_void;
    pub static _xpc_type_double: c_void;
    pub static _xpc_type_data: c_void;
    pub static _xpc_type_uuid: c_void;
    pub static _xpc_type_fd: c_void;
    pub static _xpc_type_connection: c_void;

    pub fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut c_void;

    pub fn xpc_retain(object: XpcObject) -> XpcObject;
    pub fn xpc_release(object: XpcObject);
    pub fn xpc_get_type(object: XpcObject) -> *const c_void;
    pub fn xpc_copy_description(object: XpcObject) -> *mut c_char;

    pub fn xpc_connection_create(name: *const c_char, queue: *mut c_void) -> XpcObject;
    pub fn xpc_connection_set_event_handler(connection: XpcObject, handler: &Handler);
    pub fn xpc_connection_activate(connection: XpcObject);
    pub fn xpc_connection_cancel(connection: XpcObject);
    pub fn xpc_connection_send_message_with_reply(
        connection: XpcObject,
        message: XpcObject,
        queue: *mut c_void,
        handler: &Handler,
    );
    pub fn xpc_endpoint_create(connection: XpcObject) -> XpcObject;

    pub fn xpc_dictionary_create(
        keys: *const *const c_char,
        values: *const XpcObject,
        count: usize,
    ) -> XpcObject;
    pub fn xpc_dictionary_set_value(dictionary: XpcObject, key: *const c_char, value: XpcObject);
    pub fn xpc_dictionary_get_value(dictionary: XpcObject, key: *const c_char) -> XpcObject;
    pub fn xpc_dictionary_apply(dictionary: XpcObject, applier: &DictionaryApplier) -> bool;

    pub fn xpc_array_create(objects: *const XpcObject, count: usize) -> XpcObject;
    pub fn xpc_array_append_value(array: XpcObject, value: XpcObject);
    pub fn xpc_array_get_count(array: XpcObject) -> usize;
    pub fn xpc_array_get_value(array: XpcObject, index: usize) -> XpcObject;

    pub fn xpc_null_create() -> XpcObject;
    pub fn xpc_bool_create(value: bool) -> XpcObject;
    pub fn xpc_bool_get_value(object: XpcObject) -> bool;
    pub fn xpc_int64_create(value: i64) -> XpcObject;
    pub fn xpc_int64_get_value(object: XpcObject) -> i64;
    pub fn xpc_uint64_create(value: u64) -> XpcObject;
    pub fn xpc_uint64_get_value(object: XpcObject) -> u64;
    pub fn xpc_double_create(value: f64) -> XpcObject;
    pub fn xpc_double_get_value(object: XpcObject) -> f64;
    pub fn xpc_string_create(text: *const c_char) -> XpcObject;
    pub fn xpc_string_get_string_ptr(object: XpcObject) -> *const c_char;
    pub fn xpc_data_create(bytes: *const c_void, length: usize) -> XpcObject;
    pub fn xpc_data_get_bytes_ptr(object: XpcObject) -> *const c_void;
    pub fn xpc_data_get_length(object: XpcObject) -> usize;
    pub fn xpc_uuid_create(uuid: *const u8) -> XpcObject;
    pub fn xpc_uuid_get_bytes(object: XpcObject) -> *const u8;
    pub fn xpc_fd_dup(object: XpcObject) -> c_int;
}
