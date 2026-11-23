// Записывает коммит сборки: по нему кнопка «Обновить» узнаёт, есть ли новая версия.
fn main() {
    let sha = std::env::var("GITHUB_SHA").ok().filter(|s| !s.is_empty()).or_else(|| {
        let o = std::process::Command::new("git").args(["rev-parse", "HEAD"]).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    });
    println!("cargo:rustc-env=PZ_COMMIT={}", sha.unwrap_or_else(|| "unknown".into()));
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
}
