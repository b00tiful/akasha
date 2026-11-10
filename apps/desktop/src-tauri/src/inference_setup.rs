//! Explicit offline import into the fixed native installation. No network or execution.
use crate::{
    DesktopError,
    inference::{self, Cancellation},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Asset {
    Model,
    Vulkan,
    Cpu,
}

#[derive(Debug, Serialize)]
pub struct SetupResult {
    pub asset: Asset,
    pub outcome: &'static str,
    pub bytes: u64,
    pub location: PathBuf,
}

pub async fn import_asset(
    asset: Asset,
    source: PathBuf,
    cancellation: Cancellation,
) -> Result<SetupResult, DesktopError> {
    let base = inference::installation()?;
    #[cfg(target_os = "linux")]
    {
        linux::install_at(&base, asset, &source, &cancellation).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (base, asset, source, cancellation);
        Err(inference::error("Offline setup requires Linux x86-64"))
    }
}

pub async fn verify_asset(
    asset: Asset,
    cancellation: Cancellation,
) -> Result<SetupResult, DesktopError> {
    let base = inference::installation()?;
    #[cfg(target_os = "linux")]
    {
        linux::verify_at(&base, asset, &cancellation).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (base, asset, cancellation);
        Err(inference::error("Offline setup requires Linux x86-64"))
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn import_local_inference_asset(
    jobs: tauri::State<'_, inference::InferenceJobs>,
    request_id: String,
    asset: Asset,
    source: PathBuf,
) -> Result<SetupResult, DesktopError> {
    let lease = jobs.claim(&request_id)?;
    import_asset(asset, source, lease.cancellation.clone()).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn verify_local_inference_asset(
    jobs: tauri::State<'_, inference::InferenceJobs>,
    request_id: String,
    asset: Asset,
) -> Result<SetupResult, DesktopError> {
    let lease = jobs.claim(&request_id)?;
    verify_asset(asset, lease.cancellation.clone()).await
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use inference::{MODEL_SHA256, Profile, error};
    use rustix::fs::{self as rfs, AtFlags, Mode, OFlags, RenameFlags};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::{
        collections::BTreeMap,
        fs::File,
        os::unix::fs::MetadataExt,
        path::{Component, Path},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const MODEL_BYTES: u64 = 5_966_095_584;
    const RUNTIME_FILE_LIMIT: u64 = 256 * 1024 * 1024;
    const MODEL_NAME: &str = "Qwen3.5-9B-UD-Q4_K_XL.gguf";

    #[derive(Clone)]
    enum Pin {
        File {
            hash: String,
            bytes: Option<u64>,
            executable: bool,
        },
        Link(String),
    }
    fn inventory(asset: Asset) -> BTreeMap<String, Pin> {
        if matches!(asset, Asset::Model) {
            return BTreeMap::from([(
                MODEL_NAME.into(),
                Pin::File {
                    hash: MODEL_SHA256.into(),
                    bytes: Some(MODEL_BYTES),
                    executable: false,
                },
            )]);
        }
        let pins: Value = serde_json::from_str(include_str!("inference-runtime-pins.json"))
            .expect("compiled pins");
        pins[if matches!(asset, Asset::Cpu) {
            "cpu"
        } else {
            "vulkan"
        }]
        .as_object()
        .expect("compiled inventory")
        .iter()
        .map(|(name, value)| {
            let pin = if let Some(link) = value["link"].as_str() {
                Pin::Link(link.into())
            } else {
                Pin::File {
                    hash: value["sha256"].as_str().expect("compiled hash").into(),
                    bytes: None,
                    executable: name != "LICENSE",
                }
            };
            (name.clone(), pin)
        })
        .collect()
    }
    fn destination(asset: Asset) -> (&'static str, &'static str) {
        match asset {
            Asset::Model => ("models", MODEL_NAME),
            Asset::Vulkan => ("runtimes", Profile::Vulkan.folder()),
            Asset::Cpu => ("runtimes", Profile::Cpu.folder()),
        }
    }
    fn io_error(context: &str, e: impl std::fmt::Display) -> DesktopError {
        error(format!("{context}: {e}"))
    }
    fn open_child(
        parent: &File,
        name: &std::ffi::OsStr,
        flags: OFlags,
    ) -> Result<File, DesktopError> {
        rfs::openat(parent, name, flags | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK, Mode::empty()).map(File::from).map_err(|e| io_error("Cannot open local setup path (regular files/directories without symlinks required)", e))
    }
    // Walk descriptors, not canonicalized strings: symlinked ancestors cannot redirect the read/write.
    fn directory(path: &Path, create: bool) -> Result<File, DesktopError> {
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err(error(
                "Setup requires an absolute local path of at most 4096 bytes",
            ));
        }
        let mut dir = File::open("/").map_err(|e| io_error("Cannot open filesystem root", e))?;
        for part in path.components() {
            let Component::Normal(name) = part else {
                if matches!(part, Component::RootDir) {
                    continue;
                }
                return Err(error(
                    "Setup paths cannot contain parent or special components",
                ));
            };
            if create {
                match rfs::mkdirat(&dir, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => (),
                    Err(e) => return Err(io_error("Cannot create private setup directory", e)),
                }
            }
            dir = open_child(&dir, name, OFlags::RDONLY | OFlags::DIRECTORY)?;
        }
        Ok(dir)
    }
    fn private(dir: &File) -> Result<(), DesktopError> {
        let meta = dir
            .metadata()
            .map_err(|e| io_error("Cannot inspect installation directory", e))?;
        if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o022 != 0 {
            return Err(error(
                "Installation directories must belong to you and deny group/other writes",
            ));
        }
        Ok(())
    }
    async fn stream(
        source: File,
        mut output: Option<tokio::fs::File>,
        pin: &Pin,
        cancellation: &Cancellation,
    ) -> Result<u64, DesktopError> {
        let Pin::File {
            hash: expected,
            bytes,
            ..
        } = pin
        else {
            return Err(error("Invalid file pin"));
        };
        cancellation.check()?;
        let meta = source
            .metadata()
            .map_err(|e| io_error("Cannot inspect source file", e))?;
        let limit = bytes.unwrap_or(RUNTIME_FILE_LIMIT);
        if !meta.is_file() || meta.len() > limit || bytes.is_some_and(|length| meta.len() != length)
        {
            return Err(error(
                "Artifact is not a regular file of the required bounded size",
            ));
        }
        let mut input = tokio::fs::File::from_std(source);
        let mut buffer = vec![0; 1024 * 1024];
        let mut hash = Sha256::new();
        let mut total = 0;
        loop {
            cancellation.check()?;
            let length = input
                .read(&mut buffer)
                .await
                .map_err(|e| io_error("Cannot read setup artifact", e))?;
            if length == 0 {
                break;
            }
            total += length as u64;
            if total > limit {
                return Err(error("Artifact grew beyond its byte limit"));
            }
            hash.update(&buffer[..length]);
            if let Some(output) = &mut output {
                output
                    .write_all(&buffer[..length])
                    .await
                    .map_err(|e| io_error("Cannot stage artifact (check free disk space)", e))?;
            }
        }
        let digest = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if digest != *expected || total != meta.len() {
            return Err(error(
                "Artifact SHA-256 or byte count differs from the selected pinned version",
            ));
        }
        if let Some(output) = output {
            output
                .sync_all()
                .await
                .map_err(|e| io_error("Cannot sync staged artifact", e))?;
        }
        cancellation.check()?;
        Ok(total)
    }
    fn source_file(path: &Path) -> Result<File, DesktopError> {
        let parent = directory(
            path.parent()
                .ok_or_else(|| error("Source file needs a parent"))?,
            false,
        )?;
        open_child(
            &parent,
            path.file_name()
                .ok_or_else(|| error("Source file needs a name"))?,
            OFlags::RDONLY,
        )
    }
    fn check_extra_libraries(dir: &File, pins: &BTreeMap<String, Pin>) -> Result<(), DesktopError> {
        // /proc refers to our already-open directory, never to a caller-selected path.
        use std::os::fd::AsRawFd;
        let mut count = 0;
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))
            .map_err(|e| io_error("Cannot inspect runtime inventory", e))?
        {
            count += 1;
            if count > 256 {
                return Err(error("Runtime directory exceeds 256 entries"));
            }
            let entry = entry.map_err(|e| io_error("Cannot inspect runtime entry", e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("lib") && name.contains(".so") && !pins.contains_key(&name) {
                return Err(error("Unrecognized library in runtime package"));
            }
        }
        Ok(())
    }
    async fn verify_runtime(
        dir: &File,
        pins: &BTreeMap<String, Pin>,
        cancellation: &Cancellation,
    ) -> Result<u64, DesktopError> {
        check_extra_libraries(dir, pins)?;
        let mut bytes = 0;
        for (name, pin) in pins {
            cancellation.check()?;
            match pin {
                Pin::File { executable, .. } => {
                    let file = open_child(dir, name.as_ref(), OFlags::RDONLY)?;
                    if *executable
                        && file
                            .metadata()
                            .map_err(|e| io_error("Cannot inspect runtime permissions", e))?
                            .mode()
                            & 0o100
                            == 0
                    {
                        return Err(error("Runtime executable permission is missing"));
                    }
                    bytes += stream(file, None, pin, cancellation).await?;
                }
                Pin::Link(target) => {
                    let link = rfs::readlinkat(dir, name, Vec::new())
                        .map_err(|e| io_error("Cannot read pinned runtime link", e))?;
                    if link.to_bytes() != target.as_bytes() {
                        return Err(error("Runtime symlink differs from pinned archive"));
                    }
                }
            }
        }
        Ok(bytes)
    }
    async fn verify_destination(
        parent: &File,
        name: &str,
        asset: Asset,
        pins: &BTreeMap<String, Pin>,
        cancellation: &Cancellation,
    ) -> Result<u64, DesktopError> {
        if matches!(asset, Asset::Model) {
            stream(
                open_child(parent, name.as_ref(), OFlags::RDONLY)?,
                None,
                &pins[MODEL_NAME],
                cancellation,
            )
            .await
        } else {
            verify_runtime(
                &open_child(parent, name.as_ref(), OFlags::RDONLY | OFlags::DIRECTORY)?,
                pins,
                cancellation,
            )
            .await
        }
    }
    pub(super) async fn verify_at(
        base: &Path,
        asset: Asset,
        cancellation: &Cancellation,
    ) -> Result<SetupResult, DesktopError> {
        let (folder, name) = destination(asset);
        let parent = directory(&base.join(folder), false)?;
        let bytes =
            verify_destination(&parent, name, asset, &inventory(asset), cancellation).await?;
        Ok(SetupResult {
            asset,
            outcome: "verified",
            bytes,
            location: base.join(folder).join(name),
        })
    }
    struct Lock {
        parent: File,
    }
    impl Drop for Lock {
        fn drop(&mut self) {
            let _ = rfs::unlinkat(&self.parent, ".setup-lock", AtFlags::empty());
        }
    }
    struct Stage {
        parent: File,
        name: String,
        directory: Option<File>,
        entries: Vec<String>,
        created: bool,
        committed: bool,
    }
    impl Drop for Stage {
        fn drop(&mut self) {
            if self.committed || !self.created {
                return;
            }
            if let Some(dir) = &self.directory {
                for name in &self.entries {
                    let _ = rfs::unlinkat(dir, name, AtFlags::empty());
                }
            }
            let flags = if self.directory.is_some() {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            };
            let _ = rfs::unlinkat(&self.parent, &self.name, flags);
        }
    }
    pub(super) async fn install_at(
        base: &Path,
        asset: Asset,
        source: &Path,
        cancellation: &Cancellation,
    ) -> Result<SetupResult, DesktopError> {
        install_inventory(base, asset, source, cancellation, &inventory(asset)).await
    }
    async fn install_inventory(
        base: &Path,
        asset: Asset,
        source: &Path,
        cancellation: &Cancellation,
        pins: &BTreeMap<String, Pin>,
    ) -> Result<SetupResult, DesktopError> {
        cancellation.check()?;
        // Check source path syntax before creating even an empty destination.
        if !source.is_absolute()
            || source.as_os_str().len() > 4096
            || source
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(error(
                "Choose an absolute local source path without parent traversal",
            ));
        }
        let dir = directory(base, true)?;
        private(&dir)?;
        let lock = rfs::openat(&dir, ".setup-lock", OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::from_raw_mode(0o600))
            .map_err(|_| error("Another setup owns the installation lock. If a previous process crashed, inspect .setup-lock and .import-* before removing them manually"))?;
        let _lock_file = File::from(lock);
        let _lock = Lock { parent: dir };
        let (folder, name) = destination(asset);
        let parent = directory(&base.join(folder), true)?;
        private(&parent)?;
        match rfs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => {
                let bytes = verify_destination(&parent, name, asset, pins, cancellation).await.map_err(|e| error(format!("Existing asset is invalid; it was preserved. Move it aside explicitly before importing again. {}", e.message)))?;
                return Ok(SetupResult {
                    asset,
                    outcome: "already-installed",
                    bytes,
                    location: base.join(folder).join(name),
                });
            }
            Err(rustix::io::Errno::NOENT) => (),
            Err(e) => return Err(io_error("Cannot inspect installation destination", e)),
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| error("OS randomness unavailable"))?;
        let stage_name = format!(
            ".import-{}",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let mut stage = Stage {
            parent,
            name: stage_name,
            directory: None,
            entries: vec![],
            created: false,
            committed: false,
        };
        let bytes = if matches!(asset, Asset::Model) {
            let input = source_file(source)?;
            let output = rfs::openat(
                &stage.parent,
                &stage.name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|e| io_error("Cannot stage model", e))?;
            stage.created = true;
            stream(
                input,
                Some(tokio::fs::File::from_std(File::from(output))),
                &pins[MODEL_NAME],
                cancellation,
            )
            .await?;
            // Verify stored bytes too, before atomic publication.
            stream(
                open_child(&stage.parent, stage.name.as_ref(), OFlags::RDONLY)?,
                None,
                &pins[MODEL_NAME],
                cancellation,
            )
            .await?
        } else {
            let input = directory(source, false)?;
            check_extra_libraries(&input, pins)?;
            rfs::mkdirat(&stage.parent, &stage.name, Mode::from_raw_mode(0o700))
                .map_err(|e| io_error("Cannot stage runtime", e))?;
            stage.created = true;
            stage.directory = Some(open_child(
                &stage.parent,
                stage.name.as_ref(),
                OFlags::RDONLY | OFlags::DIRECTORY,
            )?);
            let output = stage.directory.as_ref().unwrap();
            for (entry, pin) in pins {
                cancellation.check()?;
                stage.entries.push(entry.clone());
                match pin {
                    Pin::File { executable, .. } => {
                        let file = open_child(&input, entry.as_ref(), OFlags::RDONLY)?;
                        let target = rfs::openat(
                            output,
                            entry,
                            OFlags::WRONLY
                                | OFlags::CREATE
                                | OFlags::EXCL
                                | OFlags::NOFOLLOW
                                | OFlags::CLOEXEC,
                            Mode::from_raw_mode(if *executable { 0o700 } else { 0o600 }),
                        )
                        .map_err(|e| io_error("Cannot stage runtime file", e))?;
                        stream(
                            file,
                            Some(tokio::fs::File::from_std(File::from(target))),
                            pin,
                            cancellation,
                        )
                        .await?;
                    }
                    Pin::Link(target) => {
                        let original = rfs::readlinkat(&input, entry, Vec::new())
                            .map_err(|e| io_error("Cannot read source runtime link", e))?;
                        if original.to_bytes() != target.as_bytes() {
                            return Err(error("Source runtime link differs from pinned archive"));
                        }
                        rfs::symlinkat(target, output, entry)
                            .map_err(|e| io_error("Cannot stage pinned runtime link", e))?;
                    }
                }
            }
            let bytes = verify_runtime(output, pins, cancellation).await?;
            output
                .sync_all()
                .map_err(|e| io_error("Cannot sync runtime directory", e))?;
            bytes
        };
        cancellation.check()?;
        rfs::renameat_with(
            &stage.parent,
            &stage.name,
            &stage.parent,
            name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| io_error("Cannot publish installation; destination preserved", e))?;
        stage.committed = true;
        // Publication is the cancellation boundary; never report a committed asset as cancelled.
        stage.parent.sync_all().map_err(|e| {
            error(format!(
                "Asset was installed, but directory sync failed; verify it before use: {e}"
            ))
        })?;
        Ok(SetupResult {
            asset,
            outcome: "installed",
            bytes,
            location: base.join(folder).join(name),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            fs,
            os::unix::fs::{PermissionsExt, symlink},
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        struct Fixture {
            base: PathBuf,
            install: PathBuf,
            source: PathBuf,
        }
        impl Fixture {
            fn new() -> Self {
                let base = std::env::temp_dir().join(format!(
                    "akasha-offline-setup-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                fs::create_dir(&base).unwrap();
                let source = base.join("source");
                fs::create_dir(&source).unwrap();
                Self {
                    install: base.join("installation"),
                    source,
                    base,
                }
            }
            fn runtime(&self) -> BTreeMap<String, Pin> {
                let mut pins = BTreeMap::new();
                for (name, text) in [
                    ("llama-server", "fixture server"),
                    ("libbase.so.1", "fixture library"),
                    ("LICENSE", "fixture license"),
                ] {
                    fs::write(self.source.join(name), text).unwrap();
                    pins.insert(
                        name.into(),
                        Pin::File {
                            hash: digest(text.as_bytes()),
                            bytes: None,
                            executable: name != "LICENSE",
                        },
                    );
                }
                symlink("libbase.so.1", self.source.join("libbase.so")).unwrap();
                pins.insert("libbase.so".into(), Pin::Link("libbase.so.1".into()));
                pins
            }
            fn clean(&self) {
                assert!(!self.install.join(".setup-lock").exists());
                for folder in ["models", "runtimes"] {
                    if let Ok(entries) = fs::read_dir(self.install.join(folder)) {
                        assert!(entries.flatten().all(|entry| {
                            !entry.file_name().to_string_lossy().starts_with(".import-")
                        }));
                    }
                }
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.base).unwrap();
            }
        }
        fn digest(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        }
        fn model_pin(bytes: &[u8]) -> BTreeMap<String, Pin> {
            BTreeMap::from([(
                MODEL_NAME.into(),
                Pin::File {
                    hash: digest(bytes),
                    bytes: Some(bytes.len() as u64),
                    executable: false,
                },
            )])
        }

        #[tokio::test]
        async fn imports_runtime_checks_stored_bytes_and_reruns_without_replacement() {
            let f = Fixture::new();
            let pins = f.runtime();
            let c = Cancellation::new();
            let result = install_inventory(&f.install, Asset::Cpu, &f.source, &c, &pins)
                .await
                .unwrap();
            assert_eq!(result.outcome, "installed");
            let installed = f.install.join("runtimes").join(Profile::Cpu.folder());
            assert_eq!(
                fs::read_link(installed.join("libbase.so")).unwrap(),
                Path::new("libbase.so.1")
            );
            assert_eq!(
                fs::metadata(installed.join("llama-server")).unwrap().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(installed.join("LICENSE")).unwrap().mode() & 0o777,
                0o600
            );
            let inode = fs::metadata(installed.join("llama-server")).unwrap().ino();
            assert_eq!(
                install_inventory(
                    &f.install,
                    Asset::Cpu,
                    &f.base.join("missing-source"),
                    &c,
                    &pins
                )
                .await
                .unwrap()
                .outcome,
                "already-installed"
            );
            assert_eq!(
                fs::metadata(installed.join("llama-server")).unwrap().ino(),
                inode
            );
            fs::write(installed.join("llama-server"), "tampered").unwrap();
            assert!(
                install_inventory(&f.install, Asset::Cpu, &f.source, &c, &pins)
                    .await
                    .unwrap_err()
                    .message
                    .contains("preserved")
            );
            assert_eq!(
                fs::read(installed.join("llama-server")).unwrap(),
                b"tampered"
            );
            f.clean();
        }
        #[tokio::test]
        async fn imports_model_independently_and_preserves_it_after_bad_runtime() {
            let f = Fixture::new();
            let data = b"synthetic model";
            let model = f.source.join("model.gguf");
            fs::write(&model, data).unwrap();
            let c = Cancellation::new();
            let model_pins = model_pin(data);
            let result = install_inventory(&f.install, Asset::Model, &model, &c, &model_pins)
                .await
                .unwrap();
            assert_eq!(result.bytes, data.len() as u64);
            assert_eq!(
                fs::metadata(&result.location).unwrap().mode() & 0o777,
                0o600
            );
            let pins = f.runtime();
            fs::write(f.source.join("libbase.so.1"), "changed").unwrap();
            assert!(
                install_inventory(&f.install, Asset::Cpu, &f.source, &c, &pins)
                    .await
                    .is_err()
            );
            assert_eq!(fs::read(&result.location).unwrap(), data);
            assert!(
                !f.install
                    .join("runtimes")
                    .join(Profile::Cpu.folder())
                    .exists()
            );
            f.clean();
        }
        #[tokio::test]
        async fn refuses_wrong_sizes_hashes_missing_libraries_and_redirected_links() {
            for kind in ["hash", "missing", "extra", "link", "file-link"] {
                let f = Fixture::new();
                let pins = f.runtime();
                match kind {
                    "hash" => fs::write(f.source.join("LICENSE"), "changed").unwrap(),
                    "missing" => fs::remove_file(f.source.join("libbase.so.1")).unwrap(),
                    "extra" => fs::write(f.source.join("librogue.so"), "unexpected").unwrap(),
                    "link" => {
                        fs::remove_file(f.source.join("libbase.so")).unwrap();
                        symlink("/outside", f.source.join("libbase.so")).unwrap();
                    }
                    _ => {
                        fs::rename(f.source.join("libbase.so.1"), f.source.join("real")).unwrap();
                        symlink("real", f.source.join("libbase.so.1")).unwrap();
                    }
                }
                assert!(
                    install_inventory(
                        &f.install,
                        Asset::Cpu,
                        &f.source,
                        &Cancellation::new(),
                        &pins
                    )
                    .await
                    .is_err(),
                    "{kind}"
                );
                assert!(
                    !f.install
                        .join("runtimes")
                        .join(Profile::Cpu.folder())
                        .exists()
                );
                f.clean();
            }
            let f = Fixture::new();
            let model = f.source.join("wrong.gguf");
            fs::write(&model, b"wrong length").unwrap();
            assert!(
                install_inventory(
                    &f.install,
                    Asset::Model,
                    &model,
                    &Cancellation::new(),
                    &model_pin(b"correct")
                )
                .await
                .is_err()
            );
            f.clean();
        }
        #[tokio::test]
        async fn rejects_symlinked_ancestors_destinations_and_cross_process_lock() {
            let f = Fixture::new();
            let pins = f.runtime();
            let c = Cancellation::new();
            let alias = f.base.join("alias");
            symlink(&f.source, &alias).unwrap();
            assert!(
                install_inventory(&f.install, Asset::Cpu, &alias, &c, &pins)
                    .await
                    .is_err()
            );
            f.clean();
            fs::write(f.install.join(".setup-lock"), "occupied").unwrap();
            assert!(
                install_inventory(&f.install, Asset::Cpu, &f.source, &c, &pins)
                    .await
                    .unwrap_err()
                    .message
                    .contains("lock")
            );
            assert_eq!(
                fs::read(f.install.join(".setup-lock")).unwrap(),
                b"occupied"
            );
            fs::remove_file(f.install.join(".setup-lock")).unwrap();
            fs::remove_dir(f.install.join("runtimes")).unwrap();
            symlink(&f.source, f.install.join("runtimes")).unwrap();
            assert!(
                install_inventory(&f.install, Asset::Cpu, &f.source, &c, &pins)
                    .await
                    .is_err()
            );
            assert_eq!(
                fs::read(f.source.join("llama-server")).unwrap(),
                b"fixture server"
            );
            f.clean();
            assert!(directory(&f.base.join("../escape"), true).is_err());
        }
        #[tokio::test]
        async fn cancels_streaming_import_cleans_staging_and_releases_reservation() {
            let f = Fixture::new();
            let data = vec![b'x'; 64 * 1024 * 1024];
            let path = f.source.join("model");
            fs::write(&path, &data).unwrap();
            let pins = model_pin(&data);
            let jobs = inference::InferenceJobs::default();
            let id = jobs.reserve().unwrap();
            let lease = jobs.claim(&id).unwrap();
            let c = lease.cancellation.clone();
            let (result, ()) = tokio::join!(
                install_inventory(&f.install, Asset::Model, &path, &c, &pins),
                async {
                    loop {
                        if fs::read_dir(f.install.join("models")).is_ok_and(|entries| {
                            entries
                                .flatten()
                                .any(|e| e.file_name().to_string_lossy().starts_with(".import-"))
                        }) {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    jobs.cancel(&id).unwrap();
                    assert!(jobs.reserve().is_err());
                }
            );
            assert!(result.unwrap_err().message.contains("cancelled"));
            assert!(!f.install.join("models").join(MODEL_NAME).exists());
            f.clean();
            drop(lease);
            assert!(jobs.reserve().is_ok());
        }
        #[tokio::test]
        async fn atomic_publication_preserves_racing_destination() {
            let f = Fixture::new();
            let data = vec![b'z'; 32 * 1024 * 1024];
            let path = f.source.join("model");
            fs::write(&path, &data).unwrap();
            let pins = model_pin(&data);
            let c = Cancellation::new();
            let target = f.install.join("models").join(MODEL_NAME);
            let (result, ()) = tokio::join!(
                install_inventory(&f.install, Asset::Model, &path, &c, &pins),
                async {
                    loop {
                        if fs::read_dir(f.install.join("models")).is_ok_and(|entries| {
                            entries
                                .flatten()
                                .any(|e| e.file_name().to_string_lossy().starts_with(".import-"))
                        }) {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    fs::write(&target, "racing writer").unwrap();
                }
            );
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("destination preserved")
            );
            assert_eq!(fs::read(&target).unwrap(), b"racing writer");
            f.clean();
        }
        #[tokio::test]
        async fn rejects_world_writable_installation_before_locking() {
            let f = Fixture::new();
            fs::create_dir(&f.install).unwrap();
            fs::set_permissions(&f.install, fs::Permissions::from_mode(0o777)).unwrap();
            assert!(
                install_at(&f.install, Asset::Cpu, &f.source, &Cancellation::new())
                    .await
                    .unwrap_err()
                    .message
                    .contains("deny group/other writes")
            );
            f.clean();
        }

        #[tokio::test]
        #[ignore = "copies 6 GB of installed selected artifacts; requires host loopback and Vulkan"]
        async fn real_main_model_offline_import_roundtrip() {
            use crate::{
                inference::NativeInference,
                onboarding::{
                    DesktopSourceSelection, prepare_onboarding_sources, publish_onboarding_plan,
                    review_onboarding_plan,
                },
                onboarding_generation::{group_input, remap},
            };
            use akasha_core::onboarding_local::LocalOnboardingPacket;
            use serde_json::json;
            let f = Fixture::new();
            let installed = inference::installation().unwrap();
            let evidence = PathBuf::from(
                std::env::var_os("AKASHA_SETUP_EVIDENCE")
                    .expect("set explicit synthetic evidence directory"),
            );
            fs::create_dir_all(&evidence).unwrap();
            let save = |name: &str, value: Value| {
                fs::write(
                    evidence.join(name),
                    serde_json::to_vec_pretty(&value).unwrap(),
                )
                .unwrap()
            };
            let started = std::time::Instant::now();
            let mut receipts = vec![];
            for asset in [Asset::Model, Asset::Cpu, Asset::Vulkan] {
                let (folder, name) = destination(asset);
                let source = installed.join(folder).join(name);
                let start = std::time::Instant::now();
                let result = install_at(&f.install, asset, &source, &Cancellation::new())
                    .await
                    .unwrap();
                assert_eq!(result.outcome, "installed");
                save(
                    &format!("import-{asset:?}.json"),
                    serde_json::to_value(&result).unwrap(),
                );
                let checked = verify_at(&f.install, asset, &Cancellation::new())
                    .await
                    .unwrap();
                assert_eq!(result.bytes, checked.bytes);
                let again = install_at(&f.install, asset, &source, &Cancellation::new())
                    .await
                    .unwrap();
                assert_eq!(again.outcome, "already-installed");
                receipts.push(json!({"asset":asset,"bytes":result.bytes,"import_verify_rerun_seconds":start.elapsed().as_secs_f64(),"verified":checked,"rerun":again}));
            }
            save(
                "installation.json",
                json!({"receipts":receipts,"seconds":started.elapsed().as_secs_f64()}),
            );
            fn copy(from: &Path, to: &Path) {
                fs::create_dir_all(to).unwrap();
                for entry in fs::read_dir(from).unwrap() {
                    let entry = entry.unwrap();
                    let dest = to.join(entry.file_name());
                    if entry.file_type().unwrap().is_dir() {
                        copy(&entry.path(), &dest);
                    } else {
                        fs::copy(entry.path(), dest).unwrap();
                    }
                }
            }
            fn snapshot(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
                fn walk(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
                    for entry in fs::read_dir(path).unwrap() {
                        let entry = entry.unwrap();
                        if entry.file_type().unwrap().is_dir() {
                            walk(root, &entry.path(), output);
                        } else {
                            output.insert(
                                entry.path().strip_prefix(root).unwrap().into(),
                                fs::read(entry.path()).unwrap(),
                            );
                        }
                    }
                }
                let mut output = BTreeMap::new();
                walk(path, path, &mut output);
                output
            }
            let root = f.base.join("valid-root");
            let repo = f.base.join("repository");
            copy(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../tests/fixtures/resolution/valid-root"),
                &root,
            );
            fs::create_dir(&repo).unwrap();
            fs::write(repo.join("limits.rs"), "// Queue settings for the synthetic offline-install acceptance.\npub const CAPACITY: usize = 37;\n").unwrap();
            let before = snapshot(&root);
            let code_before = snapshot(&repo);
            let sources = prepare_onboarding_sources(
                Some(root.clone()),
                "example".into(),
                DesktopSourceSelection {
                    mode: "files".into(),
                    paths: vec!["limits.rs".into()],
                    focus: vec![],
                    sample_large_files: false,
                },
            )
            .unwrap();
            save("sources.json", serde_json::to_value(&sources).unwrap());
            let packet: LocalOnboardingPacket = serde_json::from_str(&sources.packet_json).unwrap();
            let mut worker =
                NativeInference::open_at(&f.install, Profile::Vulkan, Cancellation::new())
                    .await
                    .unwrap();
            let mut notes = vec![];
            let mut metrics = vec![];
            for slot in &sources.identities.slots {
                let (input, schema) = group_input(&packet, slot);
                let result = worker
                    .complete(&input, &schema, include_str!("onboarding-policy.txt"))
                    .await;
                if result.is_err() {
                    worker.close().await.unwrap();
                }
                let (answer, metric) = result.unwrap();
                save(
                    "raw-completion.json",
                    json!({"answer":answer,"metrics":metric}),
                );
                notes.push(remap(&answer, slot, &packet).unwrap());
                metrics.push(metric);
            }
            worker.close().await.unwrap();
            let plan = json!({"schema_version":5,"notes":notes}).to_string();
            let generated = review_onboarding_plan(
                Some(root.clone()),
                "example".into(),
                &sources.packet_json,
                &plan,
            )
            .unwrap();
            save(
                "generated.json",
                json!({"plan_json":plan,"review":generated,"metrics":metrics}),
            );
            assert_eq!(snapshot(&root), before);
            assert_eq!(snapshot(&repo), code_before);
            let corrected = json!({"schema_version":5,"notes":[{"slot":sources.identities.slots[0].slot,"evidence":[{"kind":"fact","claim":"The queue capacity is 37 entries.","sources":[{"source":1,"excerpt":1}]}]}]});
            save("corrected-plan.json", corrected.clone());
            let review = review_onboarding_plan(
                Some(root.clone()),
                "example".into(),
                &sources.packet_json,
                &corrected.to_string(),
            )
            .unwrap();
            save("review.json", serde_json::to_value(&review).unwrap());
            assert!(
                publish_onboarding_plan(
                    Some(root.clone()),
                    "example".into(),
                    &sources.packet_json,
                    &corrected.to_string(),
                    "sha256:stale"
                )
                .is_err()
            );
            assert_eq!(snapshot(&root), before);
            let published = publish_onboarding_plan(
                Some(root.clone()),
                "example".into(),
                &sources.packet_json,
                &corrected.to_string(),
                &review.draft.preview.preview_id,
            )
            .unwrap();
            assert_eq!(published.created_notes.len(), 1);
            save("published.json", serde_json::to_value(&published).unwrap());
            let after = snapshot(&root);
            let found = crate::inspect_librarian_sources(
                Some(root.clone()),
                "example".into(),
                "queue capacity",
                true,
            )
            .unwrap();
            let passage = found
                .review
                .sources
                .matches
                .iter()
                .find(|p| p.quote.contains("queue capacity is 37"))
                .unwrap();
            let opened = crate::open_librarian_source(
                Some(root.clone()),
                "example".into(),
                &found.packet,
                passage.citation,
            )
            .unwrap();
            assert!(opened.document.source.contains("queue capacity is 37"));
            assert_eq!(passage.status.as_deref(), Some("draft"));
            save("librarian.json", serde_json::to_value(&found).unwrap());
            save(
                "opened.json",
                serde_json::to_value(&opened.document).unwrap(),
            );
            assert_eq!(snapshot(&root), after);
            assert_eq!(snapshot(&repo), code_before);
            f.clean();
            let temporary = f.base.clone();
            drop(f);
            assert!(!temporary.exists());
            save(
                "summary.json",
                json!({"passed":true,"real_model":true,"empty_destination_import":true,"both_runtimes_verified":true,"verified_noop_reruns":true,"imported_artifact_native_generation":true,"explicit_reviewed_publication":true,"checked_librarian_open":true,"repository_unchanged":true,"disposable_installation_removed":true,"seconds":started.elapsed().as_secs_f64()}),
            );
        }
    }
}
