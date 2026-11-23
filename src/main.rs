// В релизной сборке под Windows не открывать консольное окно
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
use i18n::t;
mod i18n_en;
mod atomic;
mod selfupdate;
mod backup;
mod commands;
mod config;
mod export;
mod ini;
mod map;
mod matching;
mod mods;
mod platform;
mod players;
mod rcon;
mod runtime;
mod sandbox;
mod server;
mod sort;
mod steam;
mod tab_backup;
mod tab_map;
mod packs;
mod share;
mod tab_packs;
mod tab_players;
mod tab_share;
mod tab_updates;
mod tab_sandbox;
mod templates;
mod tracking;
mod modupdate;
mod workshop_api;
mod cli;
mod mcp;
#[cfg(test)]
mod tests_extra;
mod validate;

use config::Store;
use eframe::egui::{self, Color32, RichText};
use mods::{Mod, Severity, Source};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, channel};

const ACCENT: Color32 = Color32::from_rgb(120, 170, 90);
const WARN: Color32 = Color32::from_rgb(230, 180, 70);
const ERR: Color32 = Color32::from_rgb(225, 95, 90);
const CARD: Color32 = Color32::from_rgb(30, 33, 30);
const GREEN_BTN: Color32 = Color32::from_rgb(70, 120, 55);
const DIM: Color32 = Color32::from_gray(140);

/// Карточка на всю ширину
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(8)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Плитка с цифрой: подпись, значение, пояснение
fn tile(ui: &mut egui::Ui, title: &str, value: &str, sub: &str, sub_color: Color32) {
    card(ui, |ui| {
        ui.label(RichText::new(title).size(12.0).color(DIM));
        ui.label(RichText::new(value).size(20.0).strong());
        ui.label(RichText::new(sub).size(12.0).color(sub_color));
    });
}

/// «2 ч 14 мин», «5 мин», «40 с»
fn dur_text(secs: u64) -> String {
    if secs >= 3600 {
        tf!("{} ч {} мин", secs / 3600, secs % 3600 / 60)
    } else if secs >= 60 {
        tf!("{} мин", secs / 60)
    } else {
        tf!("{} с", secs)
    }
}

/// Сколько прошло с бэкапа по его имени `ГГГГММДД-ЧЧММСС-метка`
fn backup_age(name: &str) -> Option<u64> {
    let ts = chrono::NaiveDateTime::parse_from_str(name.get(..15)?, "%Y%m%d-%H%M%S").ok()?;
    let then = ts.and_local_timezone(chrono::Local).single()?;
    u64::try_from((chrono::Local::now() - then).num_seconds()).ok()
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Mods,
    Download,
    Server,
    Sandbox,
    Backup,
    Players,
    Map,
    Updates,
    Packs,
    Settings,
}

/// Что запустить из меню «▾»
enum Launch {
    /// клиент через Steam (как кнопка «Играть»)
    Steam,
    /// клиент с обычной папкой игры, -nosteam
    PlainClient,
    /// клиент окружения: (индекс, -nosteam)
    Client(usize, bool),
    /// сервер окружения: (индекс, -nosteam)
    Server(usize, bool),
}

enum Act {
    Toggle(String),
    Up(usize),
    Down(usize),
}

/// Сообщения из фоновых потоков
pub enum Bg {
    /// вывод команды
    Cmd(String),
    /// итог задачи (бэкап/восстановление)
    Job(String),
    /// даты из Steam
    Remote(Result<Vec<workshop_api::Remote>, String>),
    Changelog(String, Result<Vec<workshop_api::Change>, String>),
    Log(String),
    MatchProgress(usize, usize),
    Matches(Result<Vec<matching::Proposal>, String>),
    /// .pzpack записан (Some — показать в статусе)
    Shared(Option<String>),
    /// сборка установлена на клиент: пересканировать моды
    PackInstalled(String),
    Updated { ws: String, ok: bool, msg: String, baseline: Option<u64> },
    /// проверка новой версии программы
    SelfCheck(Result<selfupdate::Info, String>),
    /// новая версия установлена (Ok) или нет
    SelfApplied(Result<(), String>),
}

pub struct App {
    pub store: Store,
    pub installed: HashMap<String, Mod>,
    pub dups: HashSet<String>,
    pub cache: Vec<Mod>,
    pub tab: Tab,
    pub filter: String,
    pub selected: Option<String>,
    pub new_profile: String,
    pub dl_input: String,
    pub dl_log: Vec<String>,
    pub dl_rx: Option<Receiver<steam::Msg>>,
    /// окружение, куда ставить скачанные моды (вкладка «Загрузка»)
    pub dl_env: usize,
    pub status: String,
    pub drag: Option<usize>,
    pub last_rects: Vec<(usize, egui::Rect)>,
    /// процессы, запущенные из GUI: держим, чтобы забирать их статус (не оставлять зомби)
    pub children: Vec<std::process::Child>,
    /// состояние сервера/клиента активного окружения (общее с CLI/агентами, см. runtime.rs)
    pub srv_status: Option<runtime::Status>,
    pub cli_status: Option<runtime::Status>,
    /// (сервер, клиент) каждого окружения — для шапки и меню запуска; обновляется раз в секунду
    pub env_status: Vec<(runtime::Status, runtime::Status)>,
    /// порт сервера из ini (DefaultPort)
    pub ini_port: String,
    /// раздел «Управление»: имя последнего бэкапа и когда смотрели
    pub dash_backup: Option<String>,
    pub dash_at: Option<std::time::Instant>,
    /// Mods= из ini активного окружения: (файл есть, id по порядку, B42-стиль с «\»); обновляется раз в секунду
    pub ini_state: (bool, Vec<String>, bool),
    pub rt_polled: Option<std::time::Instant>,
    pub srv_log: Vec<String>,
    pub srv_input: String,
    pub srv_admin_pw: String,
    /// mtime config.toml после нашей последней записи/чтения
    pub cfg_mtime: std::cell::Cell<Option<std::time::SystemTime>>,
    pub bg_tx: std::sync::mpsc::Sender<Bg>,
    pub bg_rx: Receiver<Bg>,
    pub busy: u32,
    pub undo_order: Option<Vec<String>>,
    // песочница
    pub sb_fields: Vec<sandbox::Field>,
    pub sb_edits: HashMap<(String, String), String>,
    pub sb_filter: String,
    pub sb_only_changed: bool,
    pub sb_for: std::path::PathBuf,
    pub sb_status: String,
    // шаблоны
    pub tpl_confirm: Option<usize>,
    // бэкапы
    pub bk_list: Vec<backup::Entry>,
    pub bk_label: String,
    pub bk_confirm: Option<String>,
    // игроки
    pub players: Vec<players::Player>,
    pub pl_sel: Option<String>,
    pub pl_loaded: Option<std::time::Instant>,
    pub cmd_input: String,
    pub cmd_log: Vec<String>,
    pub broadcast: String,
    // карта
    pub map_view: map::View,
    pub map_fit: bool,
    pub marker_name: String,
    pub map_ctx: Option<(f32, f32)>,
    pub shot_frames: u32,
    // обновления локальных модов
    pub tracking: tracking::Tracking,
    pub remote: HashMap<String, workshop_api::Remote>,
    pub remote_at: Option<String>,
    pub checking: bool,
    pub changelogs: HashMap<String, Option<Result<Vec<workshop_api::Change>, String>>>,
    pub upd_sel: Option<String>,
    pub upd_filter: String,
    pub upd_only: bool,
    pub upd_client: bool,
    pub upd_confirm: Option<String>,
    pub upd_pin_confirm: bool,
    pub upd_log: Vec<String>,
    pub self_info: Option<selfupdate::Info>,
    pub self_busy: bool,
    pub self_msg: String,
    pub link_input: HashMap<String, String>,
    pub scan_links: HashMap<String, String>,
    pub proposals: Vec<matching::Proposal>,
    pub pk_text: String,
    pub pk_cmp: Option<usize>,
    pub pk_apply_confirm: bool,
    pub pk_missing_ws: Vec<String>,
    pub pk_incoming: Option<tab_share::Incoming>,
    /// поля блока «Сеть»: ключ окружения, порт, UDP-порт
    pub net: (String, String, String),
    pub share_polled: Option<std::time::Instant>,
    pub share_writing: bool,
    pub share_skip: HashMap<String, u64>,
    /// автор: какая ревизия уже записана в файл (по pack_id)
    pub share_written: HashMap<String, u64>,
    /// получатель: время изменения файла при последней проверке (по пути)
    pub share_seen: HashMap<String, std::time::SystemTime>,
    pub steam_cooldown: Option<std::time::Instant>,
    pub last_changelog_req: Option<std::time::Instant>,
    pub matching: Option<(usize, usize)>,
    pub match_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // двойной щелчок по .pzpack: программа стартует в окне с путём к файлу
    let open_file = (args.len() == 1).then(|| std::path::PathBuf::from(&args[0])).filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(share::EXT)));
    if open_file.is_none() && !args.is_empty() && args[0] != "gui" {
        std::process::exit(cli::main(args));
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 720.0]).with_min_inner_size([860.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PZ Mod Manager",
        options,
        Box::new(|cc| {
            style(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let mut app = App::new();
            if let Some(p) = open_file {
                app.open_pack(p);
            }
            Ok(Box::new(app))
        }),
    )
}

fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        let v = &mut s.visuals;
        v.panel_fill = Color32::from_rgb(24, 26, 24);
        v.window_fill = Color32::from_rgb(30, 33, 30);
        v.extreme_bg_color = Color32::from_rgb(18, 20, 18);
        v.selection.bg_fill = Color32::from_rgb(70, 105, 55);
        v.hyperlink_color = ACCENT;
        let r = egui::CornerRadius::same(6);
        v.widgets.noninteractive.corner_radius = r;
        v.widgets.inactive.corner_radius = r;
        v.widgets.hovered.corner_radius = r;
        v.widgets.active.corner_radius = r;
    });
}

