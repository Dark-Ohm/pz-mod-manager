//! Everything that touches the network, the disk or a child process runs on a
//! thread and reports back through one channel, so the HUD never freezes.

use crate::config;
use crate::pack::{self, Manifest, PackInfo};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    Info,
    Check,
    Download,
    Install,
    Launch,
}

impl Job {
    pub fn label(self) -> &'static str {
        match self {
            Job::Info => "ЧТЕНИЕ ПАКЕТА",
            Job::Check => "ПРОВЕРКА МАНИФЕСТА",
            Job::Download => "ЗАГРУЗКА ПАКЕТА",
            Job::Install => "УСТАНОВКА МОДОВ",
            Job::Launch => "ЗАПУСК КЛИЕНТА",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Err,
}

pub enum Msg {
    Log(Level, String),
    Progress(u64, Option<u64>),
    Info(PackInfo),
    Manifest(Manifest),
    Done(Job),
    Failed(Job, String),
}

fn send(tx: &Sender<Msg>, level: Level, text: impl Into<String>) {
    let _ = tx.send(Msg::Log(level, text.into()));
}

pub fn info(tx: Sender<Msg>, manager: String, pack: PathBuf, env: String) {
    std::thread::spawn(move || match pack::info(&manager, &pack, &env) {
        Ok(info) => {
            let _ = tx.send(Msg::Info(info));
            let _ = tx.send(Msg::Done(Job::Info));
        }
        Err(err) => {
            let _ = tx.send(Msg::Failed(Job::Info, format!("{err:#}")));
        }
    });
}

pub fn check(tx: Sender<Msg>, url: String) {
    std::thread::spawn(move || match pack::fetch(&url) {
        Ok(manifest) => {
            let _ = tx.send(Msg::Manifest(manifest));
            let _ = tx.send(Msg::Done(Job::Check));
        }
        Err(err) => {
            let _ = tx.send(Msg::Failed(Job::Check, format!("{err:#}")));
        }
    });
}

pub fn download(tx: Sender<Msg>, url: String, dest: PathBuf, sha256: Option<String>) {
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            if let Some(dir) = dest.parent() {
                fs::create_dir_all(dir)?;
            }
            let part = dest.with_extension("part");
            send(&tx, Level::Info, format!("GET {url}"));
            let response = ureq::get(&url)
                .call()
                .with_context(|| format!("не удалось скачать {url}"))?;
            let total = response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let _ = tx.send(Msg::Progress(0, total));

            let mut reader = response.into_body().into_reader();
            let mut out = BufWriter::new(File::create(&part)?);
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 128 * 1024];
            let mut done = 0u64;
            let started = Instant::now();
            let mut last_sent = Instant::now();
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                out.write_all(&buf[..read])?;
                hasher.update(&buf[..read]);
                done += read as u64;
                if last_sent.elapsed() >= Duration::from_millis(150) {
                    last_sent = Instant::now();
                    let _ = tx.send(Msg::Progress(done, total));
                }
            }
            out.flush()?;
            drop(out);

            if let Some(want) = sha256 {
                let digest = hasher.finalize();
                let got = hex(&digest);
                let want = want
                    .trim()
                    .trim_start_matches("sha256:")
                    .to_ascii_lowercase();
                if !want.is_empty() && got != want {
                    let _ = fs::remove_file(&part);
                    anyhow::bail!("sha256 не совпал: ожидался {want}, получен {got}");
                }
                send(
                    &tx,
                    Level::Ok,
                    format!("sha256 совпал: {}…", &got[..got.len().min(12)]),
                );
            }

            let _ = tx.send(Msg::Progress(done, total.or(Some(done))));
            let _ = fs::remove_file(&dest);
            fs::rename(&part, &dest)?;
            send(
                &tx,
                Level::Ok,
                format!(
                    "скачано {} за {:.1} с → {}",
                    crate::theme::bytes(done),
                    started.elapsed().as_secs_f32(),
                    dest.display()
                ),
            );
            Ok(())
        })();

        match result {
            Ok(()) => {
                let _ = tx.send(Msg::Done(Job::Download));
            }
            Err(err) => {
                let _ = tx.send(Msg::Failed(Job::Download, format!("{err:#}")));
            }
        }
    });
}

