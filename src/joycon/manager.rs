use crate::joycon::{JoyConDevice, JoyConResult};
use crossbeam_channel::{unbounded, Receiver, Sender};
use hidapi::{HidApi, DeviceInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use lazy_static::lazy_static;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct JoyConSerialNumber(pub String);

pub struct JoyConManager {
    devices: HashMap<JoyConSerialNumber, Arc<Mutex<JoyConDevice>>>,
    hid_api: Option<HidApi>,
    scanner: Option<JoinHandle<()>>,
    scan_interval: Duration,
    new_devices: Receiver<Arc<Mutex<JoyConDevice>>>,
}

impl JoyConManager {
    pub fn get_instance() -> Arc<Mutex<JoyConManager>> {
        lazy_static! {
            static ref MANAGER: Arc<Mutex<JoyConManager>> = {
                let (tx, rx) = unbounded();
                let mut manager = JoyConManager {
                    devices: HashMap::new(),
                    hid_api: None,
                    scanner: None,
                    scan_interval: Duration::from_secs(1),
                    new_devices: rx,
                };

                let _ = manager.scan();

                let manager_arc = Arc::new(Mutex::new(manager));
                let manager_weak = Arc::downgrade(&manager_arc);

                std::thread::spawn(move || {
                    while let Some(m) = manager_weak.upgrade() {
                        let interval = {
                            let mut lock = match m.lock() {
                                Ok(l) => l,
                                Err(e) => e.into_inner(),
                            };
                            if let Ok(new_devices) = lock.scan() {
                                for dev in new_devices {
                                    let _ = tx.send(dev);
                                }
                            }
                            lock.scan_interval
                        };
                        std::thread::sleep(interval);
                    }
                });

                manager_arc
            };
        }
        MANAGER.clone()
    }

    pub fn new_devices(&self) -> Receiver<Arc<Mutex<JoyConDevice>>> {
        self.new_devices.clone()
    }

    pub fn scan(&mut self) -> JoyConResult<Vec<Arc<Mutex<JoyConDevice>>>> {
        let hid_api = if let Some(hidapi) = &mut self.hid_api {
            hidapi.refresh_devices()?;
            hidapi
        } else {
            self.hid_api = Some(HidApi::new()?);
            match &mut self.hid_api {
                Some(hid_api) => hid_api,
                None => unreachable!(),
            }
        };

        let mut new_devices = Vec::new();

        for device_info in hid_api.device_list() {
            if device_info.vendor_id() == 0x057E {
                // Read incoming Product ID
                let raw_pid = device_info.product_id();
                
                // Intercept and rewrite Switch 2 controllers right here
                let product_id = if raw_pid == 0x2066 {
                    0x2006 // Switch 2 Left -> Original Left
                } else if raw_pid == 0x2067 {
                    0x2007 // Switch 2 Right -> Original Right
                } else {
                    raw_pid // Leave original Switch hardware alone
                };

                // Filter for valid Joy-Cons
                if product_id == 0x2006 || product_id == 0x2007 {
                    let serial_number = match device_info.serial_number() {
                        Some(s) => JoyConSerialNumber(s.to_string()),
                        None => continue,
                    };

                    if !self.devices.contains_key(&serial_number) {
                        // Crucial patch: wrap product_id into the correct structural type expected by joycon-rs
                        let device = Arc::new(Mutex::new(JoyConDevice::new(device_info, product_id.into())?));
                        self.devices.insert(serial_number, Arc::clone(&device));
                        new_devices.push(device);
                    }
                }
            }
        }

        Ok(new_devices)
    }
}

lazy_static! {
    pub static ref JOYCON_RECEIVER: Receiver<Arc<Mutex<JoyConDevice>>> = {
        let manager = JoyConManager::get_instance();
        let lock = match manager.lock() {
            Ok(l) => l,
            Err(e) => e.into_inner(),
        };
        lock.new_devices()
    };
}