impl App {
    fn new() -> Self {
        let (bg_tx, bg_rx) = channel();
        i18n::apply_setting(&Store::load().settings.language);
        let mut a = Self {
            store: Store::load(),
            installed: HashMap::new(),
            dups: HashSet::new(),
            cache: vec![],
            tab: Tab::Mods,
            filter: String::new(),
            selected: None,
            new_profile: String::new(),
            dl_input: String::new(),
            dl_log: vec![],
            dl_rx: None,
            dl_env: 0,
            status: String::new(),
            drag: None,
            last_rects: vec![],
            children: vec![],
            srv_status: None,
            cli_status: None,
            env_status: vec![],
            ini_state: (false, vec![], true),
            ini_port: String::new(),
            dash_backup: None,
            dash_at: None,
            rt_polled: None,
            srv_log: vec![],
            srv_input: String::new(),
            srv_admin_pw: String::new(),
            cfg_mtime: std::cell::Cell::new(Store::mtime()),
            bg_tx,
            bg_rx,
            busy: 0,
            undo_order: None,
            sb_fields: vec![],
            sb_edits: HashMap::new(),
            sb_filter: String::new(),
            sb_only_changed: false,
            sb_for: Default::default(),
            sb_status: String::new(),
            tpl_confirm: None,
            bk_list: vec![],
            bk_label: String::new(),
            bk_confirm: None,
            players: vec![],
            pl_sel: None,
            pl_loaded: None,
            cmd_input: String::new(),
            cmd_log: vec![],
            broadcast: String::new(),
            map_view: Default::default(),
            map_fit: true,
            marker_name: String::new(),
            map_ctx: None,
            shot_frames: 0,
            tracking: tracking::Tracking::load(),
            remote: HashMap::new(),
            remote_at: None,
            checking: false,
            changelogs: HashMap::new(),
            upd_sel: None,
            upd_filter: String::new(),
            upd_only: false,
            upd_client: false,
            upd_confirm: None,
            upd_pin_confirm: false,
            upd_log: vec![],
            self_info: None,
            self_busy: false,
            self_msg: String::new(),
            link_input: HashMap::new(),
            scan_links: HashMap::new(),
            proposals: vec![],
            pk_text: String::new(),
            pk_cmp: None,
            pk_apply_confirm: false,
            pk_missing_ws: vec![],
            pk_incoming: None,
            net: Default::default(),
            share_polled: None,
            share_writing: false,
            share_skip: HashMap::new(),
            share_written: HashMap::new(),
            share_seen: HashMap::new(),
            steam_cooldown: None,
            last_changelog_req: None,
            matching: None,
            match_cancel: Default::default(),
        };
        a.tab = match std::env::var("PZMM_TAB").as_deref() {
            Ok("download") => Tab::Download,
            Ok("server") => Tab::Server,
            Ok("sandbox") => Tab::Sandbox,
            Ok("backup") => Tab::Backup,
            Ok("players") => Tab::Players,
            Ok("map") => Tab::Map,
            Ok("updates") => Tab::Updates,
            Ok("packs") => Tab::Packs,
            Ok("settings") => Tab::Settings,
            _ => Tab::Mods,
        };
        a.dl_env = a.store.settings.active_server;
        a.rescan();
        a
    }

    fn rescan(&mut self) {
        let s = &self.store.settings;
        let sc = mods::scan_env(s, s.server());
        self.cache = mods::scan(&steam::cache_content(), Source::Cache);
        self.scan_links = App::compute_scan_links(&sc.all, &self.cache);
        self.dups = sc.dups;
        self.installed = sc.installed;
        self.status = tf!("Найдено модов: {}", self.installed.len());
    }

    fn save(&self) {
        self.store.save();
        self.cfg_mtime.set(Store::mtime());
    }

    /// Открыть папку в файловом менеджере системы
    pub fn open_folder(&mut self, p: &std::path::Path) {
        if !p.exists() {
            self.status = tf!("Папки нет: {}", p.display());
            return;
        }
        if let Err(e) = open::that(p) {
            self.status = tf!("Не удалось открыть {}: {}", p.display(), e);
        }
    }

    pub fn server_running(&self) -> bool {
        self.srv_status.as_ref().is_some_and(|s| s.running)
    }

    /// config.toml поменяли снаружи (CLI, агент, второй экземпляр) — перечитать
    fn reload_if_changed(&mut self) {
        let now = Store::mtime();
        if now == self.cfg_mtime.get() {
            return;
        }
        self.cfg_mtime.set(now);
        if let Ok(st) = Store::load_strict() {
            self.store = st;
            i18n::apply_setting(&self.store.settings.language);
            self.rt_polled = None;
            self.rescan();
            self.status = t("Настройки изменены снаружи — перечитаны").into();
        }
    }

    fn poll_download(&mut self) {
        let mut finished = false;
        if let Some(rx) = &self.dl_rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    steam::Msg::Log(l) => self.dl_log.push(l),
                    steam::Msg::Done(ok) => {
                        self.dl_log.push(if ok { t("✔ Готово").into() } else { t("✘ SteamCMD завершился с ошибкой").into() });
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.dl_rx = None;
            self.rescan();
        }
    }

    pub fn poll_bg(&mut self) {
        while let Ok(m) = self.bg_rx.try_recv() {
            match m {
                Bg::Cmd(t) => self.cmd_log.extend(t.lines().map(str::to_string)),
                Bg::Job(t) => {
                    self.busy = self.busy.saturating_sub(1);
                    self.status = t;
                    self.bk_list = backup::list(self.store.settings.server());
                }
                Bg::Remote(r) => {
                    self.checking = false;
                    match r {
                        Ok(list) => {
                            self.status = tf!("Проверено в Steam: {}", list.len());
                            self.remote = list.into_iter().map(|r| (r.id.clone(), r)).collect();
                            self.remote_at = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                        }
                        Err(e) => self.status = tf!("Проверка не удалась: {}", e),
                    }
                }
                Bg::Changelog(ws, r) => {
                    if r.as_ref().is_err_and(|e| e.contains("429")) {
                        self.steam_cooldown = Some(std::time::Instant::now() + std::time::Duration::from_secs(workshop_api::COOLDOWN_SECS));
                    }
                    self.changelogs.insert(ws, Some(r));
                }
                Bg::Log(l) => self.upd_log.push(l),
                Bg::Shared(m) => {
                    self.share_writing = false;
                    if let Some(m) = m {
                        self.status = m;
                    }
                }
                Bg::PackInstalled(m) => {
                    self.busy = self.busy.saturating_sub(1);
                    self.rescan();
                    self.status = m;
                }
                Bg::MatchProgress(i, n) => self.matching = Some((i, n)),
                Bg::Matches(r) => {
                    self.matching = None;
                    match r {
                        Ok(p) => {
                            self.status = tf!("Найдены варианты для {} модов — проверь и подтверди", p.len());
                            self.proposals = p;
                        }
                        Err(e) => self.status = tf!("Подбор не удался: {}", e),
                    }
                }
                Bg::SelfCheck(r) => {
                    self.self_busy = false;
                    match r {
                        Ok(i) => {
                            self.self_msg = if i.newer { tf!("Есть новая версия {}", &i.sha[..i.sha.len().min(7)]) } else { t("Установлена последняя версия") };
                            self.self_info = Some(i);
                        }
                        Err(e) => self.self_msg = format!("✘ {e}"),
                    }
                }
                Bg::SelfApplied(r) => {
                    self.self_busy = false;
                    match r {
                        Ok(()) => {
                            if let Ok(exe) = std::env::current_exe() {
                                if std::process::Command::new(exe).spawn().is_ok() {
                                    std::process::exit(0);
                                }
                            }
                            self.self_msg = t("Обновлено, перезапусти программу").to_string();
                        }
                        Err(e) => self.self_msg = format!("✘ {e}"),
                    }
                }
                Bg::Updated { ws, ok, msg, baseline } => {
                    self.busy = self.busy.saturating_sub(1);
                    self.status = msg;
                    if ok {
                        let key = self.store.settings.server().cache_dir.to_string_lossy().into_owned();
                        let it = self.tracking.server(&key).items.entry(ws).or_default();
                        it.baseline = baseline;
                        it.skipped_until = None;
                        self.tracking.save();
                        self.rescan();
                    }
                }
            }
        }
    }

