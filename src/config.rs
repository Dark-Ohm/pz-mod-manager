use crate::i18n::t;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Один сервер (у Linux и Windows сборок они отдельные)
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ServerCfg {
    /// Короткий id окружения для CLI/агентов (stable, lab…). Пусто — берётся из названия
    pub id: String,
    /// Название в списке, например «Windows pack»
    pub label: String,
    /// Папка данных сервера (-cachedir): внутри Server/*.ini, mods/, Saves/
    pub cache_dir: PathBuf,
    /// Папка данных клиента этого окружения (-cachedir клиента). Пусто — общие «Моды клиента» из настроек
    pub client_cache_dir: PathBuf,
    /// Имя сервера = имя ini без расширения
    pub name: String,
    /// Папка с игрой/сервером (там start-server.sh или StartServer64_nosteam.bat)
    pub game_dir: PathBuf,
    /// Запуск без Steam (-nosteam): воркшоп недоступен, моды только локальные
    pub nosteam: bool,
    /// Доп. аргументы запуска
    pub extra_args: String,
    /// Воркшоп-папка сервера (только для режима со Steam)
    pub workshop_dir: PathBuf,
    /// Память Java (МБ). Используется при запуске на Windows; на Linux задаётся в ProjectZomboid64.json
    pub xmx_mb: u32,
    /// Сколько бэкапов хранить
    pub backup_keep: usize,
    /// Фоновая картинка карты (необязательно) и её границы в клетках мира: x0, y0, x1, y1
    pub map_image: PathBuf,
    pub map_bounds: [i32; 4],
    pub markers: Vec<Marker>,
    /// Адрес, по которому подключаются игроки (IP или доменное имя). В ini не пишется, идёт в файл сборки
    pub public_host: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Marker {
    pub name: String,
    pub x: i32,
    pub y: i32,
}

impl Default for ServerCfg {
    fn default() -> Self {
        let home = crate::platform::home();
        Self {
            // фиксированный id: не зависит от языка интерфейса (агенты ссылаются на него)
            id: "default".into(),
            label: if cfg!(windows) { t("Windows сервер").into() } else { t("Linux сервер").into() },
            cache_dir: crate::platform::zomboid_user_dir(),
            client_cache_dir: PathBuf::new(),
            name: "servertest".into(),
            game_dir: crate::platform::default_game_dir(),
            nosteam: true,
            extra_args: String::new(),
            workshop_dir: home.join("pzserver/steamapps/workshop/content/108600"),
            xmx_mb: 4096,
            backup_keep: 10,
            map_image: PathBuf::new(),
            map_bounds: [0, 0, 16800, 16800],
            markers: vec![],
            public_host: String::new(),
        }
    }
}

impl ServerCfg {
    /// Ключ окружения: id, иначе название в нижнем регистре без пробелов
    pub fn key(&self) -> String {
        let src = if self.id.trim().is_empty() { &self.label } else { &self.id };
        src.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
    }
    /// -cachedir клиента этого окружения
    pub fn client_cache_dir(&self, s: &Settings) -> PathBuf {
        if !self.client_cache_dir.as_os_str().is_empty() {
            return self.client_cache_dir.clone();
        }
        s.client_mods_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(crate::platform::zomboid_user_dir)
    }
    /// Локальные моды клиента этого окружения: <client cachedir>/mods, иначе общая папка из настроек
    pub fn client_mods_dir(&self, s: &Settings) -> PathBuf {
        if !self.client_cache_dir.as_os_str().is_empty() {
            return self.client_cache_dir.join("mods");
        }
        s.client_mods_dir.clone()
    }
    /// Локальные моды сервера: <cachedir>/mods
    pub fn mods_dir(&self) -> PathBuf {
        self.cache_dir.join("mods")
    }
    /// <cachedir>/Server/<имя>.ini
    pub fn ini(&self) -> PathBuf {
        self.cache_dir.join("Server").join(format!("{}.ini", self.name))
    }
    /// Имена серверов, найденные в <cachedir>/Server (только *.ini, без бэкапов)
    pub fn list_names(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.cache_dir.join("Server"))
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".ini").map(str::to_string))
            .collect();
        v.sort();
        v
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Локальные моды клиента (~/Zomboid/mods)
    pub client_mods_dir: PathBuf,
    /// Воркшоп клиента из Steam
    pub client_workshop_dir: PathBuf,
    /// Папка установленного клиента игры (одна на все окружения)
    pub client_game_dir: PathBuf,
    pub servers: Vec<ServerCfg>,
    pub active_server: usize,
    /// Steam Web API key (для подбора привязок по названию). Хранится в config.toml открытым текстом
    pub steam_api_key: String,
    /// "" — язык системы, "ru" или "en"
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            client_mods_dir: crate::platform::zomboid_user_dir().join("mods"),
            client_workshop_dir: crate::platform::client_workshop_dir(),
            client_game_dir: crate::platform::default_client_dir(),
            servers: vec![ServerCfg::default()],
            active_server: 0,
            steam_api_key: String::new(),
            language: String::new(),
        }
    }
}

