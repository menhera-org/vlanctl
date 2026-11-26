use crate::Result;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::path::Path;
use toml::{Table, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    pub parent: BTreeMap<String, Parent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Parent {
    #[serde(default = "default_prefix")]
    pub vlan_ifname_prefix: String,
    #[serde(default)]
    pub vlan: Vec<Vlan>,
    pub mtu: Option<NonZeroU32>,
    pub mac_address: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Vlan {
    pub id: u16,
    pub host_interface: Option<HostInterface>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostInterface {
    pub name: String,
    pub mtu: Option<NonZeroU32>,
    pub mac_address: Option<String>,
}

fn default_prefix() -> String {
    "v".into()
}

pub(crate) fn load(main: &Path, dropins: &Path) -> Result<Config> {
    let base = match fs::read_to_string(main) {
        Ok(contents) => Some(parse(main, &contents)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("read {}: {error}", main.display()).into()),
    };

    let mut paths = Vec::new();
    match fs::read_dir(dropins) {
        Ok(entries) => {
            for entry in entries {
                let path = entry?.path();
                let name = path.file_name().unwrap().as_encoded_bytes();
                if !name.starts_with(b".") && path.extension().is_some_and(|ext| ext == "toml") {
                    paths.push(path);
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("read {}: {error}", dropins.display()).into()),
    }

    if base.is_none() && paths.is_empty() {
        return Err(format!(
            "no configuration found at {} or in {}",
            main.display(),
            dropins.display()
        )
        .into());
    }
    paths.sort();
    let mut merged = base.unwrap_or_default();
    for path in paths {
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        merge(&mut merged, parse(&path, &contents)?);
    }
    Value::Table(merged)
        .try_into()
        .map_err(|error| format!("invalid configuration: {error}").into())
}

fn parse(path: &Path, contents: &str) -> Result<Table> {
    toml::from_str(contents).map_err(|error| format!("parse {}: {error}", path.display()).into())
}

fn merge(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(base)), Value::Table(overlay)) => merge(base, overlay),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_dropins_in_filename_order_and_replaces_arrays() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("vlanctl.toml");
        let dropins = dir.path().join("vlanctl.toml.d");
        fs::create_dir(&dropins).unwrap();
        fs::write(
            &main,
            "[parent.eth0]\nmtu = 1500\n[[parent.eth0.vlan]]\nid = 10",
        )
        .unwrap();
        fs::write(dropins.join("20-last.toml"), "[parent.eth0]\nmtu = 9000").unwrap();
        fs::write(
            dropins.join("10-first.toml"),
            "[parent.eth0]\nmtu = 8000\nvlan_ifname_prefix = 'prod'\n[[parent.eth0.vlan]]\nid = 20",
        )
        .unwrap();
        fs::write(dropins.join(".ignored.toml"), "invalid toml ignored").unwrap();

        let config = load(&main, &dropins).unwrap();
        let parent = &config.parent["eth0"];
        assert_eq!(parent.mtu.unwrap().get(), 9000);
        assert_eq!(parent.vlan_ifname_prefix, "prod");
        assert_eq!(parent.vlan.len(), 1);
        assert_eq!(parent.vlan[0].id, 20);
    }
}