    /// Команда серверу: в консоль запущенного нами сервера, иначе через RCON
    pub fn send_cmd(&mut self, cmd: String) {
        if let Err(e) = validate::command(&cmd) {
            self.cmd_log.push(format!("✘ {e}"));
            return;
        }
        self.cmd_log.push(format!("> {cmd}"));
        let s = self.store.settings.clone();
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let msg = match runtime::send_command(&s, s.server(), &cmd, std::time::Duration::from_millis(1500)) {
                Ok(r) if r.output.trim().is_empty() => t("(пустой ответ)").to_string(),
                Ok(r) => r.output,
                Err(e) => format!("✘ {e:#}"),
            };
            let _ = tx.send(Bg::Cmd(msg));
        });
    }

    /// Раз в секунду: статус сервера и клиента активного окружения, хвост лога сервера
    fn poll_server(&mut self) {
        self.children.retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_)) | Err(_)));
        if self.rt_polled.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) {
            return;
        }
        self.rt_polled = Some(std::time::Instant::now());
        let s = &self.store.settings;
        self.env_status = s.servers.iter().map(|c| (runtime::status(s, c, runtime::Role::Server), runtime::status(s, c, runtime::Role::Client))).collect();
        let (st, cl) = self.env_status[s.active_server.min(s.servers.len() - 1)].clone();
        if self.tab == Tab::Server {
            self.srv_log = runtime::tail_lines(&st.log, 400);
        }
        self.srv_status = Some(st);
        self.cli_status = Some(cl);
        self.refresh_ini();
    }

    fn refresh_ini(&mut self) {
        let ini = self.store.settings.server().ini();
        let (ids, backslash) = export::read_ini_mods(&ini);
        self.ini_state = (ini.exists(), ids, backslash);
        self.ini_port = ini::read_key(&ini, "DefaultPort").unwrap_or_default();
    }

    fn bg_job(&mut self, f: impl FnOnce() -> String + Send + 'static) {
        self.busy += 1;
        self.rt_polled = None;
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Bg::Job(f()));
        });
    }

    // ---------- вкладки ----------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let mut launch: Option<Launch> = None;
        let mut switch_to: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.heading(RichText::new("PZ Mod Manager").color(ACCENT));
            ui.add_space(12.0);
            self.env_switcher(ui, &mut switch_to);
            ui.add_space(6.0);
            ui.label(RichText::new(self.env_summary()).color(Color32::from_gray(150)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let green = Color32::from_rgb(70, 120, 55);
                let more = egui::Button::new(RichText::new("▾").strong().color(Color32::WHITE)).fill(green).min_size(egui::vec2(30.0, 30.0));
                egui::containers::menu::MenuButton::from_button(more).ui(ui, |ui| self.launch_menu(ui, &mut launch)).0.on_hover_text(t("Другие способы запуска: клиент и сервер любого окружения, со Steam или -nosteam"));
                let play = egui::Button::new(RichText::new(t("▶  Играть")).strong().color(Color32::WHITE)).fill(green).min_size(egui::vec2(120.0, 30.0));
                if ui.add(play).on_hover_text(t("Запустить Project Zomboid через Steam (клиент)")).clicked() {
                    launch = Some(Launch::Steam);
                }
            });
        });
        if let Some(l) = launch {
            self.do_launch(l);
        }
        if let Some(i) = switch_to {
            self.switch_env(i);
        }
    }

    /// Меню слева: разделы по группам
    fn nav(&mut self, ui: &mut egui::Ui) {
        let groups: [(String, Vec<(Tab, &str, String)>); 3] = [
            (t("Моды"), vec![(Tab::Mods, "☰", t("Моды")), (Tab::Download, "⬇", t("Загрузка")), (Tab::Updates, "⟳", t("Обновления")), (Tab::Packs, "📦", t("Сборки"))]),
            (t("Сервер"), vec![(Tab::Server, "▶", t("Управление")), (Tab::Sandbox, "🔧", t("Песочница")), (Tab::Players, "👥", t("Игроки")), (Tab::Map, "🗺", t("Карта")), (Tab::Backup, "💾", t("Бэкапы"))]),
            (String::new(), vec![(Tab::Settings, "⚙", t("Настройки"))]),
        ];
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (title, items) in groups {
                ui.add_space(10.0);
                if !title.is_empty() {
                    ui.label(RichText::new(title).size(11.5).color(Color32::from_gray(120)));
                    ui.add_space(2.0);
                }
                for (tab, icon, name) in items {
                    ui.selectable_value(&mut self.tab, tab, format!("{icon}  {name}"));
                }
            }
        });
    }

    /// Цвет точки состояния сервера: готов / запускается / выключен
    fn server_dot(st: Option<&runtime::Status>) -> Color32 {
        match st {
            Some(s) if s.running && s.ready == Some(true) => ACCENT,
            Some(s) if s.running => WARN,
            _ => Color32::from_gray(95),
        }
    }

    /// Переключатель окружений в шапке: Stable ● | Lab ●
    fn env_switcher(&self, ui: &mut egui::Ui, switch_to: &mut Option<usize>) {
        let active = self.store.settings.active_server;
        egui::Frame::new().stroke(egui::Stroke::new(1.0, Color32::from_gray(60))).corner_radius(7).inner_margin(2).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (i, c) in self.store.settings.servers.iter().enumerate() {
                    let st = self.env_status.get(i);
                    let sel = i == active;
                    let r = egui::Frame::new()
                        .fill(if sel { Color32::from_rgb(48, 66, 42) } else { Color32::TRANSPARENT })
                        .corner_radius(5)
                        .inner_margin(egui::Margin::symmetric(10, 4))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new("●").color(Self::server_dot(st.map(|s| &s.0))).size(11.0));
                            let txt = RichText::new(&c.label).size(14.0);
                            ui.label(if sel { txt.color(Color32::WHITE).strong() } else { txt.color(Color32::from_gray(170)) });
                            if st.is_some_and(|s| s.1.running) {
                                ui.label(RichText::new("🎮").size(11.0)).on_hover_text(t("клиент запущен"));
                            }
                        })
                        .response
                        .interact(egui::Sense::click());
                    let r = r.on_hover_text(self.env_tooltip(i));
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() && !sel {
                        *switch_to = Some(i);
                    }
                }
            });
        });
    }

    fn env_tooltip(&self, i: usize) -> String {
        let s = &self.store.settings;
        let c = &s.servers[i];
        tf!("Сервер: {}\nКлиент: {}\nИмя сервера: {}", c.cache_dir.display(), c.client_cache_dir(s).display(), c.name)
    }

    /// «сервер готов · клиент закрыт» для активного окружения
    fn env_summary(&self) -> String {
        let (srv, cli) = (self.srv_status.as_ref(), self.cli_status.as_ref());
        let a = match srv {
            Some(s) if s.running && s.ready == Some(true) => t("сервер готов"),
            Some(s) if s.running => t("сервер запускается…"),
            _ => t("сервер выключен"),
        };
        let b = if cli.is_some_and(|c| c.running) { t("клиент запущен") } else { t("клиент закрыт") };
        format!("{a} · {b}")
    }

    fn switch_env(&mut self, i: usize) {
        if i >= self.store.settings.servers.len() || i == self.store.settings.active_server {
            return;
        }
        self.store.settings.active_server = i;
        self.save();
        self.rescan();
        self.rt_polled = None;
        self.pl_loaded = None;
        self.dash_at = None;
        self.poll_server();
        self.status = tf!("Окружение: {}", self.store.settings.server().label);
    }

    /// Меню «▾»: запуск клиента и сервера любого окружения со Steam или без
    fn launch_menu(&self, ui: &mut egui::Ui, launch: &mut Option<Launch>) {
        let s = &self.store.settings;
        ui.set_min_width(330.0);
        ui.label(RichText::new(t("Игра (клиент)")).strong());
        if ui.button(t("▶ Через Steam (обычная папка игры)")).clicked() {
            *launch = Some(Launch::Steam);
        }
        if ui.button(t("▶ Обычная папка игры, -nosteam")).on_hover_text(platform::zomboid_user_dir().display().to_string()).clicked() {
            *launch = Some(Launch::PlainClient);
        }
        for (i, c) in s.servers.iter().enumerate() {
            let running = self.env_status.get(i).is_some_and(|x| x.1.running);
            let can = !running && runtime::client_plan(s, c, false, &[]).is_some();
            let hover = tf!("-cachedir={}", c.client_cache_dir(s).display());
            ui.horizontal(|ui| {
                if ui.add_enabled(can, egui::Button::new(tf!("▶ Клиент {}", c.label))).on_hover_text(&hover).clicked() {
                    *launch = Some(Launch::Client(i, false));
                }
                if ui.add_enabled(can, egui::Button::new("-nosteam")).on_hover_text(&hover).clicked() {
                    *launch = Some(Launch::Client(i, true));
                }
                if running {
                    ui.colored_label(ACCENT, t("запущен"));
                }
            });
        }
        ui.separator();
        ui.label(RichText::new(t("Сервер")).strong());
        for (i, c) in s.servers.iter().enumerate() {
            let running = self.env_status.get(i).is_some_and(|x| x.0.running);
            let has_ini = c.ini().exists();
            let can = !running && has_ini && server::launch_plan(c).is_some();
            let hover = if has_ini { tf!("-cachedir={}", c.cache_dir.display()) } else { t("Первый запуск — на вкладке «Сервер» (нужен пароль админа)") };
            ui.horizontal(|ui| {
                let main = if c.nosteam { tf!("▶ Сервер {} (-nosteam)", c.label) } else { tf!("▶ Сервер {} (Steam)", c.label) };
                if ui.add_enabled(can, egui::Button::new(main)).on_hover_text(&hover).clicked() {
                    *launch = Some(Launch::Server(i, c.nosteam));
                }
                let alt = if c.nosteam { t("со Steam") } else { "-nosteam".to_string() };
                if ui.add_enabled(can, egui::Button::new(alt)).on_hover_text(t("Один раз запустить в другом режиме (настройка окружения не меняется)")).clicked() {
                    *launch = Some(Launch::Server(i, !c.nosteam));
                }
                if running {
                    ui.colored_label(ACCENT, t("запущен"));
                }
            });
        }
    }

    fn do_launch(&mut self, l: Launch) {
        let s = self.store.settings.clone();
        let started = match l {
            Launch::Steam => {
                self.status = match open::that(format!("steam://rungameid/{}", steam::APP_ID)) {
                    Ok(_) => t("Запускаю Project Zomboid через Steam…").into(),
                    Err(e) => tf!("Не удалось запустить: {}", e),
                };
                return;
            }
            Launch::PlainClient => {
                let mut c = config::ServerCfg::default();
                c.client_cache_dir = platform::zomboid_user_dir();
                runtime::start_client(&s, &c, true, &[]).map(|r| (r, t("Клиент запускается, лог: {}")))
            }
            Launch::Client(i, nosteam) => runtime::start_client(&s, &s.servers[i], nosteam, &[]).map(|r| (r, t("Клиент запускается, лог: {}"))),
            Launch::Server(i, nosteam) => {
                let mut c = s.servers[i].clone();
                c.nosteam = nosteam;
                runtime::start_server(&s, &c, None).map(|r| (r, t("Сервер запускается, лог: {}")))
            }
        };
        self.status = match started {
            Ok((r, msg)) => {
                let text = i18n::fill(&msg, &[&r.log.display()]);
                self.children.push(r.child);
                text
            }
            Err(e) => tf!("Ошибка: {}", format!("{e:#}")),
        };
        self.rt_polled = None;
    }

    fn profile_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Профиль:"));
            let mut active = self.store.active;
            egui::ComboBox::from_id_salt("profile")
                .selected_text(self.store.profile().name.clone())
                .show_ui(ui, |ui| {
                    for (i, p) in self.store.profiles.iter().enumerate() {
                        ui.selectable_value(&mut active, i, &p.name);
                    }
                });
            if active != self.store.active {
                self.store.active = active;
                self.save();
            }
            ui.add(egui::TextEdit::singleline(&mut self.new_profile).hint_text(t("имя новой сборки")).desired_width(160.0));
            if ui.button(t("＋ Создать")).clicked() && !self.new_profile.trim().is_empty() {
                let enabled = self.store.profile().enabled.clone();
                self.store.profiles.push(config::Profile { name: self.new_profile.trim().into(), enabled, ..Default::default() });
                self.store.active = self.store.profiles.len() - 1;
                self.new_profile.clear();
                self.save();
            }
            if self.store.profiles.len() > 1 && ui.button(t("🗑 Удалить")).clicked() {
                self.store.profiles.remove(self.store.active);
                self.store.active = 0;
                self.save();
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(t("🔍 поиск")).desired_width(200.0));
            if ui.button(t("⟳ Обновить")).clicked() {
                self.rescan();
            }
            if ui.button(t("⇅ Авто-порядок")).on_hover_text(t("Зависимости (require=) встанут выше зависящих от них модов")).clicked() {
                let cur = self.store.profile().enabled.clone();
                let (sorted, warns) = sort::smart_sort(&cur, &self.installed);
                if sorted != cur {
                    self.undo_order = Some(cur);
                    self.store.profile_mut().enabled = sorted;
                    self.save();
                    self.status = t("Порядок отсортирован по зависимостям").into();
                } else {
                    self.status = t("Порядок уже правильный").into();
                }
                if !warns.is_empty() {
                    self.status = warns.join("; ");
                }
            }
            if self.undo_order.is_some() && ui.button(t("↶ Отменить")).clicked() {
                if let Some(o) = self.undo_order.take() {
                    self.store.profile_mut().enabled = o;
                    self.save();
                }
            }
        });
    }

    fn tab_mods(&mut self, ui: &mut egui::Ui) {
        self.profile_bar(ui);
        self.ini_bar(ui);
        ui.add_space(2.0);

        let order = self.store.profile().enabled.clone();
        let issues = mods::check(&order, &self.installed, &self.dups);
        let q = self.filter.to_lowercase();
        let matches = |m: &Mod| q.is_empty() || m.name.to_lowercase().contains(&q) || m.id.to_lowercase().contains(&q);

        let mut disabled: Vec<&Mod> = self.installed.values().filter(|m| !order.contains(&m.id) && m.source == mods::Source::ServerLocal && matches(m)).collect();
        disabled.sort_by_key(|m| m.name.to_lowercase());

        let (errs, warns) = issues.values().flatten().fold((0, 0), |(e, w), i| match i.sev {
            Severity::Error => (e + 1, w),
            Severity::Warn => (e, w + 1),
        });

        let mut act: Option<Act> = None;
        let mut select: Option<String> = None;
        let dragging = self.drag;
        let mut drag_start: Option<usize> = None;
        let mut dropped: Option<(usize, usize)> = None;
        let mut frame_rects: Vec<(usize, egui::Rect)> = vec![];

        let mut open_dir: Option<std::path::PathBuf> = None;
        egui::Panel::right("details").default_size(320.0).show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.heading(t("Детали"));
            ui.separator();
            if let Some(m) = self.selected.as_ref().and_then(|id| self.installed.get(id)) {
                ui.label(RichText::new(&m.name).strong().size(18.0));
                ui.label(RichText::new(&m.id).monospace().weak());
                ui.add_space(4.0);
                ui.label(tf!("Источник: {}", m.source.label()));
                if ui.button(t("📂 Открыть папку мода")).on_hover_text(m.path.display().to_string()).clicked() {
                    open_dir = Some(m.path.clone());
                }
                if let Some(w) = &m.workshop_id {
                    ui.hyperlink_to(format!("Workshop {w}"), format!("https://steamcommunity.com/sharedfiles/filedetails/?id={w}"));
                }
                if !m.requires.is_empty() {
                    ui.label(tf!("Требует: {}", m.requires.join(", ")));
                }
                ui.add_space(6.0);
                ui.label(&m.description);
                if let Some(list) = issues.get(&m.id) {
                    ui.add_space(8.0);
                    for i in list {
                        let (c, ic) = if i.sev == Severity::Error { (ERR, "⛔") } else { (WARN, "⚠") };
                        ui.colored_label(c, format!("{ic} {}", i.msg));
                    }
                }
            } else {
                ui.weak(t("Выбери мод в списке"));
            }
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tf!("Включено: {}", order.len()));
                if errs > 0 {
                    ui.colored_label(ERR, tf!("⛔ ошибок: {}", errs));
                }
                if warns > 0 {
                    ui.colored_label(WARN, tf!("⚠ предупреждений: {}", warns));
                }
                if errs == 0 && warns == 0 && !order.is_empty() {
                    ui.colored_label(ACCENT, t("✔ конфликтов нет"));
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                let mut rects: Vec<(usize, egui::Rect)> = vec![];
                let mut row =|ui: &mut egui::Ui, id: &str, idx: Option<usize>, total: usize| {
                    let m = self.installed.get(id);
                    let name = m.map(|m| m.name.as_str()).unwrap_or(id);
                    let enabled = idx.is_some();
                    let list = issues.get(id);
                    let sev = list.and_then(|l| l.iter().map(|i| i.sev).min_by_key(|s| (*s == Severity::Warn) as u8));
                    let selected = self.selected.as_deref() == Some(id);
                    let being_dragged = idx.is_some() && idx == dragging;
                    let fr = egui::Frame::new()
                        .fill(if being_dragged {
                            Color32::from_rgb(55, 75, 48)
                        } else if selected {
                            Color32::from_rgb(40, 52, 38)
                        } else {
                            Color32::from_rgb(30, 33, 30)
                        })
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if let Some(i) = idx {
                                    let h = ui.add(egui::Label::new(RichText::new("≡").size(18.0).weak()).sense(egui::Sense::drag()));
                                    if h.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                    }
                                    if h.drag_started() {
                                        drag_start = Some(i);
                                    }
                                }
                                let mut on = enabled;
                                if ui.checkbox(&mut on, "").changed() {
                                    act = Some(Act::Toggle(id.to_string()));
                                }
                                if let Some(i) = idx {
                                    ui.label(RichText::new(format!("{:>3}", i + 1)).monospace().weak());
                                }
                                let txt = RichText::new(name);
                                let r = ui.add(egui::Label::new(if enabled { txt.strong() } else { txt.weak() }).sense(egui::Sense::click()));
                                if r.clicked() {
                                    select = Some(id.to_string());
                                }
                                match sev {
                                    Some(Severity::Error) => {
                                        ui.colored_label(ERR, "⛔");
                                    }
                                    Some(Severity::Warn) => {
                                        ui.colored_label(WARN, "⚠");
                                    }
                                    None => {}
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if let Some(i) = idx {
                                        if ui.add_enabled(i + 1 < total, egui::Button::new("▼").small()).clicked() {
                                            act = Some(Act::Down(i));
                                        }
                                        if ui.add_enabled(i > 0, egui::Button::new("▲").small()).clicked() {
                                            act = Some(Act::Up(i));
                                        }
                                    }
                                    if let Some(m) = m {
                                        ui.weak(m.source.label());
                                    }
                                });
                            });
                        });
                    if let Some(i) = idx {
                        rects.push((i, fr.response.rect));
                    }
                };

                for (i, id) in order.iter().enumerate() {
                    if matches_id(id, &q, &self.installed) {
                        row(ui, id, Some(i), order.len());
                    }
                }

                if !disabled.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(t("Выключены")).weak());
                    for m in &disabled {
                        row(ui, &m.id, None, 0);
                    }
                }

                // перетаскивание: линия вставки и бросок
                if let (Some(from), Some(p)) = (dragging, ui.ctx().pointer_latest_pos()) {
                    if !rects.is_empty() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        let pos = rects.iter().filter(|(_, r)| r.center().y < p.y).count();
                        let (target, y) = if pos < rects.len() {
                            (rects[pos].0, rects[pos].1.top() - 3.0)
                        } else {
                            let (li, lr) = rects[rects.len() - 1];
                            (li + 1, lr.bottom() + 3.0)
                        };
                        ui.painter().hline(rects[0].1.x_range(), y, egui::Stroke::new(2.5, ACCENT));
                        if ui.ctx().input(|i| i.pointer.any_released()) {
                            dropped = Some((from, target));
                        }
                    }
                }
                frame_rects = rects.clone();
            });
        });

        self.last_rects = frame_rects;
        if let Some(d) = open_dir {
            self.open_folder(&d);
        }
        if let Some(id) = select {
            self.selected = Some(id);
        }
        if let Some(s) = drag_start {
            self.drag = Some(s);
        }
        if let Some((from, target)) = dropped {
            let en = &mut self.store.profile_mut().enabled;
            if from < en.len() {
                let item = en.remove(from);
                let at = if target > from { target - 1 } else { target };
                en.insert(at.min(en.len()), item);
                self.save();
            }
            self.drag = None;
        } else if self.drag.is_some() && !ui.ctx().input(|i| i.pointer.any_down()) {
            self.drag = None;
        }
        if let Some(a) = act {
            let en = &mut self.store.profile_mut().enabled;
            match a {
                Act::Toggle(id) => {
                    if let Some(p) = en.iter().position(|x| *x == id) {
                        en.remove(p);
                    } else {
                        en.push(id);
                    }
                }
                Act::Up(i) => en.swap(i, i - 1),
                Act::Down(i) => en.swap(i, i + 1),
            }
            self.save();
        }
    }

    fn tab_download(&mut self, ui: &mut egui::Ui) {
        let mut open_dir: Option<std::path::PathBuf> = None;
        ui.heading(t("Загрузка из Steam Workshop"));
        ui.weak(t("Вставь ссылки или id элементов воркшопа (по одному в строке или через запятую). SteamCMD скачается автоматически."));
        ui.add(egui::TextEdit::multiline(&mut self.dl_input).desired_rows(3).desired_width(f32::INFINITY).hint_text("https://steamcommunity.com/sharedfiles/filedetails/?id=…"));
        let busy = self.dl_rx.is_some();
        ui.horizontal(|ui| {
            let ids = steam::parse_ids(&self.dl_input);
            if ui.add_enabled(!busy && !ids.is_empty(), egui::Button::new(tf!("⬇ Скачать ({})", ids.len()))).clicked() {
                let (tx, rx) = channel();
                self.dl_log.clear();
                steam::download(ids, tx);
                self.dl_rx = Some(rx);
            }
            if busy {
                ui.spinner();
                ui.label(t("загрузка…"));
            }
        });

        ui.add_space(6.0);
        egui::ScrollArea::vertical().id_salt("log").max_height(150.0).stick_to_bottom(true).show(ui, |ui| {
            egui::Frame::new().fill(Color32::from_rgb(16, 18, 16)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.dl_log.is_empty() {
                    ui.weak(t("Лог SteamCMD появится здесь"));
                }
                for l in &self.dl_log {
                    ui.label(RichText::new(l).monospace().size(12.0));
                }
            });
        });

        ui.add_space(8.0);
        ui.separator();
        let envs: Vec<String> = self.store.settings.servers.iter().map(|c| c.label.clone()).collect();
        self.dl_env = self.dl_env.min(envs.len() - 1);
        ui.horizontal(|ui| {
            ui.heading(t("Скачанные моды"));
            if ui.button(t("📂 Папка загрузок")).on_hover_text(steam::cache_content().display().to_string()).clicked() {
                open_dir = Some(steam::cache_content());
            }
            ui.separator();
            ui.label(t("Ставить в окружение:"));
            egui::ComboBox::from_id_salt("dl_env").selected_text(envs[self.dl_env].clone()).show_ui(ui, |ui| {
                for (i, l) in envs.iter().enumerate() {
                    ui.selectable_value(&mut self.dl_env, i, l);
                }
            });
        });
        let mut items: Vec<(String, Vec<&Mod>)> = vec![];
        for m in &self.cache {
            let w = m.workshop_id.clone().unwrap_or_default();
            match items.iter_mut().find(|(k, _)| *k == w) {
                Some((_, v)) => v.push(m),
                None => items.push((w, vec![m])),
            }
        }
        let settings = &self.store.settings;
        let target = self.dl_env;
        let target_label = envs[target].clone();
        let nosteam = settings.servers[target].nosteam;
        // (workshop id, окружение, клиент, сервер)
        let mut install: Vec<(String, usize, bool, bool)> = vec![];
        egui::ScrollArea::vertical().id_salt("cache").show(ui, |ui| {
            if items.is_empty() {
                ui.weak(t("Пока ничего не скачано"));
            }
            for (w, ms) in &items {
                let placed: Vec<steam::Placed> = settings.servers.iter().map(|c| steam::placed(w, settings, c)).collect();
                egui::Frame::new().fill(Color32::from_rgb(30, 33, 30)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(ms.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")).strong());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(format!("Workshop {w}")).weak().monospace());
                                for (i, p) in placed.iter().enumerate() {
                                    let mark = |b: bool| if b { "✔" } else { "—" };
                                    let txt = tf!("{}: сервер {} · клиент {}", envs[i], mark(p.server), mark(p.client));
                                    if p.server || p.client {
                                        ui.colored_label(ACCENT, txt);
                                    } else {
                                        ui.weak(txt);
                                    }
                                }
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let hint = tf!("Поставить в окружение «{}»", target_label);
                            if ui.button(t("Клиент + сервер")).on_hover_text(&hint).clicked() {
                                install.push((w.clone(), target, true, true));
                            }
                            if ui.button(if nosteam { t("На сервер (локально)") } else { t("На сервер") }).on_hover_text(&hint).clicked() {
                                install.push((w.clone(), target, false, true));
                            }
                            if ui.button(t("На клиент")).on_hover_text(&hint).clicked() {
                                install.push((w.clone(), target, true, false));
                            }
                            // «влить»: то, что уже стоит в выбранном окружении, поставить туда, где этого ещё нет
                            let src = placed[target];
                            for (j, p) in placed.iter().enumerate() {
                                let missing = (src.server && !p.server) || (src.client && !p.client);
                                if j != target && missing {
                                    let b = ui.button(tf!("⇪ Влить в {}", envs[j])).on_hover_text(tf!("Поставить в «{}» так же, как стоит в «{}»", envs[j], target_label));
                                    if b.clicked() {
                                        install.push((w.clone(), j, src.client, src.server));
                                    }
                                }
                            }
                            let dir = steam::cache_content().join(w);
                            if ui.button("📂").on_hover_text(tf!("Открыть папку: {}", dir.display())).clicked() {
                                open_dir = Some(dir);
                            }
                        });
                    });
                });
            }
        });
        if let Some(d) = open_dir {
            self.open_folder(&d);
        }
        if !install.is_empty() {
            let s = &self.store.settings;
            let msgs: Vec<String> = install
                .iter()
                .map(|(w, e, client, server)| match steam::install_into(w, s, &s.servers[*e], *client, *server) {
                    Ok(m) => m,
                    Err(e) => tf!("Ошибка установки: {}", format!("{e:#}")),
                })
                .collect();
            self.status = msgs.join("; ");
            self.rescan();
        }
    }

    /// Сверка сборки с ini сервера активного окружения и запись в него (бывшая вкладка «Экспорт»)
    fn ini_bar(&mut self, ui: &mut egui::Ui) {
        let srv = self.store.settings.server().clone();
        let ini = srv.ini();
        let (exists, ini_mods, backslash) = self.ini_state.clone();
        let (m, w) = export::build(&self.store.profile().enabled, &self.installed, srv.nosteam, backslash);
        let d = packs::diff(&ini_mods, &self.store.profile().enabled);
        let same = exists && d.added.is_empty() && d.removed.is_empty() && d.moved.is_empty();
        let file = ini.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let (mut write, mut import) = (false, false);
        egui::Frame::new().fill(Color32::from_rgb(30, 33, 30)).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} · {file}", srv.label)).strong()).on_hover_text(ini.display().to_string());
                if !exists {
                    ui.colored_label(WARN, t("ini ещё нет — сначала запусти сервер (раздел «Управление»)"));
                } else if same {
                    ui.colored_label(ACCENT, t("✔ ini совпадает со сборкой"));
                } else {
                    ui.colored_label(WARN, tf!("ini отличается от сборки: добавить {}, убрать {}, переставить {}", d.added.len(), d.removed.len(), d.moved.len()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut b = egui::Button::new(RichText::new(t("💾 Записать в ini")).color(if exists && !same { Color32::WHITE } else { Color32::from_gray(190) }));
                    if exists && !same {
                        b = b.fill(Color32::from_rgb(70, 120, 55));
                    }
                    if ui.add_enabled(exists && !same, b).on_hover_text(t("Записать порядок модов этой сборки в ini сервера (старый ini сохранится рядом как .bak)")).clicked() {
                        write = true;
                    }
                    if ui.add_enabled(exists, egui::Button::new(t("⤓ Взять из ini"))).on_hover_text(t("Заменить порядок модов сборки тем, что записан в ini")).clicked() {
                        import = true;
                    }
                });
            });
            egui::CollapsingHeader::new(RichText::new(t("Строки для ini (скопировать вручную)")).weak()).id_salt("ini_lines").show(ui, |ui| {
                if srv.nosteam {
                    ui.weak(t("Режим no-steam: WorkshopItems остаётся пустым, моды берутся из локальной папки сервера."));
                }
                for (label, val) in [("Mods=", &m), ("WorkshopItems=", &w)] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).strong());
                        if ui.small_button(t("Копировать")).clicked() {
                            ui.ctx().copy_text(format!("{label}{val}"));
                            self.status = tf!("{} скопировано", label);
                        }
                    });
                    let mut txt = val.clone();
                    ui.add(egui::TextEdit::multiline(&mut txt).desired_rows(2).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                }
            });
        });
        if write {
            self.status = match export::write_ini(&ini, &m, &w) {
                Ok(_) if self.server_running() => tf!("{} обновлён. Сервер запущен — перезапусти его, чтобы список модов применился", file),
                Ok(_) => tf!("{} обновлён (прежняя версия сохранена рядом как .bak)", file),
                Err(e) => tf!("Ошибка записи: {}", format!("{e:#}")),
            };
            self.refresh_ini();
        }
        if import {
            self.status = tf!("Из ini загружено модов: {}", ini_mods.len());
            self.undo_order = Some(self.store.profile().enabled.clone());
            self.store.profile_mut().enabled = ini_mods;
            self.save();
        }
    }

    fn tab_server(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("srvtab").auto_shrink([false; 2]).show(ui, |ui| self.tab_server_inner(ui));
    }

    fn refresh_dash(&mut self) {
        self.refresh_players();
        if self.dash_at.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5)) {
            self.dash_backup = backup::list(self.store.settings.server()).first().map(|e| e.name.clone());
            self.dash_at = Some(std::time::Instant::now());
        }
    }

    /// Раздел «Управление»: состояние сервера и клиента окружения, консоль, а параметры — в свёрнутом блоке
    /// Блок «Сеть»: адрес для игроков и порты из ini
    fn net_card(&mut self, ui: &mut egui::Ui, running: bool) {
        let srv = self.store.settings.server().clone();
        let ini = srv.ini();
        let key = srv.key();
        if self.net.0 != key {
            self.net = (key.clone(), ini::read_key(&ini, "DefaultPort").unwrap_or_default(), ini::read_key(&ini, "UDPPort").unwrap_or_default());
        }
        let exists = ini.exists();
        // порты других окружений: два сервера с одним портом не запустятся вместе
        let mut clash: Vec<String> = vec![];
        for o in self.store.settings.servers.iter().filter(|o| o.key() != key) {
            for k in ["DefaultPort", "UDPPort"] {
                let v = ini::read_key(&o.ini(), k).unwrap_or_default();
                if !v.is_empty() && (v == self.net.1 || v == self.net.2) {
                    clash.push(format!("{} ({k}={v})", o.label));
                }
            }
        }
        let (mut apply, mut reload, mut host_changed) = (false, false, false);
        ui.add_space(6.0);
        card(ui, |ui| {
            ui.label(RichText::new(t("Сеть")).strong());
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(t("Адрес для игроков"));
                host_changed = ui.add(egui::TextEdit::singleline(&mut self.store.settings.server_mut().public_host).desired_width(220.0).hint_text(t("IP или домен, например 203.0.113.5"))).changed();
                ui.label(t("порт"));
                ui.add_enabled(exists && !running, egui::TextEdit::singleline(&mut self.net.1).desired_width(60.0));
                ui.label(t("UDP-порт"));
                ui.add_enabled(exists && !running, egui::TextEdit::singleline(&mut self.net.2).desired_width(60.0));
            });
            let host = self.store.settings.server().public_host.trim().to_string();
            let port = if self.net.1.is_empty() { "16261".to_string() } else { self.net.1.clone() };
            ui.horizontal(|ui| {
                if host.is_empty() {
                    ui.weak(t("Адрес не задан: друзьям нечего вводить. Свой внешний IP можно посмотреть на любом сайте «мой IP»; для домашней сети лучше домен (DDNS)."));
                } else {
                    let addr = format!("{host}:{port}");
                    ui.label(t("Игроки подключаются:"));
                    ui.label(RichText::new(&addr).monospace().strong());
                    if ui.small_button(t("копировать")).clicked() {
                        ui.ctx().copy_text(addr);
                    }
                }
            });
            if !exists {
                ui.weak(t("Порты появятся после первого запуска сервера: тогда создаётся ini."));
            } else {
                let valid = |v: &str| v.parse::<u16>().is_ok_and(|n| n >= 1024);
                let bad = !(valid(&self.net.1) && valid(&self.net.2)) || self.net.1 == self.net.2;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!running && !bad, egui::Button::new(t("Записать порты в ini"))).on_hover_text(t("Вступит в силу после перезапуска сервера")).clicked() {
                        apply = true;
                    }
                    if ui.button("↻").on_hover_text(t("Перечитать из ini")).clicked() {
                        reload = true;
                    }
                    if running {
                        ui.colored_label(WARN, t("Сервер запущен: порты меняются только когда он остановлен."));
                    } else if bad {
                        ui.colored_label(ERR, t("Порты: числа от 1024 до 65535, разные."));
                    }
                });
                if !clash.is_empty() {
                    ui.colored_label(WARN, tf!("Те же порты у: {}. Вместе эти серверы не запустятся.", clash.join(", ")));
                }
                ui.weak(tf!("Открой у роутера и в файрволе UDP-порты {} и {} (если на сервере Windows — ещё правило входящих для java).", self.net.1, self.net.2));
            }
        });
        if host_changed {
            self.save();
        }
        if apply {
            let kv = vec![("DefaultPort".to_string(), self.net.1.clone()), ("UDPPort".to_string(), self.net.2.clone())];
            self.status = match ini::set_existing(&ini, &kv) {
                Ok(_) => tf!("Порты записаны: {} / {}", self.net.1, self.net.2),
                Err(e) => tf!("Ошибка записи: {}", format!("{e:#}")),
            };
            self.ini_port = self.net.1.clone();
        }
        if reload {
            self.net.0.clear();
        }
    }

    fn tab_server_inner(&mut self, ui: &mut egui::Ui) {
        self.refresh_dash();
        let settings = self.store.settings.clone();
        let srv = settings.server().clone();
        let plan = server::launch_plan(&srv);
        let st = self.srv_status.clone();
        let running = self.server_running();
        let ready = st.as_ref().is_some_and(|s| s.ready == Some(true));
        let first_run = !srv.ini().exists();
        let mut act: Option<&str> = None;

        // ---- сервер ----
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(Self::server_dot(st.as_ref())).size(22.0));
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("{} · {}", srv.label, srv.name)).size(18.0).strong());
                    let mut parts = vec![if ready {
                        t("сервер готов")
                    } else if running {
                        t("сервер запускается…")
                    } else {
                        t("сервер выключен")
                    }];
                    if let Some(up) = runtime::started_at(&srv.cache_dir, runtime::Role::Server).and_then(|t| t.elapsed().ok()).filter(|_| running) {
                        parts.push(tf!("работает {}", dur_text(up.as_secs())));
                    }
                    if !self.ini_port.is_empty() {
                        parts.push(tf!("порт {}", self.ini_port));
                    }
                    parts.push(if srv.nosteam { "-nosteam".to_string() } else { "Steam".to_string() });
                    if st.as_ref().is_some_and(|s| s.running && !s.managed) {
                        parts.push(t("запущен не из менеджера"));
                    }
                    ui.label(RichText::new(parts.join(" · ")).color(DIM));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if running {
                        if ui.button(RichText::new(t("Убить процесс")).color(DIM)).on_hover_text(t("Завершить без сохранения мира — только если сервер завис")).clicked() {
                            act = Some("kill");
                        }
                        if ui.button(t("■ Остановить")).on_hover_text(t("quit: сервер сохранит мир и выключится")).clicked() {
                            act = Some("quit");
                        }
                        if ui.add_enabled(self.busy == 0, egui::Button::new(t("⟲ Перезапустить"))).on_hover_text(t("Остановить с сохранением и запустить снова")).clicked() {
                            act = Some("restart");
                        }
                        if !ready {
                            ui.spinner();
                        }
                    } else {
                        let can = plan.is_some() && (!first_run || !self.srv_admin_pw.trim().is_empty());
                        let b = egui::Button::new(RichText::new(t("▶ Запустить сервер")).strong().color(Color32::WHITE)).fill(GREEN_BTN).min_size(egui::vec2(0.0, 30.0));
                        if ui.add_enabled(can, b).clicked() {
                            act = Some("start");
                        }
                        if first_run {
                            ui.add(egui::TextEdit::singleline(&mut self.srv_admin_pw).password(true).desired_width(150.0).hint_text(t("пароль админа")));
                        }
                    }
                });
            });
            if first_run && !running {
                ui.colored_label(WARN, tf!("Сервер ещё ни разу не запускался: задай пароль админа, и первый запуск создаст {}", platform::short(&srv.ini())));
            }
            if plan.is_none() {
                ui.colored_label(WARN, t("В папке игры нет скрипта запуска для этой системы — запуск отсюда недоступен (моды и ini всё равно можно редактировать)."));
            }
        });

        // ---- цифры ----
        ui.add_space(2.0);
        let (ini_exists, ini_mods, _) = &self.ini_state;
        let d = packs::diff(ini_mods, &self.store.profile().enabled);
        let same = d.added.is_empty() && d.removed.is_empty() && d.moved.is_empty();
        let online = self.players.iter().filter(|p| p.online).count();
        let backup = self.dash_backup.as_deref().and_then(backup_age);
        ui.columns(3, |c| {
            let (sub, col) = if !ini_exists {
                (t("ini ещё нет"), DIM)
            } else if same {
                (tf!("совпадает со сборкой «{}»", self.store.profile().name), ACCENT)
            } else {
                (tf!("отличается от сборки «{}»", self.store.profile().name), WARN)
            };
            tile(&mut c[0], &t("Моды в ini"), &ini_mods.len().to_string(), &sub, col);
            tile(&mut c[1], &t("Игроки онлайн"), &online.to_string(), &tf!("всего в логах: {}", self.players.len()), DIM);
            match backup {
                Some(age) => tile(&mut c[2], &t("Последний бэкап"), &tf!("{} назад", dur_text(age)), self.dash_backup.as_deref().unwrap_or_default(), DIM),
                None => tile(&mut c[2], &t("Последний бэкап"), "—", &t("бэкапов пока нет"), DIM),
            }
        });

        // ---- клиент ----
        ui.add_space(2.0);
        let client_running = self.cli_status.as_ref().is_some_and(|c| c.running);
        let client_dir = srv.client_cache_dir(&settings);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎮").size(16.0));
                ui.label(RichText::new(tf!("Клиент {}", srv.label)).strong());
                if client_running {
                    ui.colored_label(ACCENT, t("клиент запущен"));
                }
                ui.label(RichText::new(platform::short(&client_dir)).color(DIM)).on_hover_text(tf!("-cachedir={}", client_dir.display()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can = !client_running && runtime::client_plan(&settings, &srv, false, &[]).is_some();
                    if client_running && ui.button(t("■ Закрыть клиент")).clicked() {
                        act = Some("client-kill");
                    }
                    if ui.add_enabled(can, egui::Button::new("-nosteam")).on_hover_text(t("Запустить клиент без Steam")).clicked() {
                        act = Some("client-nosteam");
                    }
                    if ui.add_enabled(can, egui::Button::new(t("▶ Запустить клиент"))).on_hover_text(t("Нужен запущенный Steam")).clicked() {
                        act = Some("client");
                    }
                });
            });
        });

        // ---- консоль ----
        ui.add_space(2.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(t("Консоль сервера")).strong());
                if let Some(st) = &st {
                    ui.label(RichText::new(platform::short(&st.log)).color(DIM).size(12.0));
                    if ui.small_button("📂").on_hover_text(t("Открыть папку с логом")).clicked() {
                        act = Some("open-log");
                    }
                }
            });
            // консоль тянется до низа окна (запас под поле ввода команды)
            let log_h = (ui.clip_rect().bottom() - ui.cursor().top() - 70.0).max(160.0);
            egui::ScrollArea::vertical().id_salt("srvlog").stick_to_bottom(true).min_scrolled_height(log_h).max_height(log_h).auto_shrink([false, true]).show(ui, |ui| {
                egui::Frame::new().fill(Color32::from_rgb(16, 18, 16)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if self.srv_log.is_empty() {
                        ui.weak(t("Консоль сервера появится здесь"));
                    }
                    for l in &self.srv_log {
                        ui.label(RichText::new(l).monospace().size(12.0));
                    }
                });
            });
            ui.add_enabled_ui(running, |ui| {
                let hint = if running { t("команда серверу, Enter — отправить") } else { t("сервер выключен") };
                let r = ui.add(egui::TextEdit::singleline(&mut self.srv_input).hint_text(hint).desired_width(f32::INFINITY));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.srv_input.trim().is_empty() {
                    act = Some("cmd");
                    r.request_focus();
                }
            });
        });

        // ---- действия ----
        match act {
            Some("start") => {
                let pw = std::mem::take(&mut self.srv_admin_pw);
                match runtime::start_server(&settings, &srv, Some(pw.as_str()).filter(|p| !p.trim().is_empty())) {
                    Ok(started) => {
                        self.children.push(started.child);
                        self.status = tf!("Сервер запускается, лог: {}", started.log.display());
                    }
                    Err(e) => self.status = tf!("Ошибка: {}", format!("{e:#}")),
                }
                self.rt_polled = None;
            }
            Some("quit") => {
                let (s2, c2) = (settings.clone(), srv.clone());
                self.bg_job(move || match runtime::send_command(&s2, &c2, "quit", std::time::Duration::from_millis(200)) {
                    Ok(_) => t("Команда quit отправлена — сервер сохраняет мир и выключается").into(),
                    Err(e) => tf!("Ошибка: {}", format!("{e:#}")),
                });
            }
            Some("restart") => {
                let (s2, c2) = (settings.clone(), srv.clone());
                self.status = t("Перезапуск: сервер сохраняет мир и выключается…").into();
                self.bg_job(move || {
                    match runtime::stop_server(&s2, &c2, std::time::Duration::from_secs(120), false) {
                        Ok(r) if r.stopped => {}
                        Ok(r) => return tf!("Перезапуск отменён: {}", r.note),
                        Err(e) => return tf!("Ошибка: {}", format!("{e:#}")),
                    }
                    match runtime::start_server(&s2, &c2, None) {
                        Ok(st) => tf!("Сервер запускается, лог: {}", st.log.display()),
                        Err(e) => tf!("Ошибка: {}", format!("{e:#}")),
                    }
                });
            }
            Some("kill") => {
                let (s2, c2) = (settings.clone(), srv.clone());
                self.bg_job(move || match runtime::kill(&s2, &c2, runtime::Role::Server) {
                    Ok(r) if r.stopped => t("Процесс сервера завершён").into(),
                    Ok(_) => t("Сервер не завершился").into(),
                    Err(e) => tf!("Ошибка: {}", format!("{e:#}")),
                });
            }
            Some(c @ ("client" | "client-nosteam")) => {
                match runtime::start_client(&settings, &srv, c == "client-nosteam", &[]) {
                    Ok(started) => {
                        self.children.push(started.child);
                        self.status = tf!("Клиент запускается, лог: {}", started.log.display());
                    }
                    Err(e) => self.status = tf!("Ошибка: {}", format!("{e:#}")),
                }
                self.rt_polled = None;
            }
            Some("client-kill") => {
                let (s2, c2) = (settings.clone(), srv.clone());
                self.bg_job(move || match runtime::kill(&s2, &c2, runtime::Role::Client) {
                    Ok(r) if r.stopped => t("Клиент закрыт").into(),
                    Ok(_) => t("Клиент не завершился").into(),
                    Err(e) => tf!("Ошибка: {}", format!("{e:#}")),
                });
            }
            Some("cmd") => {
                let c = std::mem::take(&mut self.srv_input);
                let (s2, c2) = (settings.clone(), srv.clone());
                self.bg_job(move || match runtime::send_command(&s2, &c2, &c, std::time::Duration::from_millis(0)) {
                    Ok(r) if r.via == "rcon" => r.output,
                    Ok(_) => tf!("> {}", c),
                    Err(e) => tf!("Ошибка: {}", format!("{e:#}")),
                });
            }
            Some("open-log") => {
                if let Some(dir) = st.as_ref().and_then(|s| s.log.parent().map(|p| p.to_path_buf())) {
                    self.open_folder(&dir);
                }
            }
            _ => {}
        }

        self.net_card(ui, running);

        // ---- параметры окружения ----
        ui.add_space(6.0);
        let mut changed = false;
        egui::CollapsingHeader::new(RichText::new(t("Параметры окружения: папки, имя сервера, режим")).strong()).id_salt("env_params").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(t("＋ Добавить окружение")).clicked() {
                    let mut c = config::ServerCfg::default();
                    c.label = tf!("Сервер {}", self.store.settings.servers.len() + 1);
                    c.id = format!("server{}", self.store.settings.servers.len() + 1);
                    self.store.settings.servers.push(c);
                    self.store.settings.active_server = self.store.settings.servers.len() - 1;
                    changed = true;
                }
                if self.store.settings.servers.len() > 1 && ui.add_enabled(!running, egui::Button::new(t("🗑 Удалить это окружение"))).on_hover_text(t("Убирает только запись из настроек, файлы остаются")).clicked() {
                    let i = self.store.settings.active_server;
                    self.store.settings.servers.remove(i);
                    self.store.settings.active_server = 0;
                    changed = true;
                }
            });
            ui.add_space(4.0);
            let c = self.store.settings.server_mut();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(t("Название"));
                    changed |= ui.add(egui::TextEdit::singleline(&mut c.label).desired_width(260.0)).changed();
                });
                ui.vertical(|ui| {
                    ui.label(t("Id окружения (для CLI и агентов)"));
                    let hint = c.key();
                    changed |= ui.add(egui::TextEdit::singleline(&mut c.id).desired_width(140.0).hint_text(hint)).changed();
                });
            });
            changed |= path_row(ui, t("Папка данных сервера (-cachedir, внутри Server/ и mods/)"), &mut c.cache_dir, true);
            changed |= path_row(ui, t("Папка данных клиента этого окружения (-cachedir клиента; пусто — общие моды клиента)"), &mut c.client_cache_dir, true);
            changed |= path_row(ui, t("Папка игры / сервера (start-server.sh, StartServer64_nosteam.bat)"), &mut c.game_dir, true);
            if !c.nosteam {
                changed |= path_row(ui, t("Воркшоп сервера (steamapps/workshop/content/108600)"), &mut c.workshop_dir, true);
            }
            ui.horizontal(|ui| {
                ui.label(t("Имя сервера:"));
                let names = c.list_names();
                egui::ComboBox::from_id_salt("srvname").selected_text(c.name.clone()).show_ui(ui, |ui| {
                    for n in &names {
                        changed |= ui.selectable_value(&mut c.name, n.clone(), n).changed();
                    }
                });
                if names.is_empty() {
                    changed |= ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(160.0)).changed();
                    ui.weak(t("ini появится после первого запуска"));
                }
            });
            ui.horizontal(|ui| {
                ui.label(t("Память Java (МБ):"));
                changed |= ui.add(egui::DragValue::new(&mut c.xmx_mb).range(1024..=65536).speed(256)).changed();
                ui.weak(t("(Windows; на Linux задаётся в ProjectZomboid64.json)"));
            });
            changed |= ui.checkbox(&mut c.nosteam, t("No-steam (-nosteam): без Steam, моды только из локальной папки")).changed();
            ui.label(t("Доп. аргументы запуска"));
            changed |= ui.add(egui::TextEdit::singleline(&mut c.extra_args).desired_width(f32::INFINITY).hint_text(t("например: -adminpassword …"))).changed();
            ui.weak(tf!("Локальные моды сервера: {}", platform::short(&c.mods_dir())));
            ui.weak(tf!("Конфиг: {}", platform::short(&c.ini())));
        });
        if changed {
            self.save();
            self.rescan();
            self.rt_polled = None;
            self.dash_at = None;
        }
    }

    fn tab_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading(t("Пути"));
        let mut changed = false;
        let s = &mut self.store.settings;
        for (label, p) in [
            (t("Моды клиента (если у окружения не задана папка данных клиента)"), &mut s.client_mods_dir),
            (t("Воркшоп клиента (Steam)"), &mut s.client_workshop_dir),
            (t("Папка установленного клиента игры (общая для всех окружений)"), &mut s.client_game_dir),
        ] {
            changed |= path_row(ui, label, p, true);
        }
        ui.add_space(8.0);
        ui.weak(tf!("SteamCMD и кэш загрузок: {}", platform::short(&config::data_dir())));
        ui.add_space(10.0);
        ui.heading(t("Обновление программы"));
        ui.weak(tf!("Версия: {}", &selfupdate::COMMIT[..selfupdate::COMMIT.len().min(7)]));
        ui.horizontal(|ui| {
            let newer = self.self_info.as_ref().is_some_and(|i| i.newer);
            if newer {
                if ui.add_enabled(!self.self_busy, egui::Button::new(t("⬆ Обновить и перезапустить"))).clicked() {
                    if let Some(info) = self.self_info.clone() {
                        self.self_busy = true;
                        self.self_msg = t("Скачиваю…").to_string();
                        let tx = self.bg_tx.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(Bg::SelfApplied(selfupdate::apply(&info).map(|_| ()).map_err(|e| format!("{e:#}"))));
                        });
                    }
                }
            } else if ui.add_enabled(!self.self_busy, egui::Button::new(t("⟳ Проверить обновление"))).clicked() {
                self.self_busy = true;
                self.self_msg = t("Проверяю…").to_string();
                let tx = self.bg_tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Bg::SelfCheck(selfupdate::check().map_err(|e| format!("{e:#}"))));
                });
            }
            ui.label(&self.self_msg);
        });
        if let Some(i) = self.self_info.as_ref().filter(|i| i.newer && !i.notes.is_empty()) {
            ui.weak(&i.notes);
        }
        ui.add_space(10.0);
        ui.heading("Language / Язык");
        {
            let cur = self.store.settings.language.clone();
            let label = |l: &str| match l {
                "ru" => "Русский".to_string(),
                "en" => "English".to_string(),
                _ => t("Как в системе"),
            };
            let mut pick = cur.clone();
            egui::ComboBox::from_id_salt("lang").selected_text(label(&cur)).show_ui(ui, |ui| {
                for l in ["", "ru", "en"] {
                    ui.selectable_value(&mut pick, l.to_string(), label(l));
                }
            });
            if pick != cur {
                self.store.settings.language = pick;
                i18n::apply_setting(&self.store.settings.language);
                self.save();
            }
        }
        ui.add_space(10.0);
        ui.heading("Steam Web API");
        ui.label(t("Ключ нужен только для подбора привязок модов по названию (вкладка «Обновления»). Бесплатный, выдаётся на странице steamcommunity.com/dev/apikey."));
        let mut key_changed = false;
        ui.horizontal(|ui| {
            key_changed = ui.add(egui::TextEdit::singleline(&mut self.store.settings.steam_api_key).password(true).hint_text(t("ключ API")).desired_width(320.0)).changed();
            ui.hyperlink_to(t("получить ключ"), "https://steamcommunity.com/dev/apikey");
        });
        ui.weak(t("Хранится в config.toml открытым текстом на этом компьютере и уходит только в запросы к api.steampowered.com."));
        if key_changed {
            self.save();
        }
        if changed {
            self.save();
            self.rescan();
        }
    }
}

