//! What Windows knows: the local radio address, whether the Bluetooth link to
//! the headphones is up, which audio render endpoints exist, and which COM
//! port Windows bound to the headphones' SPP service. Non-Windows builds get
//! stubs so the pure logic can still be compiled and tested elsewhere.

#![cfg_attr(not(windows), allow(dead_code))]

use anyhow::Result;
use bmap::Mac;

/// Bose's Bluetooth SIG vendor id as it appears in Windows device instance ids.
pub const BOSE_VID: &str = "VID&0001009E";
pub const SPP_UUID_PREFIX: &str = "{00001101-0000-1000-8000-00805F9B34FB}";

/// One SPP registration found in the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SppPort {
    pub mac: Mac,
    pub port: String,
    pub instance: String,
    pub is_bose: bool,
}

/// Pulls the 12-hex-digit MAC out of a BTHENUM instance key such as
/// `B&28F269D8&0&68F21F370282_C00000000`.
pub fn mac_from_instance(instance: &str) -> Option<Mac> {
    let tail = instance.rsplit("&0&").next()?;
    let hex: String = tail.chars().take(12).collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) || hex == "000000000000" {
        return None;
    }
    Mac::parse(&hex).ok()
}

/// One audio render endpoint as Windows enumerates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub name: String,
    pub enabled: bool,
}

/// Windows names a Bluetooth audio endpoint "Headphones (<device name>)", so
/// match the parenthesised device name; a bare substring would let a device
/// called "phones" match every "Headphones (...)" endpoint on the machine.
pub fn default_endpoint_match(device_name: &str) -> String {
    format!("({device_name})")
}

/// True when `endpoint_match` (case-insensitive) occurs in an enabled render endpoint name.
pub fn endpoint_matches(endpoints: &[Endpoint], endpoint_match: &str) -> bool {
    let needle = endpoint_match.to_lowercase();
    !needle.is_empty()
        && endpoints
            .iter()
            .any(|e| e.enabled && e.name.to_lowercase().contains(&needle))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{anyhow, Context};
    use windows::Devices::Bluetooth::{BluetoothAdapter, BluetoothConnectionStatus, BluetoothDevice};
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Media::Devices::MediaDevice;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    pub fn local_mac() -> Result<Mac> {
        let adapter = BluetoothAdapter::GetDefaultAsync()
            .context("BluetoothAdapter.GetDefaultAsync")?
            .join()
            .context("waiting for the Bluetooth adapter")?;
        Ok(Mac::from_u64(
            adapter.BluetoothAddress().context("reading the adapter address")?,
        ))
    }

    fn device(mac: Mac) -> Result<BluetoothDevice> {
        BluetoothDevice::FromBluetoothAddressAsync(mac.to_u64())
            .context("BluetoothDevice.FromBluetoothAddressAsync")?
            .join()
            .with_context(|| format!("no Bluetooth device record for {mac}; is it paired?"))
    }

    pub fn link_up(mac: Mac) -> Result<bool> {
        let d = device(mac)?;
        Ok(d.ConnectionStatus().context("reading ConnectionStatus")? == BluetoothConnectionStatus::Connected)
    }

    pub fn device_name(mac: Mac) -> Result<String> {
        Ok(device(mac)?.Name().context("reading device Name")?.to_string())
    }

    pub fn render_endpoints() -> Result<Vec<Endpoint>> {
        let selector = MediaDevice::GetAudioRenderSelector().context("GetAudioRenderSelector")?;
        let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)
            .context("FindAllAsyncAqsFilter")?
            .join()
            .context("enumerating audio render endpoints")?;
        let n = devices.Size().context("Size")?;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let d = devices.GetAt(i).context("GetAt")?;
            out.push(Endpoint {
                name: d.Name().context("Name")?.to_string(),
                enabled: d.IsEnabled().unwrap_or(false),
            });
        }
        Ok(out)
    }

    pub fn spp_ports() -> Result<Vec<SppPort>> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let bthenum = hklm
            .open_subkey(r"SYSTEM\CurrentControlSet\Enum\BTHENUM")
            .context("opening HKLM\\SYSTEM\\CurrentControlSet\\Enum\\BTHENUM")?;
        let mut out = Vec::new();
        for key_name in bthenum.enum_keys().flatten() {
            if !key_name.to_uppercase().starts_with(SPP_UUID_PREFIX) {
                continue;
            }
            let Ok(key) = bthenum.open_subkey(&key_name) else {
                continue;
            };
            let is_bose = key_name.to_uppercase().contains(BOSE_VID);
            for instance in key.enum_keys().flatten() {
                let Some(mac) = mac_from_instance(&instance) else {
                    continue;
                };
                let Ok(params) = key.open_subkey(format!("{instance}\\Device Parameters")) else {
                    continue;
                };
                let Ok(port) = params.get_value::<String, _>("PortName") else {
                    continue;
                };
                out.push(SppPort {
                    mac,
                    port,
                    instance: format!("{key_name}\\{instance}"),
                    is_bose,
                });
            }
        }
        if out.is_empty() {
            return Err(anyhow!(
                "no Bluetooth serial ports registered; pair the headphones first"
            ));
        }
        Ok(out)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use anyhow::anyhow;

    fn unsupported<T>() -> Result<T> {
        Err(anyhow!("this needs Windows"))
    }
    pub fn local_mac() -> Result<Mac> {
        unsupported()
    }
    pub fn link_up(_mac: Mac) -> Result<bool> {
        unsupported()
    }
    pub fn device_name(_mac: Mac) -> Result<String> {
        unsupported()
    }
    pub fn render_endpoints() -> Result<Vec<Endpoint>> {
        unsupported()
    }
    pub fn spp_ports() -> Result<Vec<SppPort>> {
        unsupported()
    }
}

