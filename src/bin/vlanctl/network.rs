use crate::Result;
use crate::config::Config;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::num::NonZeroU32;

pub(crate) const FILE_PREFIX: &str = "01-vlanctl-";

const NETWORK: &str = "\
[Network]
DHCP=no
LinkLocalAddressing=no
LLMNR=no
LLDP=no
EmitLLDP=no
Address=
IPv6AcceptRA=no
ConfigureWithoutCarrier=yes
IgnoreCarrierLoss=yes
";

pub(crate) fn render(config: Config, hostname: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut names = BTreeSet::new();
    let mut files = BTreeMap::new();

    for (name, parent) in config.parent {
        reserve_name(&mut names, &name)?;
        let mut parent_network = format!("[Match]\nName={name}\n\n");
        if parent.mac_address.is_some() || parent.mtu.is_some() {
            parent_network.push_str("[Link]\n");
            append_link(
                &mut parent_network,
                &name,
                parent.mac_address.as_deref(),
                parent.mtu,
            )?;
            parent_network.push('\n');
        }
        parent_network.push_str(NETWORK);
        if !parent.vlan.is_empty() {
            parent_network.push('\n');
        }

        for vlan in parent.vlan {
            let id = vlan.id;
            if id > 4094 {
                return Err(format!("invalid VLAN ID {id} on {name}; expected 0 to 4094").into());
            }
            let vlan_name = format!("{}{id}", parent.vlan_ifname_prefix);
            reserve_name(&mut names, &vlan_name)?;
            writeln!(parent_network, "VLAN={vlan_name}")?;

            let mut vlan_network = format!("[Match]\nName={vlan_name}\nKind=vlan\n\n{NETWORK}");
            if let Some(host) = vlan.host_interface {
                reserve_name(&mut names, &host.name)?;
                writeln!(vlan_network, "\nMACVLAN={}", host.name)?;
                let mut netdev = format!("[NetDev]\nKind=macvlan\nName={}\n", host.name);
                let mac = host
                    .mac_address
                    .unwrap_or_else(|| host_mac(hostname, &host.name));
                append_link(&mut netdev, &host.name, Some(&mac), host.mtu)?;
                netdev.push_str("\n[MACVLAN]\nMode=bridge\n");
                files.insert(format!("{FILE_PREFIX}{}.netdev", host.name), netdev);
            }
            files.insert(
                format!("{FILE_PREFIX}{vlan_name}.netdev"),
                format!("[NetDev]\nKind=vlan\nName={vlan_name}\n\n[VLAN]\nId={id}\n"),
            );
            files.insert(format!("{FILE_PREFIX}{vlan_name}.network"), vlan_network);
        }
        files.insert(format!("{FILE_PREFIX}{name}.network"), parent_network);
    }

    Ok(files)
}

fn reserve_name(names: &mut BTreeSet<String>, name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 15
        || matches!(name, "." | "..")
        || name
            .bytes()
            .any(|byte| !byte.is_ascii_graphic() || b"/:%\\\"'!?*[]".contains(&byte))
    {
        return Err(format!("invalid interface name {name:?}").into());
    }
    if !names.insert(name.to_owned()) {
        return Err(format!("interface name {name:?} is used more than once").into());
    }
    Ok(())
}

fn append_link(
    output: &mut String,
    name: &str,
    mac: Option<&str>,
    mtu: Option<NonZeroU32>,
) -> Result<()> {
    if let Some(mac) = mac {
        if mac.len() != 17
            || !mac
                .split(':')
                .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(format!("invalid MAC address {mac:?} for {name}").into());
        }
        writeln!(output, "MACAddress={mac}")?;
    }
    if let Some(mtu) = mtu {
        writeln!(output, "MTUBytes={mtu}")?;
    }
    Ok(())
}

fn host_mac(hostname: &[u8], name: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(hostname);
    hash.update([0]);
    hash.update(name.as_bytes());
    let digest = hash.finalize();
    let a = 0x40 | (digest[0] >> 4);
    let b = (digest[0] << 4) | (digest[1] >> 4);
    let c = (digest[1] << 4) | (digest[2] >> 4);
    format!("60:a4:34:{a:02x}:{b:02x}:{c:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_readme_example() {
        let mut blocks = include_str!("../../../README.md").split("\x60\x60\x60");
        let service = blocks.nth(1).unwrap();
        #[cfg(feature = "install")]
        assert_eq!(
            service.trim_start(),
            include_str!("../../../vlanctl-generator.service")
        );
        #[cfg(not(feature = "install"))]
        let _ = service;

        let example = blocks.nth(1).unwrap().strip_prefix("toml\n").unwrap();
        let files = render(toml::from_str(example).unwrap(), b"smoketest").unwrap();
        let names = [
            "ens1.network",
            "v100.netdev",
            "v100.network",
            "en-100.netdev",
            "v200.netdev",
            "v200.network",
        ];
        assert_eq!(files.len(), names.len());
        for (name, example) in names.into_iter().zip(blocks.skip(1).step_by(2)) {
            assert_eq!(files[&format!("{FILE_PREFIX}{name}")], example.trim_start());
        }
    }

    #[test]
    fn derives_a_mac_from_the_first_20_digest_bits() {
        assert_eq!(host_mac(b"smoketest", "en-100"), "60:a4:34:40:03:c5");
    }

    #[test]
    fn rejects_duplicate_and_pattern_names() {
        let config =
            toml::from_str("[parent.eth0]\n[[parent.eth0.vlan]]\nid = 10\n[parent.v10]").unwrap();
        assert!(
            render(config, b"smoketest")
                .unwrap_err()
                .to_string()
                .contains("used more than once")
        );
        let config = toml::from_str("[parent.'eth*']").unwrap();
        assert!(
            render(config, b"smoketest")
                .unwrap_err()
                .to_string()
                .contains("invalid interface name")
        );
    }
}
