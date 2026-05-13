use std::path::PathBuf;

pub struct Paths {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub pgdata_dir: PathBuf,
    pub raw_dir: PathBuf,
    pub inbox_dir: PathBuf,
    pub dead_letter_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub socket_path: PathBuf,
    pub pid_file: PathBuf,
}

impl Paths {
    pub fn resolve() -> std::io::Result<Self> {
        #[cfg(unix)]
        let (data, config) = {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME unset"))?;
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("teramind");
            let conf = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("teramind");
            (data, conf)
        };
        #[cfg(windows)]
        let (data, config) = {
            let local = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA unset")
                })?;
            let app = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| local.clone());
            (local.join("teramind").join("data"), app.join("teramind"))
        };

        let socket_path = teramind_ipc::transport::default_socket_path();
        Ok(Paths {
            pgdata_dir: data.join("pgdata"),
            raw_dir: data.join("raw"),
            inbox_dir: data.join("inbox"),
            dead_letter_dir: data.join("dead_letter"),
            logs_dir: data.join("logs"),
            pid_file: data.join("teramindd.pid"),
            data_dir: data,
            config_dir: config,
            socket_path,
        })
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for d in [
            &self.data_dir,
            &self.config_dir,
            &self.pgdata_dir,
            &self.raw_dir,
            &self.inbox_dir,
            &self.dead_letter_dir,
            &self.logs_dir,
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ensure_dirs_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_DATA_HOME", tmp.path().join("xdg-data"));
        std::env::set_var("XDG_CONFIG_HOME", tmp.path().join("xdg-config"));
        #[cfg(windows)]
        {
            std::env::set_var("LOCALAPPDATA", tmp.path());
            std::env::set_var("APPDATA", tmp.path());
        }
        let p = Paths::resolve().unwrap();
        p.ensure_dirs().unwrap();
        p.ensure_dirs().unwrap();
        assert!(p.data_dir.exists());
        assert!(p.raw_dir.exists());
    }
}