impl Settings {
    pub fn server(&self) -> &ServerCfg {
        &self.servers[self.active_server.min(self.servers.len() - 1)]
    }
    pub fn server_mut(&mut self) -> &mut ServerCfg {
        let i = self.active_server.min(self.servers.len() - 1);
        &mut self.servers[i]
    }
    /// Окружение по id, названию (без учёта регистра) или номеру
    pub fn find_env(&self, q: &str) -> Option<usize> {
        let q = q.trim();
        let ql = q.to_lowercase();
        self.servers
            .iter()
            .position(|c| c.key() == ql || c.id.eq_ignore_ascii_case(q) || c.label.to_lowercase() == ql)
            .or_else(|| q.parse::<usize>().ok().filter(|i| *i < self.servers.len()))
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Profile {
    pub name: String,
    /// id модов в порядке загрузки
    pub enabled: Vec<String>,
    /// заметки о сборке
    #[serde(default)]
    pub notes: String,
    /// помечена как стабильная
    #[serde(default)]
    pub stable: bool,
    /// постоянный id сборки: по нему программа узнаёт ту же сборку в новом файле
    #[serde(default)]
    pub pack_id: String,
    /// номер ревизии; растёт, когда состав, порядок, название или адрес сервера меняются
    #[serde(default)]
    pub revision: u64,
    /// автор: файл .pzpack, который перезаписывается при каждом изменении сборки
    #[serde(default)]
    pub sync_path: String,
    /// автор: окружение, чьи моды и адрес сервера идут в файл
    #[serde(default)]
    pub sync_env: String,
    /// хеш содержимого, с которым записана текущая ревизия
    #[serde(default)]
    pub last_hash: String,
    /// получатель: файл .pzpack, за которым следим (его кладёт автор)
    #[serde(default)]
    pub source_path: String,
    /// получатель: какая ревизия источника уже применена
    #[serde(default)]
    pub applied_revision: u64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Store {
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    pub active: usize,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            profiles: vec![Profile { name: t("Основной").into(), enabled: vec![], ..Default::default() }],
            active: 0,
        }
    }
}

impl Store {
    pub fn profile(&self) -> &Profile {
        &self.profiles[self.active.min(self.profiles.len() - 1)]
    }
    pub fn profile_mut(&mut self) -> &mut Profile {
        let i = self.active.min(self.profiles.len() - 1);
        &mut self.profiles[i]
    }

    pub fn path() -> PathBuf {
        config_root().join("config.toml")
    }

    /// Время изменения config.toml (GUI перечитывает конфиг, если его поменял CLI/агент)
    pub fn mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::path()).and_then(|m| m.modified()).ok()
    }

    pub fn load() -> Self {
        let s: Store = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default();
        s.fixed()
    }

    /// Как load, но битый config.toml — ошибка, а не тихие настройки по умолчанию
    /// (иначе следующее сохранение затёрло бы файл пользователя)
    pub fn load_strict() -> anyhow::Result<Self> {
        let p = Self::path();
        let s: Store = match std::fs::read_to_string(&p) {
            Ok(text) => toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(e) => return Err(anyhow::anyhow!("{}: {e}", p.display())),
        };
        Ok(s.fixed())
    }

    /// Блокировка config.toml на время чтения-изменения-записи (CLI и GUI работают параллельно)
    pub fn lock() -> anyhow::Result<std::fs::File> {
        std::fs::create_dir_all(config_root())?;
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(config_root().join("config.lock"))?;
        f.lock()?;
        Ok(f)
    }

    fn fixed(mut self) -> Self {
        let s = &mut self;
        if s.settings.servers.is_empty() {
            s.settings.servers = vec![ServerCfg::default()];
        }
        if s.profiles.is_empty() {
            s.profiles = Store::default().profiles;
        }
        self
    }

    pub fn save(&self) {
        let _ = self.try_save();
    }

    /// Запись через временный файл и rename: читатель никогда не видит половину файла
    pub fn try_save(&self) -> anyhow::Result<()> {
        let p = Self::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let t = toml::to_string_pretty(self)?;
        let tmp = p.with_file_name(format!(".config.toml.tmp-{}", std::process::id()));
        std::fs::write(&tmp, t)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }
}

/// Рабочая папка менеджера: SteamCMD и кэш загрузок
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("PZMM_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    // Windows: %LOCALAPPDATA% (кэш может быть большим и не должен ездить с профилем)
    dirs::data_local_dir().unwrap_or_default().join("pz-mod-manager")
}

/// Папка настроек менеджера. В тестах — временная, чтобы `cargo test` не трогал настоящие настройки.
pub fn config_root() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("pzmm-test-config-{}", std::process::id()));
    }
    if let Some(d) = std::env::var_os("PZMM_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    dirs::config_dir().unwrap_or_default().join("pz-mod-manager")
}