fn matches_id(id: &str, q: &str, installed: &HashMap<String, Mod>) -> bool {
    q.is_empty() || id.to_lowercase().contains(q) || installed.get(id).is_some_and(|m| m.name.to_lowercase().contains(q))
}

fn path_row(ui: &mut egui::Ui, label: impl AsRef<str>, p: &mut std::path::PathBuf, dir: bool) -> bool {
    let mut changed = false;
    ui.label(label.as_ref());
    ui.horizontal(|ui| {
        let mut txt = p.to_string_lossy().into_owned();
        if ui.add(egui::TextEdit::singleline(&mut txt).desired_width(ui.available_width() - 100.0)).changed() {
            *p = txt.into();
            changed = true;
        }
        if ui.button(t("Обзор…")).clicked() {
            let d = rfd::FileDialog::new();
            let pick = if dir { d.pick_folder() } else { d.pick_file() };
            if let Some(n) = pick {
                *p = n;
                changed = true;
            }
        }
    });
    changed
}

impl App {
    /// Отладка: PZMM_SHOT=<файл.png> — сохранить снимок СВОЕГО окна и выйти
    fn debug_screenshot(&mut self, ui: &egui::Ui) {
        let Ok(path) = std::env::var("PZMM_SHOT") else { return };
        self.shot_frames += 1;
        ui.ctx().request_repaint();
        if self.shot_frames == 8 {
            ui.ctx().request_screenshot(move |img| {
                let (w, h) = (img.width() as u32, img.height() as u32);
                let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(w, h, raw) {
                    let _ = buf.save(&path);
                }
                std::process::exit(0);
            });
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.debug_screenshot(ui);
        self.reload_if_changed();
        self.poll_download();
        self.poll_server();
        self.poll_bg();
        self.poll_share();
        if self.dl_rx.is_some() || self.busy > 0 || matches!(self.tab, Tab::Players | Tab::Map) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            // статус сервера/клиента и внешние изменения конфига видны без движения мыши
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }

        egui::Panel::top("top").show_inside(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            let order = &self.store.profile().enabled;
            let issues = mods::check(order, &self.installed, &self.dups);
            let (errs, warns) = issues.values().flatten().fold((0, 0), |(e, w), i| if i.sev == Severity::Error { (e + 1, w) } else { (e, w + 1) });
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if errs > 0 {
                        ui.colored_label(ERR, tf!("⛔ ошибок: {}", errs));
                    }
                    if warns > 0 {
                        ui.colored_label(WARN, tf!("⚠ предупреждений: {}", warns));
                    }
                    ui.label(RichText::new(tf!("{} · сборка «{}» · модов: {}", self.store.settings.server().label, self.store.profile().name, order.len())).color(DIM));
                });
            });
        });
        egui::Panel::left("nav").resizable(false).exact_size(168.0).show_inside(ui, |ui| self.nav(ui));

        self.share_window(ui.ctx());
        let tab = self.tab;
        if tab == Tab::Mods {
            egui::CentralPanel::default().show_inside(ui, |ui| self.tab_mods(ui));
        } else {
            egui::CentralPanel::default().show_inside(ui, |ui| match tab {
                Tab::Download => self.tab_download(ui),
                Tab::Server => self.tab_server(ui),
                Tab::Sandbox => self.tab_sandbox(ui),
                Tab::Backup => self.tab_backup(ui),
                Tab::Players => self.tab_players(ui),
                Tab::Map => self.tab_map(ui),
                Tab::Updates => self.tab_updates(ui),
                Tab::Packs => self.tab_packs(ui),
                Tab::Settings => self.tab_settings(ui),
                Tab::Mods => {}
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(id: &str, req: &[&str]) -> Mod {
        Mod {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            requires: req.iter().map(|s| s.to_string()).collect(),
            workshop_id: Some("1".into()),
            url: None,
            path: Default::default(),
            source: Source::ClientLocal,
        }
    }

    fn app(ids: &[&str]) -> App {
        let mut a = App::new();
        a.store = Store::default();
        a.installed = ids.iter().map(|i| (i.to_string(), fake(i, &[]))).collect();
        a.store.profile_mut().enabled = ids.iter().map(|s| s.to_string()).collect();
        a
    }

    fn frame(ctx: &egui::Context, a: &mut App, t: f64, events: Vec<egui::Event>) {
        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1100.0, 700.0)));
        raw.time = Some(t);
        raw.events = events;
        let mut out = ctx.run_ui(raw, |ui| a.tab_mods(ui));
        out.textures_delta.clear();
    }

    fn btn(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    #[test]
    fn launch_menu_renders_for_every_environment() {
        let mut a = app(&[]);
        let mut lab = config::ServerCfg::default();
        lab.id = "lab".into();
        lab.label = "Lab".into();
        lab.nosteam = false;
        a.store.settings.servers.push(lab);
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)));
        let mut launch = None;
        let mut out = ctx.run_ui(raw, |ui| a.launch_menu(ui, &mut launch));
        out.textures_delta.clear();
        assert!(launch.is_none(), "nothing is launched without a click");
    }

    #[test]
    fn drag_handle_reorders_rows() {
        let mut a = app(&["a", "b", "c", "d"]);
        let ctx = egui::Context::default();
        frame(&ctx, &mut a, 0.0, vec![]);
        frame(&ctx, &mut a, 0.1, vec![]);
        assert_eq!(a.last_rects.len(), 4, "rows must be laid out");

        // хватаем ручку первой строки ("a") и тащим ниже строки "c"
        let r0 = a.last_rects[0].1;
        let start = egui::pos2(r0.left() + 14.0, r0.center().y);
        let r2 = a.last_rects[2].1;
        let end = egui::pos2(start.x, r2.bottom() - 2.0); // ниже центра "c", выше "d"

        frame(&ctx, &mut a, 0.2, vec![egui::Event::PointerMoved(start)]);
        frame(&ctx, &mut a, 0.3, vec![btn(start, true)]);
        let steps = 8;
        for i in 1..=steps {
            let p = start + (end - start) * (i as f32 / steps as f32);
            frame(&ctx, &mut a, 0.3 + i as f64 * 0.05, vec![egui::Event::PointerMoved(p)]);
        }
        assert_eq!(a.drag, Some(0), "drag should have started");
        frame(&ctx, &mut a, 1.0, vec![btn(end, false)]);
        frame(&ctx, &mut a, 1.1, vec![]);

        assert_eq!(a.store.profile().enabled, ["b", "c", "a", "d"]);
        assert_eq!(a.drag, None);
    }

    #[test]
    fn drag_to_top_and_bottom() {
        for (from, to_y_top, expect) in [(2usize, true, ["c", "a", "b", "d"]), (0, false, ["b", "c", "d", "a"])] {
            let mut a = app(&["a", "b", "c", "d"]);
            let ctx = egui::Context::default();
            frame(&ctx, &mut a, 0.0, vec![]);
            frame(&ctx, &mut a, 0.1, vec![]);
            let r = a.last_rects[from].1;
            let start = egui::pos2(r.left() + 14.0, r.center().y);
            let end_y = if to_y_top { a.last_rects[0].1.top() + 1.0 } else { a.last_rects[3].1.bottom() + 20.0 };
            let end = egui::pos2(start.x, end_y);
            frame(&ctx, &mut a, 0.2, vec![egui::Event::PointerMoved(start)]);
            frame(&ctx, &mut a, 0.3, vec![btn(start, true)]);
            for i in 1..=8 {
                let p = start + (end - start) * (i as f32 / 8.0);
                frame(&ctx, &mut a, 0.3 + i as f64 * 0.05, vec![egui::Event::PointerMoved(p)]);
            }
            frame(&ctx, &mut a, 1.0, vec![btn(end, false)]);
            frame(&ctx, &mut a, 1.1, vec![]);
            assert_eq!(a.store.profile().enabled, expect);
        }
    }

    #[test]
    fn click_without_drag_does_not_reorder() {
        let mut a = app(&["a", "b", "c"]);
        let ctx = egui::Context::default();
        frame(&ctx, &mut a, 0.0, vec![]);
        frame(&ctx, &mut a, 0.1, vec![]);
        let r = a.last_rects[1].1;
        let p = egui::pos2(r.left() + 14.0, r.center().y);
        frame(&ctx, &mut a, 0.2, vec![egui::Event::PointerMoved(p)]);
        frame(&ctx, &mut a, 0.3, vec![btn(p, true)]);
        frame(&ctx, &mut a, 0.4, vec![btn(p, false)]);
        frame(&ctx, &mut a, 0.5, vec![]);
        assert_eq!(a.store.profile().enabled, ["a", "b", "c"]);
        assert_eq!(a.drag, None);
    }

    #[test]
    fn conflict_checks_and_export() {
        let mut inst: HashMap<String, Mod> = HashMap::new();
        inst.insert("lib".into(), fake("lib", &[]));
        inst.insert("addon".into(), fake("addon", &["lib", "ghost"]));
        let order = vec!["addon".to_string(), "lib".to_string()];
        let issues = mods::check(&order, &inst, &HashSet::new());
        let msgs: Vec<_> = issues["addon"].iter().map(|i| i.msg.clone()).collect();
        assert!(msgs.iter().any(|m| m.contains("ghost")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("выше")), "{msgs:?}");
        let (m, w) = export::build(&order, &inst, false, false);
        assert_eq!(m, "addon;lib");
        assert_eq!(w, "1");
    }

    #[test]
    fn parse_ids_from_links() {
        let ids = steam::parse_ids("https://steamcommunity.com/sharedfiles/filedetails/?id=2200148440 123, 123\n456;x");
        assert_eq!(ids, ["2200148440", "123", "456"]);
    }

    /// Живая проверка: cargo test live_download -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_download() {
        let id = std::env::var("PZ_TEST_ITEM").unwrap_or("2619072426".into());
        let (tx, rx) = std::sync::mpsc::channel();
        steam::download(vec![id.clone()], tx);
        let mut ok = false;
        for m in rx {
            match m {
                steam::Msg::Log(l) => println!("{l}"),
                steam::Msg::Done(o) => ok = o,
            }
        }
        assert!(ok, "steamcmd failed");
        let found = mods::scan(&steam::cache_content(), Source::Cache);
        println!("found: {:?}", found.iter().map(|m| (&m.id, &m.workshop_id)).collect::<Vec<_>>());
        assert!(found.iter().any(|m| m.workshop_id.as_deref() == Some(id.as_str())));

        let base = config::data_dir().join("e2e");
        let client = base.join("client_mods");
        let server = base.join("server_ws");
        println!("{}", steam::install(&id, Some(&client), Some(steam::ServerTarget::Workshop(&server))).unwrap());
        assert!(!mods::scan(&client, Source::ClientLocal).is_empty(), "client install empty");
        assert!(!mods::scan(&server, Source::ServerWorkshop).is_empty(), "server install empty");
    }

    #[test]
    fn write_ini_keeps_crlf_semicolon_and_old_backup() {
        let d = std::env::temp_dir().join("pzmm-test-ini");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("pack.ini");
        std::fs::write(&f, "A=1\r\nMods=\\x;\\y;\r\nWorkshopItems=\r\nB=2\r\n").unwrap();
        std::fs::write(d.join("pack.ini.bak"), "OLD").unwrap();
        export::write_ini(&f, "\\a;\\b", "").unwrap();
        let t = std::fs::read_to_string(&f).unwrap();
        assert_eq!(t, "A=1\r\nMods=\\a;\\b;\r\nWorkshopItems=\r\nB=2\r\n");
        assert_eq!(std::fs::read_to_string(d.join("pack.ini.bak")).unwrap(), "OLD", "чужой бэкап не тронут");
        assert!(std::fs::read_dir(&d).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("pack.ini.bak-2")));
        let (ids, bs) = export::read_ini_mods(&f);
        assert_eq!(ids, ["a", "b"]);
        assert!(bs);
    }

    #[test]
    fn server_args_and_names() {
        let tmp = std::env::temp_dir().join("pzmm-test-srv");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("Server")).unwrap();
        for f in ["pack.ini", "pack.ini.bak", "pack_SandboxVars.lua", "other.ini"] {
            std::fs::write(tmp.join("Server").join(f), "").unwrap();
        }
        let mut c = config::ServerCfg::default();
        c.cache_dir = tmp.clone();
        c.name = "pack".into();
        c.extra_args = "-x  -y".into();
        assert_eq!(c.list_names(), ["other", "pack"]);
        assert_eq!(c.mods_dir(), tmp.join("mods"));
        assert_eq!(c.ini(), tmp.join("Server/pack.ini"));
        let a = server::game_args(&c);
        assert_eq!(a[0], "-nosteam");
        assert_eq!(a[2..], ["-servername", "pack", "-x", "-y"]);
        c.nosteam = false;
        assert!(!server::game_args(&c).contains(&"-nosteam".to_string()));
    }

    /// Только чтение: PZ_WIN_DIR=<папка Zomboid> PZ_WIN_NAME=pack cargo test win_folder -- --ignored --nocapture
    #[test]
    #[ignore]
    fn win_folder_readonly() {
        let mut c = config::ServerCfg::default();
        c.cache_dir = std::env::var("PZ_WIN_DIR").unwrap().into();
        c.name = std::env::var("PZ_WIN_NAME").unwrap_or("pack".into());
        println!("servers: {:?}", c.list_names());
        let found = mods::scan(&c.mods_dir(), Source::ServerLocal);
        println!("local mods found: {}", found.len());
        let ini = std::fs::read_to_string(c.ini()).unwrap();
        let _ = ini;
        let (ids, bs) = export::read_ini_mods(&c.ini());
        println!("B42 backslash style: {bs}");
        let inst: HashMap<String, Mod> = found.into_iter().map(|m| (m.id.clone(), m)).collect();
        println!("ini Mods= {} ids, {} not found on disk", ids.len(), ids.iter().filter(|i| !inst.contains_key(*i)).count());
        println!("missing: {:?}", ids.iter().filter(|i| !inst.contains_key(*i)).take(10).collect::<Vec<_>>());
        let issues = mods::check(&ids, &inst, &HashSet::new());
        let (e, w) = issues.values().flatten().fold((0, 0), |(e, w), i| if i.sev == Severity::Error { (e + 1, w) } else { (e, w + 1) });
        println!("conflict check: {e} errors, {w} warnings");
        let (m, ws) = export::build(&ids, &inst, true, bs);
        let orig = std::fs::read_to_string(c.ini()).unwrap();
        let orig_line = orig.lines().find(|l| l.starts_with("Mods=")).unwrap();
        {
            let a = format!("Mods={m}");
            let b = orig_line.trim_end().trim_end_matches(';');
            println!("len mine={} orig={} firstdiff={:?}", a.len(), b.len(), a.chars().zip(b.chars()).position(|(x, y)| x != y));
        }
        println!("export roundtrip identical to ini (ignoring trailing ;): {}, WorkshopItems empty: {}", format!("Mods={m}") == orig_line.trim_end().trim_end_matches(';'), ws.is_empty());
        c.game_dir = "/nonexistent".into();
        println!("launch script on Linux for missing game dir: {:?}", server::launch_plan(&c));
    }
}
