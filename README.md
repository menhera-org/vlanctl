# vlanctl

A systemd `.network`/`.netdev` files generator written in Rust,
specifically made for container/VM hosts which need fast switched
VLAN interfaces.

`/usr/local/lib/systemd/system/vlanctl-generator.service`:

```
[Unit]
Description=Generate VLAN and MACVLAN configuration with vlanctl
DefaultDependencies=no
After=systemd-remount-fs.service
Before=systemd-networkd.service
Before=shutdown.target
Conflicts=shutdown.target

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/vlanctl generate
RemainAfterExit=yes

[Install]
RequiredBy=systemd-networkd.service
```

## Operations

- Generate: `vlanctl generate`. Generates network files at `/run/systemd/network`. The directory is created if not exists: `std::fs::create_dir_all("/run/systemd/network")?;`.
- Apply: `vlanctl apply`. Generate plus `networkctl reload`.
- Generation removes obsolete `.network` and `.netdev` files with the `01-vlanctl-` prefix. Existing kernel interfaces may need to be removed separately.

## Configuration

Configuration is at `/etc/vlanctl.toml` plus `/etc/vlanctl.toml.d/*.toml`
drop-ins. Drop-ins load in lexicographic filename order after the main file.
Tables merge recursively, and later values replace earlier scalars and arrays.
To override a parent's VLAN array, list every VLAN you want to keep for that
parent in the later file. At least one configuration file must exist.

```toml
[parent.ens1]
mtu = 9216 # default: unset
vlan_ifname_prefix = "v" # default
mac_address = "60:a4:34:42:22:22" # default: unset

[[parent.ens1.vlan]]
id = 100

# optional; host macvlan is not created if none.
[parent.ens1.vlan.host_interface]
name = "en-100" # required
mac_address = "60:a4:34:40:00:01" # default: the first 20 bits of SHA256(hostname || NUL || name) fill 60:a4:34:4x:xx:xx
mtu = 1500 # default: unset

[[parent.ens1.vlan]]
id = 200
```

This makes `ens1` managed by vlanctl like:

`/run/systemd/network/01-vlanctl-ens1.network`:

```
[Match]
Name=ens1

[Link]
MACAddress=60:a4:34:42:22:22
MTUBytes=9216

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

VLAN=v100
VLAN=v200
```

`/run/systemd/network/01-vlanctl-v100.netdev`:

```
[NetDev]
Kind=vlan
Name=v100

[VLAN]
Id=100
```

`/run/systemd/network/01-vlanctl-v100.network`:

```
[Match]
Name=v100
Kind=vlan

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

MACVLAN=en-100
```

`/run/systemd/network/01-vlanctl-en-100.netdev`:

```
[NetDev]
Kind=macvlan
Name=en-100
MACAddress=60:a4:34:40:00:01
MTUBytes=1500

[MACVLAN]
Mode=bridge
```

`/run/systemd/network/01-vlanctl-v200.netdev`:

```
[NetDev]
Kind=vlan
Name=v200

[VLAN]
Id=200
```

`/run/systemd/network/01-vlanctl-v200.network`:

```
[Match]
Name=v200
Kind=vlan

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
```

---

Note that this tool does not configure host macvlan interfaces
with its own `.network` files.

## Installation

With the feature `install` (default on), `vlanctl install` command
is available.

It copies its own binary file atomically into `/usr/local/sbin/vlanctl`,
installs its embedded systemd unit at `/usr/local/lib/systemd/system`,
and enables the service. It creates the installation directories as needed.
It also creates `/etc/vlanctl.toml.d/` and installs a commented-out
example at `/etc/vlanctl.toml` if those paths are missing.

## Runtime library dependencies

None. The release build is created with musl, statically linked. It invokes `networkctl` (or `systemctl` on installation) though.

## License

`Apache-2.0 OR MPL-2.0`.
