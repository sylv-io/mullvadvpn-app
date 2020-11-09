use std::{
    collections::HashSet,
    ffi::OsStr,
    fs, io,
    net::IpAddr,
    path::{Path, PathBuf},
};

use which::which;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(err_derive::Error, Debug)]
pub enum Error {
    #[error(display = "Failed to detect 'resolvconf' program")]
    NoResolvconf,

    #[error(display = "The resolvconf in PATH is just a symlink to systemd-resolved")]
    ResolvconfUsesResolved,

    #[error(display = "Failed to execute 'resolvconf' program")]
    RunResolvconf(#[error(source)] io::Error),

    #[error(display = "Using 'resolvconf' to add a record failed: {}", stderr)]
    AddRecordError { stderr: String },

    #[error(display = "Using 'resolvconf' to delete a record failed")]
    DeleteRecordError,

    #[error(display = "Detected dnsmasq is runing and misconfigured")]
    DnsmasqMisconfigurationError,

    #[error(display = "Current /etc/resolv.conf is not generated by resolvconf")]
    ResolvconfNotInUseError,
}

pub struct Resolvconf {
    record_names: HashSet<String>,
    resolvconf: PathBuf,
}

impl Resolvconf {
    pub fn new() -> Result<Self> {
        let resolvconf_path = which("resolvconf").map_err(|_| Error::NoResolvconf)?;
        if Self::resolvconf_is_resolved_symlink(&resolvconf_path) {
            return Err(Error::ResolvconfUsesResolved);
        }

        let is_dnsmasq_running = Self::is_dnsmasq_running();

        // Check if resolvconf is managing DNS by /etc/resolv.conf
        if !is_dnsmasq_running
            && !(Self::check_if_resolvconf_is_symlinked_correctly()
                || Self::check_if_resolvconf_was_generated())
        {
            return Err(Error::ResolvconfNotInUseError);
        }

        // Check if resolvconf can manage DNS via dnsmasq
        if is_dnsmasq_running && Self::is_dnsmasq_configured_wrong() {
            return Err(Error::DnsmasqMisconfigurationError);
        }

        Ok(Resolvconf {
            record_names: HashSet::new(),
            resolvconf: resolvconf_path,
        })
    }

    fn resolvconf_is_resolved_symlink(resolvconf_path: &Path) -> bool {
        fs::read_link(resolvconf_path)
            .map(|resolvconf_target| {
                resolvconf_target.file_name() == Some(OsStr::new("resolvectl"))
            })
            .unwrap_or_else(|_| false)
    }

    pub fn set_dns(&mut self, interface: &str, servers: &[IpAddr]) -> Result<()> {
        let record_name = format!("{}.mullvad", interface);
        let mut record_contents = String::new();

        for address in servers {
            record_contents.push_str("nameserver ");
            record_contents.push_str(&address.to_string());
            record_contents.push('\n');
        }

        let output = duct::cmd!(&self.resolvconf, "-a", &record_name)
            .stdin_bytes(record_contents)
            .stderr_capture()
            .unchecked()
            .run()
            .map_err(Error::RunResolvconf)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(Error::AddRecordError { stderr });
        }

        self.record_names.insert(record_name);

        Ok(())
    }

    pub fn reset(&mut self) -> Result<()> {
        let mut result = Ok(());

        for record_name in self.record_names.drain() {
            let output = duct::cmd!(&self.resolvconf, "-d", &record_name, "-f")
                .stderr_capture()
                .unchecked()
                .run()
                .map_err(Error::RunResolvconf)?;

            if !output.status.success() {
                log::error!(
                    "Failed to delete 'resolvconf' record '{}':\n{}",
                    record_name,
                    String::from_utf8_lossy(&output.stderr)
                );
                result = Err(Error::DeleteRecordError);
            }
        }

        result
    }

    fn is_dnsmasq_running() -> bool {
        let pid = match fs::read_to_string("/var/run/dnsmasq/dnsmasq.pid") {
            Ok(pid) => pid,
            Err(_err) => {
                return false;
            }
        };

        PathBuf::from(format!("/proc/{}/", &pid)).exists()
    }

    // Have to check whether dnsmasq has been configured to ignore
    // DNS server lists from external sources
    // Verify if dnsmasq is configured to ignore any external servers
    // by checking for the `no-resolv` config option.
    fn is_dnsmasq_configured_wrong() -> bool {
        let mut config_paths = fs::read_dir("/etc/dnsmasq.d/")
            .map(|entries| {
                entries
                    .into_iter()
                    .filter_map(|entry| entry.ok().map(|e| e.path()))
                    .collect()
            })
            .unwrap_or(vec![]);

        config_paths.push(PathBuf::from("/etc/dnsmasq.conf"));
        config_paths
            .iter()
            .filter_map(|file_path| fs::read(file_path).ok())
            .any(|contents| {
                String::from_utf8_lossy(contents.as_slice())
                    .lines()
                    .any(|line| line.trim().starts_with("no-resolv"))
            })
    }

    // Returns true if /etc/resolv.conf contents indicate that they've been generated by resolvconf
    fn check_if_resolvconf_was_generated() -> bool {
        match fs::read_to_string("/etc/resolv.conf") {
            Ok(contents) => contents.contains("Generated by resolvconf"),
            Err(err) => {
                log::error!("Couldn't read /etc/resolv.conf - {}", err);
                return false;
            }
        }
    }

    // Returns true if /etc/resolv.conf is symlinked to resolvconf's runtime directory
    // (`/var/run/resolvconf`)
    fn check_if_resolvconf_is_symlinked_correctly() -> bool {
        match fs::canonicalize("/etc/resolv.conf") {
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    log::error!("Failed to canonicalize /etc/resolv.conf - {}", err);
                }
                false
            }
            Ok(path) => path.starts_with("/var/run/resolvconf"),
        }
    }
}
