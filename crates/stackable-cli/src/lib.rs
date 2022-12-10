#![deny(clippy::all)]
#![deny(missing_debug_implementations)]

mod cli;
mod indicators;
mod manifest;
mod utils;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use cargo_metadata::Metadata;
use clap::Parser;
use cli::{Cli, Command};
use console::{style, Term};
use futures::future::ready;
use futures::stream::unfold;
use futures::{pin_mut, FutureExt, Stream, StreamExt};
use manifest::Manifest;
use notify::{recommended_watcher, Event, RecursiveMode, Watcher};
use stackable_core::dev::StackctlMetadata;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::sleep;
use tokio::{fs, spawn};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::indicators::ServeProgress;
use crate::utils::random_str;

#[derive(Debug)]
struct Stackctl {
    cli: Arc<Cli>,
    manifest: Arc<Manifest>,
}

impl Stackctl {
    async fn workspace_dir(&self) -> Result<PathBuf> {
        self.cli
            .manifest_path
            .canonicalize()?
            .parent()
            .context("failed to find workspace directory")
            .map(|m| m.to_owned())
    }

    async fn watch_changes(&self) -> Result<impl Stream<Item = SystemTime>> {
        let workspace_dir = self.workspace_dir().await?;
        let (tx, rx) = unbounded_channel::<PathBuf>();

        let mut watcher = recommended_watcher(move |e: Result<Event, _>| {
            if let Ok(e) = e {
                for path in e.paths {
                    if tx.send(path).is_err() {
                        break;
                    }
                }
            }
        })
        .context("failed to watch workspace changes")?;

        watcher
            .watch(&workspace_dir, RecursiveMode::Recursive)
            .context("failed to watch workspace")?;

        let stream = UnboundedReceiverStream::new(rx)
            .filter(|p| {
                let p_str = p.as_os_str().to_string_lossy();
                if p_str.contains("target/") {
                    return ready(false);
                }
                if p_str.contains(".stackable/") {
                    return ready(false);
                }
                if !p_str.contains("src/") {
                    return ready(false);
                }

                ready(true)
            })
            .boxed();

        Ok(unfold(
            (stream, watcher),
            |(mut stream, watcher)| async move {
                // We wait until first item is available.
                stream.next().await?;

                let sleep_fur = sleep(Duration::from_millis(100)).fuse();
                pin_mut!(sleep_fur);

                // This makes sure we filter all items between first item and sleep completes,
                // whilst still returns at least 1 item at the end of the period.
                loop {
                    let next_path_fur = stream.next().fuse();
                    pin_mut!(next_path_fur);

                    futures::select! {
                        _ = sleep_fur => break,
                        _ = next_path_fur => {},
                    }
                }

                Some((SystemTime::now(), (stream, watcher)))
            },
        ))
    }

    fn is_release(&self) -> bool {
        match self.cli.command {
            Command::Serve { .. } => false,
            Command::Build { .. } => true,
        }
    }

    /// Creates and returns the path of the data directory.
    ///
    /// This is `build` directory in the same parent directory as `stackable.toml`.
    async fn build_dir(&self) -> Result<PathBuf> {
        let data_dir = self.workspace_dir().await?.join("build");

        fs::create_dir_all(&data_dir)
            .await
            .context("failed to create build directory")?;

        Ok(data_dir)
    }

    /// Creates and returns the path of the data directory.
    ///
    /// This is `.stackable` directory in the same parent directory as `stackable.toml`.
    async fn data_dir(&self) -> Result<PathBuf> {
        let data_dir = self.workspace_dir().await?.join(".stackable");

        fs::create_dir_all(&data_dir)
            .await
            .context("failed to create data directory")?;

        Ok(data_dir)
    }

    async fn frontend_data_dir(&self) -> Result<PathBuf> {
        let frontend_data_dir = self.data_dir().await?.join("frontend");

        fs::create_dir_all(&frontend_data_dir)
            .await
            .context("failed to create frontend data directory")?;

        Ok(frontend_data_dir)
    }

    async fn backend_data_dir(&self) -> Result<PathBuf> {
        let backend_data_dir = self.data_dir().await?.join("backend");

        fs::create_dir_all(&backend_data_dir)
            .await
            .context("failed to create backend data directory")?;

        Ok(backend_data_dir)
    }

    async fn frontend_build_dir(&self) -> Result<PathBuf> {
        let frontend_build_dir = match self.cli.command {
            Command::Build { .. } => {
                let build_dir = self.build_dir().await?;
                build_dir.join("frontend")
            }
            Command::Serve { .. } => {
                let frontend_data_dir = self.frontend_data_dir().await?;
                frontend_data_dir.join("dev-builds").join(random_str()?)
            }
        };

        fs::create_dir_all(&frontend_build_dir)
            .await
            .context("failed to create build directory for frontend build.")?;

        Ok(frontend_build_dir)
    }

