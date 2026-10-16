use std::{
    fs::File,
    io::{Read, Result as IoResult},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

use crate::{Store, download, source::extract};

const PYTHON_RELEASE: &str = "3.12.7+20241016";
const DOWNLOADER_RELEASE: &str = "3.1.4";
const DOWNLOADER_WHEEL: &str = "https://files.pythonhosted.org/packages/65/2a/3ecb3edf72d0f46a0ef0fc9f7b3383c30fea73055e3c474f28311c2a6b39/mangadex_downloader-3.1.4-py3-none-any.whl";
const DOWNLOADER_SHA256: &str = "66d7587cb12358d1f8555bab28f569d8bf0c895a0659c8e203af8ac1313b0982";

pub struct MangaDexDownloader { python: PathBuf }

impl MangaDexDownloader {
    pub async fn prepare() -> Result<Self> {
        let platform = Platform::host()?;
        let root = Store::directory(
            Store::root().join("mangadex").join(PYTHON_RELEASE).join(DOWNLOADER_RELEASE).join(platform.name()),
            move |path| platform.complete(path),
            move |stage| async move { platform.install(&stage).await },
        ).await?;
        Ok(Self { python: root.join(platform.python_path()) })
    }

    #[must_use]
    pub fn command(&self, manga_url: &str, language: &str, output: &Path) -> tokio::process::Command {
        let output = if output.is_absolute() { output.to_owned() } else { std::env::current_dir().expect("current directory is unavailable").join(output) };
        let mut command = tokio::process::Command::new(&self.python);
        command.args(["-m", "mangadex_downloader", manga_url, "--language", language, "--save-as", "raw", "--path"]);
        command.arg(output);
        command
    }

    #[cfg(test)]
    fn for_test(python: PathBuf) -> Self { Self { python } }
}

#[derive(Clone, Copy, Debug)]
enum Platform { MacosArm64, WindowsX64, LinuxX64, LinuxArm64 }
struct Artifact { url: &'static str, sha256: &'static str }

impl Platform {
    fn host() -> Result<Self> {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) { Ok(Self::MacosArm64) }
        else if cfg!(all(target_os = "windows", target_arch = "x86_64")) { Ok(Self::WindowsX64) }
        else if cfg!(all(target_os = "linux", target_arch = "x86_64")) { Ok(Self::LinuxX64) }
        else if cfg!(all(target_os = "linux", target_arch = "aarch64")) { Ok(Self::LinuxArm64) }
        else { anyhow::bail!("MangaDex Downloader does not support this target") }
    }
    const fn name(self) -> &'static str { match self { Self::MacosArm64 => "aarch64-apple-darwin", Self::WindowsX64 => "x86_64-pc-windows-msvc", Self::LinuxX64 => "x86_64-unknown-linux-gnu", Self::LinuxArm64 => "aarch64-unknown-linux-gnu" } }
    const fn artifact(self) -> Artifact {
        // Python is PSF licensed; each archive keeps its LICENSE. MangaDex Downloader is MIT licensed.
        match self {
            Self::MacosArm64 => Artifact { url: "https://github.com/indygreg/python-build-standalone/releases/download/20241016/cpython-3.12.7%2B20241016-aarch64-apple-darwin-install_only.tar.gz", sha256: "4c18852bf9c1a11b56f21bcf0df1946f7e98ee43e9e4c0c5374b2b3765cf9508" },
            Self::WindowsX64 => Artifact { url: "https://github.com/indygreg/python-build-standalone/releases/download/20241016/cpython-3.12.7%2B20241016-x86_64-pc-windows-msvc-install_only.tar.gz", sha256: "f05531bff16fa77b53be0776587b97b466070e768e6d5920894de988bdcd547a" },
            Self::LinuxX64 => Artifact { url: "https://github.com/indygreg/python-build-standalone/releases/download/20241016/cpython-3.12.7%2B20241016-x86_64-unknown-linux-gnu-install_only.tar.gz", sha256: "43576f7db1033dd57b900307f09c2e86f371152ac8a2607133afa51cbfc36064" },
            Self::LinuxArm64 => Artifact { url: "https://github.com/indygreg/python-build-standalone/releases/download/20241016/cpython-3.12.7%2B20241016-aarch64-unknown-linux-gnu-install_only.tar.gz", sha256: "bba3c6be6153f715f2941da34f3a6a69c2d0035c9c5396bc5bb68c6d2bd1065a" },
        }
    }
    const fn python_path(self) -> &'static str { match self { Self::WindowsX64 => "python/python.exe", _ => "python/bin/python3" } }
    fn required_files(self) -> [&'static str; 4] { [self.python_path(), "python/LICENSE", "python/lib/python3.12/site-packages/mangadex_downloader/__main__.py", "python/lib/python3.12/site-packages/mangadex_downloader-3.1.4.dist-info/licenses/LICENSE"] }
    fn complete(self, root: &Path) -> bool { self.required_files().iter().all(|path| root.join(path).is_file()) }
    async fn install(self, stage: &Path) -> Result<()> {
        let artifact = self.artifact();
        let archive = tempfile::Builder::new().suffix(".tar.gz").tempfile()?;
        download::fetch(artifact.url, archive.path()).await?;
        validate_sha256(archive.path(), artifact.sha256)?;
        extract(archive.path(), stage, &["python/**"])?;
        let wheel = tempfile::Builder::new().suffix(".whl").tempfile()?;
        download::fetch(DOWNLOADER_WHEEL, wheel.path()).await?;
        validate_sha256(wheel.path(), DOWNLOADER_SHA256)?;
        let status = tokio::process::Command::new(stage.join(self.python_path()))
            .args(["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "--target"])
            .arg(stage.join("python/lib/python3.12/site-packages")).arg(wheel.path()).status().await
            .context("failed to run managed Python pip")?;
        ensure!(status.success(), "managed Python failed to install MangaDex Downloader");
        Ok(())
    }
}

fn validate_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    copy_to_hasher(&mut file, &mut hasher)
        .with_context(|| format!("failed to hash {}", path.display()))?;
    ensure!(
        format!("{:x}", hasher.finalize()) == expected,
        "SHA-256 mismatch for {}",
        path.display()
    );
    Ok(())
}

fn copy_to_hasher(reader: &mut impl Read, hasher: &mut Sha256) -> IoResult<()> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 { return Ok(()); }
        hasher.update(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use super::{MangaDexDownloader, Platform};
    #[test] fn selects_an_artifact_for_every_shipping_target() { for target in [Platform::MacosArm64, Platform::WindowsX64, Platform::LinuxX64, Platform::LinuxArm64] { let artifact = target.artifact(); assert!(artifact.url.starts_with("https://")); assert_eq!(artifact.sha256.len(), 64); } }
    #[test] fn accepts_only_complete_runtime_caches() { let root = tempfile::tempdir().unwrap(); let target = Platform::LinuxX64; assert!(!target.complete(root.path())); for path in target.required_files() { let path = root.path().join(path); std::fs::create_dir_all(path.parent().unwrap()).unwrap(); std::fs::write(path, b"ready").unwrap(); } assert!(target.complete(root.path())); }
    #[test] fn rejects_an_incomplete_runtime_cache() { let root = tempfile::tempdir().unwrap(); let target = Platform::WindowsX64; let python = root.path().join(target.python_path()); std::fs::create_dir_all(python.parent().unwrap()).unwrap(); std::fs::write(python, b"ready").unwrap(); assert!(!target.complete(root.path())); }
    #[test] fn accepts_a_matching_python_archive_hash() { let file = tempfile::NamedTempFile::new().unwrap(); std::fs::write(file.path(), b"hello").unwrap(); super::validate_sha256(file.path(), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824").unwrap(); }
    #[test] fn rejects_a_tampered_downloader_wheel() { let file = tempfile::NamedTempFile::new().unwrap(); std::fs::write(file.path(), b"tampered").unwrap(); assert!(super::validate_sha256(file.path(), "66d7587cb12358d1f8555bab28f569d8bf0c895a0659c8e203af8ac1313b0982").is_err()); }
    #[test] fn builds_a_raw_image_command_with_absolute_output() { let root = tempfile::tempdir().unwrap(); let downloader = MangaDexDownloader::for_test(root.path().join("python")); let command = downloader.command("https://mangadex.org/title/abc", "ja", Path::new("downloads")); let actual = command.get_args().map(|value| value.to_string_lossy().into_owned()).collect::<Vec<_>>(); assert_eq!(actual, vec!["-m", "mangadex_downloader", "https://mangadex.org/title/abc", "--language", "ja", "--save-as", "raw", "--path", &std::env::current_dir().unwrap().join("downloads").display().to_string()]); }
}