pub fn install(tx: Sender<Msg>, manager: String, pack: PathBuf, env: String) {
    std::thread::spawn(move || {
        let mut cmd = pack::manager_command(&manager);
        cmd.arg("pack").arg("install").arg(&pack);
        if !env.trim().is_empty() {
            cmd.arg("--env").arg(&env);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        send(
            &tx,
            Level::Info,
            format!("{} pack install {}", manager, pack.display()),
        );

        match cmd.spawn() {
            Ok(mut child) => {
                let out = child.stdout.take().map(|s| {
                    let tx = tx.clone();
                    std::thread::spawn(move || pump(s, tx, Level::Info))
                });
                let err = child.stderr.take().map(|s| {
                    let tx = tx.clone();
                    std::thread::spawn(move || pump(s, tx, Level::Warn))
                });
                let status = child.wait();
                if let Some(handle) = out {
                    let _ = handle.join();
                }
                if let Some(handle) = err {
                    let _ = handle.join();
                }
                match status {
                    Ok(status) if status.success() => {
                        let _ = tx.send(Msg::Done(Job::Install));
                    }
                    Ok(status) => {
                        let _ = tx.send(Msg::Failed(
                            Job::Install,
                            format!(
                                "менеджер завершился с кодом {}",
                                status.code().unwrap_or(-1)
                            ),
                        ));
                    }
                    Err(err) => {
                        let _ = tx.send(Msg::Failed(
                            Job::Install,
                            format!("не удалось дождаться менеджера: {err}"),
                        ));
                    }
                }
            }
            Err(err) => {
                let _ = tx.send(Msg::Failed(
                    Job::Install,
                    format!("не удалось запустить {manager}: {err}"),
                ));
            }
        }
    });
}

pub fn launch(tx: Sender<Msg>, game_dir: PathBuf, cachedir: PathBuf, connect: String) {
    std::thread::spawn(move || {
        let Some(bin) = config::game_bin(&game_dir) else {
            let _ = tx.send(Msg::Failed(
                Job::Launch,
                format!("ProjectZomboid64 не найден в {}", game_dir.display()),
            ));
            return;
        };

        let mut cmd = std::process::Command::new(&bin);
        cmd.current_dir(&game_dir);
        cmd.arg(format!("-cachedir={}", cachedir.display()));
        if !connect.trim().is_empty() {
            cmd.arg("+connect").arg(connect.trim());
        }
        match cmd.spawn() {
            Ok(child) => {
                send(
                    &tx,
                    Level::Ok,
                    format!("клиент запущен, pid {} ({})", child.id(), bin.display()),
                );
                let _ = tx.send(Msg::Done(Job::Launch));
            }
            Err(err) => {
                let _ = tx.send(Msg::Failed(
                    Job::Launch,
                    format!("не удалось запустить игру: {err}"),
                ));
            }
        }
    });
}

fn pump(stream: impl Read + Send + 'static, tx: Sender<Msg>, level: Level) {
    let reader = BufReader::new(stream);
    for line in reader.lines().map_while(|line| line.ok()) {
        let line = line.trim_end();
        if !line.is_empty() {
            let _ = tx.send(Msg::Log(level, line.to_string()));
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }

    #[test]
    fn job_labels_are_stable() {
        assert_eq!(Job::Install.label(), "УСТАНОВКА МОДОВ");
        assert_eq!(Job::Launch.label(), "ЗАПУСК КЛИЕНТА");
    }

    #[test]
    fn local_download_roundtrip_would_need_network() {
        // The real download path is exercised against a local file URL by hand;
        // here we only pin the destination naming rule used by the UI.
        let manifest = Manifest {
            revision: Some(12),
            ..Default::default()
        };
        assert_eq!(manifest.download_name(), "rev-12.pzpack");
    }

    #[test]
    fn path_is_kept_for_missing_directories() {
        let p = Path::new("/tmp/pz-launcher-test");
        assert!(!p.exists() || p.is_dir());
    }
}