    async fn backend_build_dir(&self) -> Result<PathBuf> {
        let frontend_build_dir = match self.cli.command {
            Command::Build { .. } => {
                let build_dir = self.build_dir().await?;
                build_dir.join("backend")
            }
            Command::Serve { .. } => {
                let frontend_data_dir = self.backend_data_dir().await?;
                frontend_data_dir.join("dev-builds").join(random_str()?)
            }
        };

        fs::create_dir_all(&frontend_build_dir)
            .await
            .context("failed to create build directory for backend build.")?;

        Ok(frontend_build_dir)
    }

    async fn transfer_to_file<R, P>(source: R, target: P) -> Result<()>
    where
        R: 'static + AsyncRead + Send,
        P: Into<PathBuf>,
    {
        let target_path = target.into();
        let mut target = fs::File::create(&target_path)
            .await
            .with_context(|| format!("failed to create {}", target_path.display()))?;

        let inner = async move {
            tokio::pin!(source);

            loop {
                let mut buf = [0_u8; 8192];
                let buf_len = source.read(&mut buf[..]).await?;

                if buf_len == 0 {
                    break;
                }
                target.write_all(&buf[..buf_len]).await?;
            }

            Ok::<(), anyhow::Error>(())
        };

        spawn(async move {
            if let Err(e) = inner
                .await
                .with_context(|| format!("failed to transfer logs to: {}", target_path.display()))
            {
                tracing::error!("{:#?}", e);
            }
        });

        Ok(())
    }

