//! Converting between the core crate's [`XpcValue`] and live libxpc
//! objects, so the messages themselves are built and read in tested code.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::rc::Rc;

use block2::RcBlock;
use edart_core::usb::xpc::XpcValue;
use objc2::runtime::Bool;

use super::sys::*;

/// An owned libxpc object, released when dropped.
pub struct Xpc(pub XpcObject);

// SAFETY: libxpc objects are reference-counted and safe to send and to
// release from any thread.
unsafe impl Send for Xpc {}

impl Drop for Xpc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this wrapper owns one reference.
            unsafe { xpc_release(self.0) }
        }
    }
}

fn c_string(text: &str) -> CString {
    // A NUL inside a key or string cannot be sent; cut there.
    let end = text.find('\0').unwrap_or(text.len());
    CString::new(&text[..end]).unwrap_or_default()
}

/// Build a libxpc object from a value.
pub fn to_xpc(value: &XpcValue) -> Xpc {
    // SAFETY: every call gets valid arguments; containers retain what is
    // put in them, so each child's own reference is dropped after.
    unsafe {
        Xpc(match value {
            XpcValue::Null => xpc_null_create(),
            XpcValue::Bool(v) => xpc_bool_create(*v),
            XpcValue::Int64(v) => xpc_int64_create(*v),
            XpcValue::Uint64(v) => xpc_uint64_create(*v),
            XpcValue::Double(v) => xpc_double_create(*v),
            XpcValue::Data(bytes) => xpc_data_create(bytes.as_ptr().cast(), bytes.len()),
            XpcValue::String(s) => xpc_string_create(c_string(s).as_ptr()),
            XpcValue::Uuid(bytes) => xpc_uuid_create(bytes.as_ptr()),
            XpcValue::Array(items) => {
                let array = xpc_array_create(std::ptr::null(), 0);
                for item in items {
                    let child = to_xpc(item);
                    xpc_array_append_value(array, child.0);
                }
                array
            }
            XpcValue::Dictionary(entries) => {
                let dictionary = xpc_dictionary_create(std::ptr::null(), std::ptr::null(), 0);
                for (key, item) in entries {
                    let child = to_xpc(item);
                    xpc_dictionary_set_value(dictionary, c_string(key).as_ptr(), child.0);
                }
                dictionary
            }
        })
    }
}

fn is_type(object: XpcObject, kind: *const c_void) -> bool {
    // SAFETY: `object` is a live libxpc object.
    !object.is_null() && unsafe { xpc_get_type(object) } == kind
}

macro_rules! xpc_type {
    ($name:ident) => {
        // The address of a libxpc type object, which is all a type is.
        std::ptr::addr_of!($name)
    };
}

pub fn is_dictionary(object: XpcObject) -> bool {
    is_type(object, xpc_type!(_xpc_type_dictionary))
}

pub fn is_connection(object: XpcObject) -> bool {
    is_type(object, xpc_type!(_xpc_type_connection))
}

pub fn is_fd(object: XpcObject) -> bool {
    is_type(object, xpc_type!(_xpc_type_fd))
}

/// How deep a message may nest before the rest is dropped.
const MAX_DEPTH: usize = 48;

/// Copy a libxpc object into a value. Types a value cannot hold (file
/// descriptors, endpoints, errors) become `Null`.
pub fn from_xpc(object: XpcObject) -> XpcValue {
    from_xpc_at(object, 0)
}

fn from_xpc_at(object: XpcObject, depth: usize) -> XpcValue {
    if object.is_null() || depth > MAX_DEPTH {
        return XpcValue::Null;
    }
    // SAFETY: `object` is live for this call, and each getter is only used
    // on an object of its own type.
    unsafe {
        let kind = xpc_get_type(object);
        if kind == xpc_type!(_xpc_type_dictionary) {
            let entries = Rc::new(RefCell::new(BTreeMap::new()));
            let sink = Rc::clone(&entries);
            let applier = RcBlock::new(move |key: *const c_char, value: XpcObject| {
                if !key.is_null() {
                    let key = CStr::from_ptr(key).to_string_lossy().into_owned();
                    sink.borrow_mut().insert(key, from_xpc_at(value, depth + 1));
                }
                Bool::YES
            });
            xpc_dictionary_apply(object, &applier);
            drop(applier);
            let entries = std::mem::take(&mut *entries.borrow_mut());
            XpcValue::Dictionary(entries)
        } else if kind == xpc_type!(_xpc_type_array) {
            let count = xpc_array_get_count(object);
            XpcValue::Array(
                (0..count)
                    .map(|i| from_xpc_at(xpc_array_get_value(object, i), depth + 1))
                    .collect(),
            )
        } else if kind == xpc_type!(_xpc_type_string) {
            let text = xpc_string_get_string_ptr(object);
            if text.is_null() {
                XpcValue::Null
            } else {
                XpcValue::String(CStr::from_ptr(text).to_string_lossy().into_owned())
            }
        } else if kind == xpc_type!(_xpc_type_int64) {
            XpcValue::Int64(xpc_int64_get_value(object))
        } else if kind == xpc_type!(_xpc_type_uint64) {
            XpcValue::Uint64(xpc_uint64_get_value(object))
        } else if kind == xpc_type!(_xpc_type_bool) {
            XpcValue::Bool(xpc_bool_get_value(object))
        } else if kind == xpc_type!(_xpc_type_double) {
            XpcValue::Double(xpc_double_get_value(object))
        } else if kind == xpc_type!(_xpc_type_uuid) {
            let mut bytes = [0u8; 16];
            let source = xpc_uuid_get_bytes(object);
            if !source.is_null() {
                std::ptr::copy_nonoverlapping(source, bytes.as_mut_ptr(), 16);
            }
            XpcValue::Uuid(bytes)
        } else if kind == xpc_type!(_xpc_type_data) {
            let length = xpc_data_get_length(object);
            let source = xpc_data_get_bytes_ptr(object);
            if source.is_null() || length == 0 {
                XpcValue::Data(Vec::new())
            } else {
                XpcValue::Data(std::slice::from_raw_parts(source.cast::<u8>(), length).to_vec())
            }
        } else {
            XpcValue::Null
        }
    }
}

/// libxpc's own one-line description of an object, for error messages.
pub fn describe(object: XpcObject) -> String {
    if object.is_null() {
        return "nothing".into();
    }
    // SAFETY: the description is a malloc'd C string this code must free.
    unsafe {
        let text = xpc_copy_description(object);
        if text.is_null() {
            return "an XPC object".into();
        }
        let out = CStr::from_ptr(text).to_string_lossy().into_owned();
        free(text.cast());
        out
    }
}
