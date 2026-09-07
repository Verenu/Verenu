//! Capture-path hardware mute controls via the Windows Device Topology API.
//!
//! `IAudioEndpointVolume::GetMute` is the endpoint (mixer) mute bit. Many USB
//! Audio Class microphones and headsets put the physical mute button on an
//! `IAudioMute` subunit along the capture path instead. Never calls SetMute.

use std::collections::HashSet;
use windows::core::Interface;
use windows::Win32::Media::Audio::{IAudioMute, IDeviceTopology, IMMDevice, IPart};
use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};

const MAX_DEPTH: u8 = 16;

pub struct TopologyMuteWatch {
    mutes: Vec<IAudioMute>,
    last: Vec<bool>,
}

impl TopologyMuteWatch {
    pub fn from_device(device: &IMMDevice) -> Self {
        let mutes = collect_capture_path_mutes(device);
        let last = mutes
            .iter()
            .map(|mute| {
                unsafe { mute.GetMute() }
                    .map(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .collect();
        log::info!(
            "mic_mute_trigger: {} hardware mute control(s) on the capture path",
            mutes.len()
        );
        Self { mutes, last }
    }

    pub fn control_count(&self) -> usize {
        self.mutes.len()
    }

    pub fn any_muted(&self) -> Option<bool> {
        if self.last.is_empty() {
            None
        } else {
            Some(self.last.iter().any(|&muted| muted))
        }
    }

    /// Returns a debounced-ready raw mute bit when any path control changes.
    pub fn poll_edge(&mut self) -> Option<bool> {
        for (i, mute) in self.mutes.iter().enumerate() {
            let Ok(flag) = (unsafe { mute.GetMute() }) else {
                continue;
            };
            let muted = flag.as_bool();
            if self.last.get(i).copied() != Some(muted) {
                if let Some(slot) = self.last.get_mut(i) {
                    *slot = muted;
                }
                return Some(muted);
            }
        }
        None
    }
}

fn collect_capture_path_mutes(device: &IMMDevice) -> Vec<IAudioMute> {
    let topology: IDeviceTopology = match unsafe { device.Activate(CLSCTX_ALL, None) } {
        Ok(topo) => topo,
        Err(err) => {
            log::debug!("mic_mute_trigger: IDeviceTopology activate failed: {err}");
            return Vec::new();
        }
    };

    let mut mutes = Vec::new();
    let mut visited = HashSet::new();
    let connector_count = unsafe { topology.GetConnectorCount() }.unwrap_or(0);
    for index in 0..connector_count {
        let Ok(connector) = (unsafe { topology.GetConnector(index) }) else {
            continue;
        };
        if let Ok(part) = connector.cast::<IPart>() {
            try_push_mute(&part, &mut mutes);
        }
        let connected = unsafe { connector.IsConnected() }
            .ok()
            .is_some_and(|flag| flag.as_bool());
        if !connected {
            continue;
        }
        let Ok(other) = (unsafe { connector.GetConnectedTo() }) else {
            continue;
        };
        if let Ok(part) = other.cast::<IPart>() {
            walk_incoming(&part, &mut mutes, &mut visited, 0);
        }
        // Prefer the mute closest to the capture endpoint. Watching every
        // subunit and OR-ing them can stick muted on unused mux paths.
        if !mutes.is_empty() {
            mutes.truncate(1);
            break;
        }
    }
    mutes
}

fn walk_incoming(
    part: &IPart,
    mutes: &mut Vec<IAudioMute>,
    visited: &mut HashSet<String>,
    depth: u8,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Ok(global_id) = unsafe { part.GetGlobalId() } {
        let id = unsafe { global_id.to_string() }.unwrap_or_default();
        unsafe {
            CoTaskMemFree(Some(global_id.0 as *const _));
        }
        if !id.is_empty() && !visited.insert(id) {
            return;
        }
    }

    try_push_mute(part, mutes);

    let Ok(list) = (unsafe { part.EnumPartsIncoming() }) else {
        return;
    };
    let count = unsafe { list.GetCount() }.unwrap_or(0);
    for index in 0..count {
        if let Ok(next) = unsafe { list.GetPart(index) } {
            walk_incoming(&next, mutes, visited, depth + 1);
        }
    }
}

fn try_push_mute(part: &IPart, mutes: &mut Vec<IAudioMute>) {
    let Some(mute) = activate_mute(part) else {
        return;
    };
    let name = match unsafe { part.GetName() } {
        Ok(pwstr) => {
            let text = unsafe { pwstr.to_string() }.unwrap_or_else(|_| "unnamed".into());
            unsafe {
                CoTaskMemFree(Some(pwstr.0 as *const _));
            }
            text
        }
        Err(_) => "unnamed".into(),
    };
    log::info!("mic_mute_trigger: capture-path hardware mute '{name}'");
    mutes.push(mute);
}

fn activate_mute(part: &IPart) -> Option<IAudioMute> {
    unsafe {
        let mut ppv: *mut std::ffi::c_void = std::ptr::null_mut();
        part.Activate(CLSCTX_ALL.0, &IAudioMute::IID, Some(&mut ppv))
            .ok()?;
        if ppv.is_null() {
            return None;
        }
        Some(IAudioMute::from_raw(ppv))
    }
}
