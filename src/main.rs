mod config;
mod network;

use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, ExitCode};
use tempfile::{NamedTempFile, TempPath};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const CONFIG_FILE: &str = "/etc/vlanctl.toml";
const CONFIG_DROPINS: &str = "/etc/vlanctl.toml.d";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vlanctl: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let usage = if cfg!(feature = "install") {
        "Usage: vlanctl <generate|apply|install>"
    } else {
        "Usage: vlanctl <generate|apply>"
    };
    let mut args = env::args_os().skip(1);
    let command = args.next();
    if args.next().is_some() {
        return Err(usage.into());
    }
    match command.as_deref().and_then(|arg| arg.to_str()) {
        Some("generate") => generate(false),
        Some("apply") => generate(true),
        #[cfg(feature = "install")]
        Some("install") => install(),
        Some("-h" | "--help") => {
            println!("{usage}");
            Ok(())
        }
        _ => Err(usage.into()),
    }
}

fn generate(reload: bool) -> Result<()> {
    let dir = Path::new("/run/systemd/network");
    fs::create_dir_all(dir)?;
    let directory = File::open(dir)?;
    directory.lock()?;

    let config = config::load(Path::new(CONFIG_FILE), Path::new(CONFIG_DROPINS))?;
    let hostname = fs::read("/proc/sys/kernel/hostname")
        .map_err(|error| format!("read kernel hostname: {error}"))?;
    let files = network::render(config, hostname.strip_suffix(b"\n").unwrap_or(&hostname))?;

    let mut stale = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(name) = name.to_str()
            && name.starts_with(network::FILE_PREFIX)
            && (name.ends_with(".network") || name.ends_with(".netdev"))
            && !files.contains_key(name)
            && entry.file_type()?.is_file()
        {
            stale.push(entry.path());
        }
    }

    let mut pending = Vec::new();
    for (name, contents) in files {
        let path = dir.join(name);
        if fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file())
            && fs::read(&path).is_ok_and(|current| current == contents.as_bytes())
        {
            continue;
        }
        let temp = stage(dir, 0o644, |file| file.write_all(contents.as_bytes()))?;
        pending.push((temp, path));
    }
    for (temp, path) in pending {
        publish(temp, &path)?;
    }
    for path in stale {
        fs::remove_file(&path).map_err(|error| format!("remove {}: {error}", path.display()))?;
    }
    if reload {
        command("networkctl", &["--no-pager", "reload"])?;
    }
    Ok(())
}

fn stage(
    dir: &Path,
    mode: u32,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<TempPath> {
    let result = (|| -> io::Result<_> {
        let mut temp = NamedTempFile::with_prefix_in(".vlanctl-", dir)?;
        write(temp.as_file_mut())?;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
        temp.as_file().sync_all()?;
        Ok(temp.into_temp_path())
    })();
    result.map_err(|error| format!("stage file in {}: {error}", dir.display()).into())
}

fn publish(temp: TempPath, path: &Path) -> Result<()> {
    temp.persist(path)
        .map_err(|error| format!("write {}: {}", path.display(), error.error))?;
    Ok(())
}

fn command(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("run {program}: {error}"))?;
    if !status.success() {
        return Err(format!("{program} {} failed ({status})", args.join(" ")).into());
    }
    Ok(())
}

#[cfg(feature = "install")]
fn install() -> Result<()> {
    const SERVICE: &str = "vlanctl-generator.service";

    let bin_dir = Path::new("/usr/local/sbin");
    let unit_dir = Path::new("/usr/local/lib/systemd/system");
    fs::create_dir_all(bin_dir)?;
    fs::create_dir_all(unit_dir)?;
    let directory = File::open(unit_dir)?;
    directory.lock()?;

    // /proc/self/exe still identifies the running executable if it has been replaced.
    let mut executable = File::open("/proc/self/exe")?;
    let binary = stage(bin_dir, 0o755, |file| {
        io::copy(&mut executable, file)?;
        Ok(())
    })?;
    let service = stage(unit_dir, 0o644, |file| {
        file.write_all(include_bytes!("../vlanctl-generator.service"))
    })?;

    fs::create_dir_all(CONFIG_DROPINS)
        .map_err(|error| format!("create {CONFIG_DROPINS}: {error}"))?;
    let config = stage(Path::new("/etc"), 0o644, |file| {
        file.write_all(include_bytes!("../vlanctl.toml.example"))
    })?;
    match config.persist_noclobber(CONFIG_FILE) {
        Ok(()) => {}
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("create {CONFIG_FILE}: {}", error.error).into()),
    }

    publish(binary, &bin_dir.join("vlanctl"))?;
    publish(service, &unit_dir.join(SERVICE))?;
    command("systemctl", &["--no-pager", "enable", SERVICE])
}