    async fn build_frontend(&self) -> Result<PathBuf> {
        use tokio::process::Command;

        let frontend_data_dir = self.frontend_data_dir().await?;
        let frontend_build_dir = self.frontend_build_dir().await?;
        let workspace_dir = self.workspace_dir().await?;

        let create_proc = || {
            let mut proc = Command::new("trunk");
            proc.arg("build")
                .arg("--dist")
                .arg(&frontend_build_dir)
                .arg(workspace_dir.join("index.html"))
                .current_dir(&workspace_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            if self.is_release() {
                proc.arg("--release")
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
            }
            proc
        };

        let mut child = create_proc().spawn()?;

        if let Some(m) = child.stdout.take() {
            Self::transfer_to_file(
                m,
                frontend_data_dir.join(format!("log-stdout-{}", random_str()?)),
            )
            .await?;
        }

        if let Some(m) = child.stderr.take() {
            Self::transfer_to_file(
                m,
                frontend_data_dir.join(format!("log-stderr-{}", random_str()?)),
            )
            .await?;
        }

        let status = child.wait().await?;

        // We try again with logs printed to console.
        if !status.success() {
            if self.is_release() {
                bail!("trunk failed with status {}", status);
            }

            let mut proc = create_proc();
            proc.stdout(Stdio::inherit()).stderr(Stdio::inherit());

            let mut child = proc.spawn()?;
            let status = child.wait().await?;

            if !status.success() {
                bail!("trunk failed with status {}", status);
            }
        }

        Ok(frontend_build_dir)
    }

    async fn build_backend<P>(&self, frontend_build_dir: P) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        use tokio::process::Command;

        let frontend_build_dir = frontend_build_dir.as_ref();

        let backend_data_dir = self.backend_data_dir().await?;
        let workspace_dir = self.workspace_dir().await?;
        let backend_build_dir = self.backend_build_dir().await?;

        let create_proc = || {
            let mut proc = Command::new("cargo");
            proc.arg("build")
                .arg("--bin")
                .arg(&self.manifest.dev_server.bin_name)
                .current_dir(&workspace_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env("STACKABLE_FRONTEND_BUILD_DIR", frontend_build_dir)
                .kill_on_drop(true);

            if self.is_release() {
                proc.arg("--release")
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
            }

            proc
        };

        let mut child = create_proc().spawn()?;

        if let Some(m) = child.stdout.take() {
            Self::transfer_to_file(
                m,
                backend_data_dir.join(format!("log-stdout-{}", random_str()?)),
            )
            .await?;
        }

        if let Some(m) = child.stderr.take() {
            Self::transfer_to_file(
                m,
                backend_data_dir.join(format!("log-stderr-{}", random_str()?)),
            )
            .await?;
        }

        let status = child.wait().await?;

        // We try again with logs printed to console.
        if !status.success() {
            if self.is_release() {
                bail!("trunk failed with status {}", status);
            }

            let mut proc = create_proc();
            proc.stdout(Stdio::inherit()).stderr(Stdio::inherit());

            let mut child = proc.spawn()?;
            let status = child.wait().await?;

            if !status.success() {
                bail!("trunk failed with status {}", status);
            }
        }

        // Copy artifact from target directory.
        let pkg_meta_output = Command::new("cargo")
            .arg("metadata")
            .arg("--format-version=1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&workspace_dir)
            .spawn()?
            .wait_with_output()
            .await
            .context("failed to read package metadata")?;

        if !pkg_meta_output.status.success() {
            bail!(
                "cargo metadata failed with status {}",
                pkg_meta_output.status
            );
        }

        let meta: Metadata = serde_json::from_slice(&pkg_meta_output.stdout)
            .context("failed to parse package metadata")?;

        let bin_path = meta
            .target_directory
            .join_os("release")
            .join(&self.manifest.dev_server.bin_name);

        let backend_bin_path = backend_build_dir.join(&self.manifest.dev_server.bin_name);

        fs::copy(bin_path, &backend_bin_path)
            .await
            .context("failed to copy binary")?;

        Ok(backend_bin_path)
    }

    async fn open_browser(&self, http_listen_addr: &str) -> Result<()> {
        use tokio::process::Command;
        let workspace_dir = self.workspace_dir().await?;

        Command::new("open")
            .arg(http_listen_addr)
            .current_dir(&workspace_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?
            .wait()
            .await
            .context("failed to open url")?;

        Ok(())
    }

    async fn serve_once(&self) -> Result<Child> {
        use tokio::process::Command;

        let http_listen_addr = format!("http://{}/", self.manifest.dev_server.listen);

        let bar = ServeProgress::new();

        let workspace_dir = self.workspace_dir().await?;
        bar.step_build_frontend();
        let frontend_build_dir = self.build_frontend().await?;

        bar.step_build_backend();
        self.build_backend(&frontend_build_dir).await?;

        let meta = StackctlMetadata {
            listen_addr: self.manifest.dev_server.listen.to_string(),
            frontend_dev_build_dir: frontend_build_dir.clone(),
        };

        bar.step_starting();

        let server_proc = Command::new("cargo")
            .arg("run")
            .arg("--quiet")
            .arg("--bin")
            .arg(&self.manifest.dev_server.bin_name)
            .current_dir(&workspace_dir)
            .env(StackctlMetadata::ENV_NAME, meta.to_json()?)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;

        while reqwest::ClientBuilder::default()
            .timeout(Duration::from_secs(1))
            .build()?
            .get(&http_listen_addr)
            .send()
            .await
            .and_then(|m| m.error_for_status())
            .is_err()
        {
            sleep(Duration::from_secs(1)).await;
        }

        bar.hide();

        Ok(server_proc)
    }

    async fn run_serve(&self, open: bool) -> Result<()> {
        let changes = self.watch_changes().await?;
        pin_mut!(changes);

        let mut first_run = true;

        'outer: loop {
            let start_time = SystemTime::now();
            let http_listen_addr = format!("http://{}/", self.manifest.dev_server.listen);

            let server_proc = match self.serve_once().await {
                Ok(server_proc) => {
                    let time_taken_in_f64 =
                        f64::try_from(i32::try_from(start_time.elapsed()?.as_millis())?)? / 1000.0;

                    Term::stderr().clear_screen()?;

                    eprintln!(
                        "{}",
                        style(format!("Built in {:.2}s!", time_taken_in_f64))
                            .green()
                            .bold()
                    );
                    eprintln!("Stackable development server has started!");
                    eprintln!();
                    eprintln!();
                    eprintln!("    Listen: {}", http_listen_addr);
                    eprintln!();
                    eprintln!();
                    eprintln!(
                        "To produce a production build, you can use `{}`",
                        style("stackctl build --release").cyan().bold()
                    );

                    Some(server_proc)
                }
                Err(e) => {
                    tracing::error!("failed to build development server: {:?}", e);
                    None
                }
            };

            if open && first_run {
                self.open_browser(&http_listen_addr).await?;
            }

            first_run = false;

            'inner: loop {
                match changes.next().await {
                    Some(change_time) => {
                        if change_time > start_time {
                            break 'inner;
                        }
                    }
                    None => break 'outer,
                }
            }

            if let Some(mut m) = server_proc {
                m.kill().await.context("failed to stop server")?;
            }
        }

        Ok(())
    }

    async fn run_build(&self, release: bool) -> Result<()> {
        if !release {
            bail!("building distributable in debug mode is not yet supported!");
        }

        eprintln!(
            "{}",
            style("Building Release Distribution...").cyan().bold()
        );

        let start_time = SystemTime::now();

        let frontend_build_dir = self.build_frontend().await?;
        let backend_build_path = self.build_backend(&frontend_build_dir).await?;

        let time_taken_in_f64 =
            f64::try_from(i32::try_from(start_time.elapsed()?.as_millis())?)? / 1000.0;
        eprintln!(
            "{}",
            style(format!("Built in {:.2}s!", time_taken_in_f64))
                .green()
                .bold()
        );
        eprintln!(
            "The server binary is available at: {}",
            backend_build_path.display()
        );

        Ok(())
    }

    async fn run(&self) -> Result<()> {
        match self.cli.command {
            Command::Serve { open } => {
                self.run_serve(open).await?;
            }
            Command::Build { release } => {
                self.run_build(release).await?;
            }
        }

        Ok(())
    }
}

pub async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().pretty())
        .with(
            EnvFilter::builder()
                .with_default_directive(Level::INFO.into())
                .with_env_var("STACKCTL_LOG")
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();
    let manifest = cli.load_manifest().await?;

    Stackctl {
        cli: cli.into(),
        manifest,
    }
    .run()
    .await?;

    Ok(())
}
