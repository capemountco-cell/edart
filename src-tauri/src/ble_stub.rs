//! Bluetooth on platforms other than macOS: not supported. Keeps the command
//! surface identical so the UI needs no special cases.

use edart_core::status::BluetoothStatus;

pub fn status() -> BluetoothStatus {
    BluetoothStatus {
        supported: false,
        state: "unsupported".into(),
        ..Default::default()
    }
}

pub fn send(_report_id: u8, _payload: &[u8]) -> bool {
    false
}

pub fn restart() {}

pub fn reach_out() {}

pub fn pause() {}

pub fn resume() {}