pub use imp::{device_name, link_up, local_mac, render_endpoints, spp_ports};

/// The SPP port for `mac`, if Windows registered one.
#[allow(dead_code)]
pub fn spp_port_for(mac: Mac) -> Result<Option<String>> {
    Ok(spp_ports()?.into_iter().find(|p| p.mac == mac).map(|p| p.port))
}

/// Is any active render endpoint the headphones'?
pub fn endpoint_present(endpoint_match: &str) -> Result<bool> {
    Ok(endpoint_matches(&render_endpoints()?, endpoint_match))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_mac_from_instance_ids() {
        let m = mac_from_instance("B&28F269D8&0&68F21F370282_C00000000").unwrap();
        assert_eq!(m.to_string(), "68:F2:1F:37:02:82");
        assert_eq!(mac_from_instance("B&28F269D8&0&000000000000_00000004"), None);
        assert_eq!(mac_from_instance("garbage"), None);
        assert_eq!(mac_from_instance("B&1&0&ZZZZZZZZZZZZ_C0"), None);
    }

    fn ep(name: &str, enabled: bool) -> Endpoint {
        Endpoint {
            name: name.to_string(),
            enabled,
        }
    }

    #[test]
    fn endpoint_matching_is_case_insensitive_and_needs_enabled() {
        let eps = vec![ep("Speakers (Realtek)", true), ep("Headphones (phones)", true)];
        assert!(endpoint_matches(&eps, "(phones)"));
        assert!(endpoint_matches(&eps, "HEADPHONES (PHONES)"));
        assert!(!endpoint_matches(&eps, "buds"));
        assert!(!endpoint_matches(&eps, ""));
        assert!(!endpoint_matches(&[], "(phones)"));
        let disabled = vec![ep("Headphones (phones)", false)];
        assert!(!endpoint_matches(&disabled, "(phones)"));
    }

    #[test]
    fn default_match_does_not_collide_with_other_headphones() {
        let m = default_endpoint_match("phones");
        assert_eq!(m, "(phones)");
        let eps = vec![
            ep("Headphones (Bose QC Ultra 2 HP (USB))", true),
            ep("Headphones (Galaxy Buds Pro)", true),
        ];
        assert!(!endpoint_matches(&eps, &m));
        let with = vec![ep("Headphones (phones)", true)];
        assert!(endpoint_matches(&with, &m));
    }

    #[cfg(not(windows))]
    #[test]
    fn stubs_report_windows_only() {
        assert!(local_mac().is_err());
        assert!(spp_ports().is_err());
        assert!(endpoint_present("x").is_err());
        assert!(spp_port_for(Mac([0; 6])).is_err());
    }
}
