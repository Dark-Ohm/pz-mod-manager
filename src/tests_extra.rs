use crate::config::ServerCfg;
use crate::mods::{Mod, Source};
use crate::*;
use std::collections::HashMap;
use std::path::PathBuf;

fn m(id: &str, req: &[&str]) -> Mod {
    Mod { id: id.into(), name: id.into(), description: String::new(), requires: req.iter().map(|s| s.to_string()).collect(), workshop_id: None, url: None, path: PathBuf::new(), source: Source::ServerLocal }
}
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pzmm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn cfg_in(d: &PathBuf) -> ServerCfg {
    let mut c = ServerCfg::default();
    c.cache_dir = d.clone();
    c.name = "pack".into();
    c
}

#[test]
fn smart_sort_puts_dependencies_first_and_keeps_rest() {
    let inst: HashMap<String, Mod> = [m("a", &["c"]), m("b", &[]), m("c", &["d"]), m("d", &[]), m("e", &["ghost"])].into_iter().map(|x| (x.id.clone(), x)).collect();
    let order: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
    let (out, w) = sort::smart_sort(&order, &inst);
    assert!(w.is_empty());
    let pos = |x: &str| out.iter().position(|s| s == x).unwrap();
    assert!(pos("d") < pos("c") && pos("c") < pos("a"));
    assert_eq!(out.len(), 5);
    // независимый порядок не ломается
    assert!(pos("b") < pos("e"));
}

#[test]
fn smart_sort_reports_cycle() {
    let inst: HashMap<String, Mod> = [m("a", &["b"]), m("b", &["a"]), m("c", &[])].into_iter().map(|x| (x.id.clone(), x)).collect();
    let order: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let (out, w) = sort::smart_sort(&order, &inst);
    assert_eq!(out[0], "c");
    assert_eq!(w.len(), 1);
    assert_eq!(out.len(), 3);
}

const SB: &str = "SandboxVars = {\r\n    VERSION = 6,\r\n    Zombies = 4, -- note\r\n    Name = \"abc\",\r\n    ZombieLore = {\r\n        Speed = 2,\r\n    },\r\n}\r\n";

#[test]
fn sandbox_parse_and_update_keep_crlf_quotes_comments() {
    let f = sandbox::parse(SB);
    let get = |c: &str, k: &str| f.iter().find(|x| x.category == c && x.key == k).unwrap().clone();
    assert_eq!(get(sandbox::ROOT, "Zombies").value, "4");
    assert!(get(sandbox::ROOT, "Name").quoted);
    assert_eq!(get("ZombieLore", "Speed").value, "2");
    let (out, n) = sandbox::update(SB, &[(sandbox::ROOT.into(), "Zombies".into(), "2".into()), ("ZombieLore".into(), "Speed".into(), "3".into()), (sandbox::ROOT.into(), "Name".into(), "xyz".into()), (sandbox::ROOT.into(), "Speed".into(), "9".into())]);
    assert_eq!(n, 3, "Speed в корне не существует и не должен меняться");
    assert_eq!(out, "SandboxVars = {\r\n    VERSION = 6,\r\n    Zombies = 2, -- note\r\n    Name = \"xyz\",\r\n    ZombieLore = {\r\n        Speed = 3,\r\n    },\r\n}\r\n");
}

#[test]
fn template_apply_changes_only_existing_keys_and_protects_secrets() {
    let d = tmp("tpl");
    std::fs::create_dir_all(d.join("Server")).unwrap();
    std::fs::write(d.join("Server/pack.ini"), "Faction=true\r\nPlayerSafehouse=true\r\nRCONPassword=secret\r\nDefaultPort=16261\r\n").unwrap();
    std::fs::write(d.join("Server/pack_SandboxVars.lua"), SB).unwrap();
    let c = cfg_in(&d);
    let t = templates::all().into_iter().find(|t| t.id == "hardcore-survivor").unwrap();
    assert!(!t.ini.contains_key("RCONPassword"));
    assert!(templates::apply(&c, true, &t).is_err(), "при запущенном сервере отказ");
    let r = templates::apply(&c, false, &t).unwrap();
    assert!(r.ini_applied.contains(&"Faction".to_string()));
    let ini = std::fs::read_to_string(d.join("Server/pack.ini")).unwrap();
    assert_eq!(ini, "Faction=false\r\nPlayerSafehouse=false\r\nRCONPassword=secret\r\nDefaultPort=16261\r\n");
    let sb = std::fs::read_to_string(d.join("Server/pack_SandboxVars.lua")).unwrap();
    assert!(sb.contains("Zombies = 2") && sb.contains("Speed = 2") && sb.contains("\r\n"));
    assert!(std::fs::read_dir(d.join("Server")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains(".bak-")));
}

#[test]
fn backup_create_list_prune_restore() {
    let d = tmp("bk");
    std::fs::create_dir_all(d.join("Server")).unwrap();
    std::fs::create_dir_all(d.join("Saves/Multiplayer/pack")).unwrap();
    std::fs::write(d.join("Saves/Multiplayer/pack/map.bin"), "WORLD-1").unwrap();
    std::fs::write(d.join("Server/pack.ini"), "A=1\r\n").unwrap();
    let c = cfg_in(&d);
    let b = backup::create(&c, "t1", 10).unwrap();
    assert!(b.join("world/map.bin").exists() && b.join("pack.ini").exists());
    // портим мир и конфиг
    std::fs::write(d.join("Saves/Multiplayer/pack/map.bin"), "WORLD-2").unwrap();
    std::fs::write(d.join("Server/pack.ini"), "A=2\r\n").unwrap();
    let name = backup::list(&c)[0].name.clone();
    assert!(backup::restore(&c, &name, true).is_err(), "при запущенном сервере отказ");
    assert!(backup::restore(&c, "../x", false).is_err());
    let safety = backup::restore(&c, &name, false).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("Saves/Multiplayer/pack/map.bin")).unwrap(), "WORLD-1");
    assert_eq!(std::fs::read_to_string(d.join("Server/pack.ini")).unwrap(), "A=1\r\n");
    assert_eq!(std::fs::read_to_string(safety.join("world/map.bin")).unwrap(), "WORLD-2", "снимок хранит состояние до восстановления");
    assert!(!d.join("Saves/Multiplayer/pack.mm-old").exists() && !d.join("Saves/Multiplayer/pack.mm-new").exists());
    // ротация
    for i in 0..4 {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        backup::create(&c, &format!("p{i}"), 2).unwrap();
    }
    assert_eq!(backup::list(&c).len(), 2);
}

#[test]
fn players_parse_real_log_formats() {
    let user = "[11-11-26 23:01:27.789] 37.142.197.188 \"Peace_Thats\" fully connected (10724,10430,0).\n[11-11-26 23:18:54.553] 37.142.197.188 \"Peace_Thats\" disconnected player (10725,10419,0).\n[11-11-26 23:20:00.000] 1.2.3.4 \"Other\" fully connected (100,200,0).\n";
    let perk = "[11-11-26 23:15:22.346] [Peace_Thats][10718,10420,1][Level Changed][Reloading][2][Hours Survived: 12].\n";
    let conn = "[x] event=\"fully-connected\" message=\"\" guid=\"1\" ip=\"1.2.3.4\" steam-id=\"0\" role=\"admin\" username=\"Other\" connection-type=\"UDPRakNet\".\n";
    let p = players::parse_all(user, perk, conn);
    let a = p.iter().find(|x| x.name == "Peace_Thats").unwrap();
    assert!(!a.online);
    assert_eq!(a.pos, Some((10718, 10420, 1)));
    let o = p.iter().find(|x| x.name == "Other").unwrap();
    assert!(o.online && o.role == "admin" && o.pos == Some((100, 200, 0)));
}

#[test]
fn windows_and_linux_launch_plans() {
    let d = tmp("plan");
    std::fs::create_dir_all(d.join("jre64/bin")).unwrap();
    std::fs::write(d.join("jre64/bin/java.exe"), "").unwrap();
    std::fs::write(d.join("jre64/bin/java"), "").unwrap();
    std::fs::write(d.join("projectzomboid.jar"), "").unwrap();
    std::fs::write(d.join("start-server.sh"), "").unwrap();
    let mut c = ServerCfg::default();
    c.game_dir = d.clone();
    c.cache_dir = PathBuf::from("/data/Zomboid");
    c.name = "pack".into();
    c.xmx_mb = 12288;
    c.extra_args = "-adminpassword x".into();
    let linux = server::plan_for(&c, false).unwrap();
    assert!(linux.program.ends_with("start-server.sh"));
    assert_eq!(linux.args, ["-nosteam", "-cachedir=/data/Zomboid", "-servername", "pack", "-adminpassword", "x"]);
    if !cfg!(windows) {
        // путь к java.exe на Windows проверяется по файлу, который мы создали
        let win = server::plan_for(&c, true).unwrap();
        assert!(win.program.ends_with("java.exe"));
        assert!(win.args.contains(&"-Xmx12288m".to_string()) && win.args.contains(&"zombie.network.GameServer".to_string()));
        let gs = win.args.iter().position(|a| a == "zombie.network.GameServer").unwrap();
        assert_eq!(win.args[gs + 1], "-nosteam");
    }
    c.nosteam = false;
    assert!(!server::game_args(&c).contains(&"-nosteam".to_string()));
    std::fs::remove_file(d.join("start-server.sh")).unwrap();
    assert!(server::plan_for(&c, false).is_none());
}

#[test]
fn rcon_config_from_ini_and_args() {
    let d = tmp("rcon");
    let f = d.join("pack.ini");
    std::fs::write(&f, "RCONPort=27020\r\nRCONPassword=pw\r\n").unwrap();
    let r = rcon::from_ini(&f).unwrap();
    assert_eq!((r.port, r.password.as_str()), (27020, "pw"));
    std::fs::write(&f, "RCONPassword=\r\n").unwrap();
    assert!(rcon::from_ini(&f).is_none());
    let a = rcon::client_args(&d, &r, "players");
    assert!(a.contains(&"zombie.network.RCONClient".to_string()) && a.ends_with(&["-command".into(), "players".into()]));
}

#[test]
fn command_validation_blocks_injection() {
    assert!(validate::username("Peace_Thats").is_ok());
    assert!(validate::username("a\"; quit").is_err());
    assert!(validate::command("players").is_ok());
    assert!(validate::command("players\nquit").is_err());
    let t = &commands::PLAYER_TEMPLATES[0];
    assert_eq!(commands::fill(t, "Bob").unwrap(), "kickuser \"Bob\" -r \"причина\"");
    assert!(commands::fill(t, "x\"y").is_err());
    assert_eq!(commands::servermsg("hi \"all\""), "servermsg \"hi 'all'\"");
}

#[test]
fn map_view_roundtrip_and_zoom_keeps_cursor_point() {
    use eframe::egui::{pos2, vec2, Rect};
    let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
    let mut v = map::View::default();
    let p = pos2(400.0, 300.0);
    let (wx, wy) = v.to_world(rect, p);
    let back = v.to_screen(rect, wx, wy);
    assert!((back.x - p.x).abs() < 0.01 && (back.y - p.y).abs() < 0.01);
    v.zoom_at(rect, p, 2.0);
    let (wx2, wy2) = v.to_world(rect, p);
    assert!((wx - wx2).abs() < 0.01 && (wy - wy2).abs() < 0.01, "точка под курсором не сдвигается при масштабе");
    assert!(v.grid_step() % map::CELL == 0);
}

#[test]
fn ini_set_existing_keeps_crlf() {
    let d = tmp("ini");
    let f = d.join("a.ini");
    std::fs::write(&f, "A=1\r\nB=2\r\n").unwrap();
    let (applied, _) = ini::set_existing(&f, &[("A".into(), "9".into()), ("Z".into(), "1".into())]).unwrap();
    assert_eq!(applied, ["A"]);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "A=9\r\nB=2\r\n");
}

/// Только чтение реальной папки: PZ_WIN_DIR=<Zomboid> PZ_WIN_NAME=pack cargo test real_folder -- --ignored --nocapture
#[test]
#[ignore]
fn real_folder_readonly() {
    let dir = PathBuf::from(std::env::var("PZ_WIN_DIR").unwrap());
    let c = cfg_in(&dir);
    let pl = players::load(&dir);
    println!("игроков в логах: {} -> {:?}", pl.len(), pl.iter().map(|p| (&p.name, p.online, p.pos)).collect::<Vec<_>>());
    let sb = std::fs::read_to_string(sandbox::path(&c)).unwrap();
    let f = sandbox::parse(&sb);
    println!("полей песочницы: {}, категорий: {}", f.len(), f.iter().map(|x| x.category.clone()).collect::<std::collections::BTreeSet<_>>().len());
    // шаблон на КОПИИ
    let d = tmp("realcopy");
    std::fs::create_dir_all(d.join("Server")).unwrap();
    std::fs::copy(c.ini(), d.join("Server/pack.ini")).unwrap();
    std::fs::copy(sandbox::path(&c), d.join("Server/pack_SandboxVars.lua")).unwrap();
    let c2 = cfg_in(&d);
    let t = templates::all().into_iter().find(|t| t.id == "hardcore-survivor").unwrap();
    let r = templates::apply(&c2, false, &t).unwrap();
    println!("шаблон на копии: ini {:?}, песочница {}/{}", r.ini_applied, r.sandbox_applied, r.sandbox_total);
    let orig_ini = std::fs::read_to_string(c.ini()).unwrap();
    let new_ini = std::fs::read_to_string(d.join("Server/pack.ini")).unwrap();
    assert_eq!(orig_ini.lines().count(), new_ini.lines().count(), "число строк ini не изменилось");
    assert_eq!(orig_ini.matches("\r\n").count(), new_ini.matches("\r\n").count(), "CRLF сохранён");
    assert_eq!(ini::get(&orig_ini, "RCONPassword"), ini::get(&new_ini, "RCONPassword"));
    let new_sb = std::fs::read_to_string(d.join("Server/pack_SandboxVars.lua")).unwrap();
    assert_eq!(sb.matches("\r\n").count(), new_sb.matches("\r\n").count());
    assert_eq!(sb.lines().count(), new_sb.lines().count());
}

#[test]
fn file_uri_for_linux_and_windows_paths() {
    assert_eq!(platform::file_uri("/home/a/map.png"), "file:///home/a/map.png");
    assert_eq!(platform::file_uri("C:\\Users\\a\\map.png"), "file:///C:/Users/a/map.png");
}

#[test]
fn steamcmd_source_matches_platform() {
    if cfg!(windows) {
        assert!(platform::steamcmd_url().ends_with("steamcmd.zip") && platform::steamcmd_bin() == "steamcmd.exe");
    } else {
        assert!(platform::steamcmd_url().ends_with("steamcmd_linux.tar.gz") && platform::steamcmd_bin() == "steamcmd.sh");
    }
}

// ---------- обновления локальных модов ----------

const CHANGELOG_HTML: &str = r#"
<div class="detailBox workshopAnnouncement noFooter changeLogCtn">
  <div class="changelog headline">
      Update: 6 Jan, 2022 @ 1:59am						</div>
  <div style="clear: inline-end"></div>
  <p id="1641463158">v1.8.2<br>- Fixed ammo &amp; battery <b>counter</b>.</p>
</div>
<div class="detailBox workshopAnnouncement noFooter changeLogCtn">
  <div class="changelog headline">Update: 27 Dec, 2021 @ 9:11am</div>
  <p id="1640599899">v1.8.1<br>- Old fix<br>- Another</p>
</div>"#;

#[test]
fn changelog_parse_keeps_timestamps_and_cleans_html() {
    let c = workshop_api::parse_changelog(CHANGELOG_HTML);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].ts, Some(1641463158));
    assert_eq!(c[0].when, "Update: 6 Jan, 2022 @ 1:59am");
    assert_eq!(c[0].text, "v1.8.2\n- Fixed ammo & battery counter.");
    assert_eq!(c[1].text, "v1.8.1\n- Old fix\n- Another");
}

#[test]
fn details_parse_handles_string_numbers_and_missing_items() {
    let j = r#"{"response":{"result":1,"resultcount":2,"publishedfiledetails":[
      {"publishedfileid":"1","result":1,"title":"A","time_updated":1700000000,"time_created":1600000000,"file_size":"12345"},
      {"publishedfileid":"2","result":9}]}}"#;
    let r = workshop_api::parse_details(j).unwrap();
    assert_eq!(r[0], workshop_api::Remote { id: "1".into(), title: "A".into(), time_updated: 1700000000, time_created: 1600000000, size: 12345, ok: true });
    assert!(!r[1].ok);
    assert!(workshop_api::parse_details("not json").is_err());
}

#[test]
fn status_rules() {
    use tracking::{Item, Status, status};
    use workshop_api::Remote;
    let r = |t: u64, ok: bool| Remote { id: "1".into(), time_updated: t, ok, ..Default::default() };
    let it = |b: Option<u64>| Item { baseline: b, ..Default::default() };
    assert_eq!(status(None, None), Status::NoBaseline);
    assert_eq!(status(Some(&it(Some(5))), None), Status::Unknown);
    assert_eq!(status(Some(&it(Some(5))), Some(&r(5, true))), Status::UpToDate);
    assert_eq!(status(Some(&it(Some(5))), Some(&r(9, true))), Status::Update(9));
    assert_eq!(status(Some(&it(None)), Some(&r(9, true))), Status::NoBaseline);
    assert_eq!(status(Some(&it(Some(5))), Some(&r(9, false))), Status::Gone);
    let mut skipped = it(Some(5));
    skipped.skipped_until = Some(9);
    assert_eq!(status(Some(&skipped), Some(&r(9, true))), Status::Skipped(9));
    assert_eq!(status(Some(&skipped), Some(&r(12, true))), Status::Update(12), "новое обновление после пропущенного снова считается");
    let mut frozen = it(Some(5));
    frozen.frozen = true;
    assert_eq!(status(Some(&frozen), Some(&r(99, true))), Status::Frozen);
}

#[test]
fn link_resolution_priority_and_manifest_import() {
    use std::collections::HashMap;
    let mut st = tracking::ServerTrack::default();
    let mut local = m("ModA", &[]);
    local.url = Some("https://steamcommunity.com/sharedfiles/filedetails/?id=111&x=1".into());
    let scan: HashMap<String, String> = [("ModA".to_string(), "333".to_string())].into();
    assert_eq!(tracking::workshop_of(&local, &st, &scan).as_deref(), Some("111"), "url= из mod.info раньше найденного при скане");
    st.links.insert("ModA".into(), "222".into());
    assert_eq!(tracking::workshop_of(&local, &st, &scan).as_deref(), Some("222"), "ручная привязка главнее всего");
    let mut gh = m("ModB", &[]);
    gh.url = Some("https://github.com/x/y".into());
    assert_eq!(tracking::workshop_of(&gh, &st, &HashMap::new()), None, "ссылка не на Steam не считается");
    assert_eq!(tracking::id_from_url("https://steamcommunity.com/sharedfiles/filedetails/?id=3759676725"), Some("3759676725".into()));
    assert_eq!(tracking::id_from_url(" 123 "), Some("123".into()));
    assert_eq!(tracking::id_from_url("hello"), None);
    let manifest = "\u{feff}[{\"workshop_id\":\"3832815095\",\"files\":16,\"mods\":[{\"path\":\"x\",\"name\":\"Keep\",\"id\":\"KeepSkillsOnDeath\"},{\"id\":\"KeepSkillsOnDeath\"}]},{\"workshop_id\":42,\"mods\":[{\"id\":\"Z\"}]}]";
    let pairs = tracking::import_manifest(manifest);
    assert!(pairs.contains(&("KeepSkillsOnDeath".into(), "3832815095".into())) && pairs.contains(&("Z".into(), "42".into())));
}

#[test]
fn update_install_keeps_old_version_and_rollback_restores_it() {
    let d = tmp("upd");
    let mods = d.join("Zomboid/mods");
    let cache = d.join("Zomboid");
    std::fs::create_dir_all(mods.join("MyMod/42")).unwrap();
    std::fs::write(mods.join("MyMod/42/mod.info"), "id=MyMod\nname=OLD\n").unwrap();
    std::fs::write(mods.join("MyMod/extra_old.txt"), "old only").unwrap();
    std::fs::create_dir_all(mods.join("Untouched")).unwrap();
    std::fs::write(mods.join("Untouched/keep.txt"), "x").unwrap();
    // «скачанный» элемент воркшопа
    let item = d.join("item/999");
    std::fs::create_dir_all(item.join("mods/MyMod/42")).unwrap();
    std::fs::write(item.join("mods/MyMod/42/mod.info"), "id=MyMod\nname=NEW\n").unwrap();
    let done = modupdate::install(&item, "999", "server", &mods, &cache, Some(100)).unwrap();
    assert_eq!(done, ["MyMod"]);
    assert!(std::fs::read_to_string(mods.join("MyMod/42/mod.info")).unwrap().contains("NEW"));
    assert!(!mods.join("MyMod/extra_old.txt").exists(), "папка заменена целиком");
    assert!(mods.join("Untouched/keep.txt").exists(), "чужие моды не тронуты");
    assert!(!mods.join("MyMod.mm-new").exists());
    let bs = modupdate::backups_for(&cache, "999");
    assert_eq!(bs.len(), 1);
    assert!(bs[0].0.join("MyMod/extra_old.txt").exists(), "старая версия сохранена");
    assert_eq!(bs[0].1.baseline_before, Some(100));
    // откат
    let old_baseline = modupdate::rollback(&cache, "999", Some(200)).unwrap();
    assert_eq!(old_baseline, Some(100));
    assert!(std::fs::read_to_string(mods.join("MyMod/42/mod.info")).unwrap().contains("OLD"));
    assert!(mods.join("MyMod/extra_old.txt").exists());
    // новая версия тоже не потеряна: ушла в бэкап отката
    let again = modupdate::backups_for(&cache, "999");
    assert_eq!(again.len(), 1);
    assert!(std::fs::read_to_string(again[0].0.join("MyMod/42/mod.info")).unwrap().contains("NEW"));
    assert_eq!(again[0].1.baseline_before, Some(200));
    // нечего ставить
    assert!(modupdate::install(&d.join("empty"), "1", "server", &mods, &cache, None).is_err());
}

#[test]
fn local_mods_group_by_workshop_item() {
    use std::collections::HashMap;
    let (a, b, c) = (m("A", &[]), m("B", &[]), m("C", &[]));
    let map: HashMap<&str, &str> = [("A", "1"), ("B", "1")].into();
    let (g, none) = tracking::group(&[&a, &b, &c], |x| map.get(x.id.as_str()).map(|s| s.to_string()));
    assert_eq!(g["1"], ["A", "B"]);
    assert_eq!(none, ["C"]);
}

/// Живой запрос в Steam: cargo test live_steam -- --ignored --nocapture
#[test]
#[ignore]
fn live_steam_details_and_changelog() {
    let ids = vec!["2619072426".to_string(), "3806764280".to_string(), "1".to_string()];
    let r = workshop_api::fetch_details(&ids).unwrap();
    println!("{:?}", r.iter().map(|x| (&x.id, &x.title, x.time_updated, x.ok)).collect::<Vec<_>>());
    assert!(r.iter().find(|x| x.id == "2619072426").unwrap().ok);
    let wc = r.iter().find(|x| x.id == "2619072426").unwrap();
    assert_eq!(wc.time_updated, 1641463158);
    // страница изменений ограничивает частоту (429): один повтор через минуту
    let cl = match workshop_api::fetch_changelog("2619072426") {
        Ok(c) => c,
        Err(e) if format!("{e}").contains("429") => {
            println!("429, жду 70 с…");
            std::thread::sleep(std::time::Duration::from_secs(70));
            workshop_api::fetch_changelog("2619072426").unwrap()
        }
        Err(e) => panic!("{e:#}"),
    };
    println!("changelog entries: {}, first: {:?}", cl.len(), cl.first().map(|c| (&c.when, c.ts)));
    assert!(!cl.is_empty());
    assert_eq!(cl[0].ts, Some(wc.time_updated), "последняя запись changelog совпадает с time_updated");
    assert!(workshop_api::fetch_changelog("12a").is_err());
}



#[test]
fn matching_query_cleanup_and_scoring() {
    assert_eq!(matching::clean_query("Fists of Survival [B42] (v1.2)"), "Fists of Survival");
    assert_eq!(matching::clean_query("[B42]"), "[B42]", "если после чистки пусто — оставляем как есть");
    assert_eq!(matching::split_camel("ViewpointAutoView"), "Viewpoint Auto View");
    assert_eq!(matching::split_camel("better-auto_mechanics"), "better auto mechanics");
    assert_eq!(matching::score("Viewpoint", "Viewpoint"), 1.0);
    assert_eq!(matching::score("Fists of survival", "Fists of Survival [B42]"), 1.0, "метка билда не мешает");
    assert!(matching::score("Viewpoint", "Viewpoint Advanced Movement") >= 0.5 && matching::score("Viewpoint", "Viewpoint Advanced Movement") < 1.0);
    assert!(matching::score("Viewpoint", "Zombie Backpacks") < 0.2);
    assert_eq!(matching::score("", "x"), 0.0);
}

#[test]
fn matching_parse_search_sorts_by_score() {
    let j = r#"{"response":{"total":3,"publishedfiledetails":[
      {"publishedfileid":"2","title":"Viewpoint Advanced Movement"},
      {"publishedfileid":"1","title":"Viewpoint"},
      {"publishedfileid":"3","title":"Other"}]}}"#;
    let c = matching::parse_search(j, "Viewpoint").unwrap();
    assert_eq!(c[0].id, "1");
    assert_eq!(c[0].score, 1.0);
    assert_eq!(c.last().unwrap().id, "3");
    assert!(matching::parse_search(r#"{"response":{}}"#, "x").unwrap().is_empty());
}

#[test]
fn wrong_link_is_caught_before_install() {
    let found: Vec<String> = ["Other".to_string()].into();
    assert!(!modupdate::item_matches(&found, &["MyMod".to_string()]));
    assert!(modupdate::item_matches(&["MyMod".to_string(), "Other".to_string()], &["MyMod".to_string()]));
    assert!(modupdate::item_matches(&found, &[]));
}

#[test]
fn ignored_mods_roundtrip_in_tracking_json() {
    let mut t = tracking::Tracking::default();
    t.server("k").ignored.insert("MyPatch".into());
    t.server("k").links.insert("A".into(), "1".into());
    let j = serde_json::to_string(&t).unwrap();
    let back: tracking::Tracking = serde_json::from_str(&j).unwrap();
    assert!(back.servers["k"].ignored.contains("MyPatch") && back.servers["k"].links["A"] == "1");
    // старый файл без поля ignored читается
    let old: tracking::Tracking = serde_json::from_str(r#"{"servers":{"k":{"items":{},"links":{"A":"1"}}}}"#).unwrap();
    assert!(old.servers["k"].ignored.is_empty());
}

#[test]
fn changelog_cache_roundtrip_and_expiry() {
    let id = format!("9{}", std::process::id());
    assert!(workshop_api::cached_changelog(&id).is_none());
    let list = workshop_api::parse_changelog(CHANGELOG_HTML);
    workshop_api::store_changelog(&id, &list);
    assert_eq!(workshop_api::cached_changelog(&id).unwrap(), list, "из свежего кэша читается то же самое");
    let far = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + workshop_api::CHANGELOG_TTL + 10;
    assert!(workshop_api::cached_changelog_at(&id, far).is_none(), "устаревший кэш не используется");
}

// ---------- сборки модов ----------

#[test]
fn id_list_parsing_handles_ini_lines_links_and_spaces() {
    let p = packs::parse_id_list("Mods=\\ZombieBuddy;\\True Damage;\\Viewpoint;\r\nWorkshopItems=111;222\n");
    assert_eq!(p.mods, ["ZombieBuddy", "True Damage", "Viewpoint"]);
    assert_eq!(p.workshop, ["111", "222"]);
    let p = packs::parse_id_list("ModA\nModB, ModC\nhttps://steamcommunity.com/sharedfiles/filedetails/?id=3759676725\n3809306528\nModA\n");
    assert_eq!(p.mods, ["ModA", "ModB", "ModC"], "дубли убираются, порядок сохраняется");
    assert_eq!(p.workshop, ["3759676725", "3809306528"]);
    let p = packs::parse_id_list("Name With Space\nAnother One");
    assert_eq!(p.mods, ["Name With Space", "Another One"], "без разделителей строка — один id");
    assert_eq!(packs::parse_id_list("  \n\n"), packs::Parsed::default());
}

#[test]
fn pack_diff_reports_added_removed_and_moved() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let d = packs::diff(&s(&["a", "b", "c", "d"]), &s(&["a", "c", "d", "b", "e"]));
    assert_eq!(d.added, ["e"]);
    assert!(d.removed.is_empty());
    assert_eq!(d.moved, ["b"], "сдвинут один мод, остальные сохранили относительный порядок");
    let d2 = packs::diff(&s(&["a", "b"]), &s(&["b"]));
    assert_eq!((d2.removed, d2.added, d2.moved), (s(&["a"]), vec![], vec![]));
    assert_eq!(packs::diff(&s(&["a", "b"]), &s(&["a", "b"])), packs::Diff::default());
}

#[test]
fn pack_file_roundtrip_and_future_format_rejected() {
    let mut inst: HashMap<String, Mod> = HashMap::new();
    let mut a = m("A", &[]);
    a.path = PathBuf::from("/srv/Zomboid/mods/AFolder/42");
    a.workshop_id = Some("77".into());
    inst.insert("A".into(), a);
    let p = config::Profile { name: "Стабильная".into(), enabled: vec!["A".into(), "Ghost".into()], notes: "n".into(), stable: true, ..Default::default() };
    let roots = vec![PathBuf::from("/srv/Zomboid/mods")];
    let f = packs::to_file(&p, &inst, &|m| m.workshop_id.clone(), &roots, None);
    assert_eq!(f.mods[0].folder.as_deref(), Some("AFolder"));
    assert_eq!(f.mods[0].workshop_id.as_deref(), Some("77"));
    assert_eq!(f.mods[1].name, "", "мод не на диске сохраняется только по id");
    let back = packs::from_json(&packs::to_json(&f)).unwrap();
    assert_eq!((back.name.as_str(), back.mods.len(), back.notes.as_str()), ("Стабильная", 2, "n"));
    assert!(packs::from_json("{\"format\":999,\"name\":\"x\",\"mods\":[]}").is_err());
    assert!(packs::from_json("not json").is_err());
}

#[test]
fn player_zip_contains_local_mods_modlist_and_install_notes() {
    let d = tmp("zip");
    let mods = d.join("Zomboid/mods");
    std::fs::create_dir_all(mods.join("LocalMod/42")).unwrap();
    std::fs::write(mods.join("LocalMod/42/mod.info"), "id=LocalMod\n").unwrap();
    std::fs::write(mods.join("LocalMod/readme.txt"), "hello").unwrap();
    std::fs::create_dir_all(mods.join("NotInPack")).unwrap();
    std::fs::write(mods.join("NotInPack/x.txt"), "x").unwrap();
    let mut local = m("LocalMod", &[]);
    local.path = mods.join("LocalMod/42");
    let mut ws = m("SteamMod", &[]);
    ws.source = Source::ClientWorkshop;
    ws.workshop_id = Some("555".into());
    ws.path = PathBuf::from("/steam/content/108600/555/mods/SteamMod");
    let inst: HashMap<String, Mod> = [local, ws].into_iter().map(|x| (x.id.clone(), x)).collect();
    let p = config::Profile { name: "Пак".into(), enabled: vec!["LocalMod".into(), "SteamMod".into(), "Gone".into()], ..Default::default() };
    let dest = d.join("pack.zip");
    let rep = packs::build_player_zip(&dest, &p, &inst, &[mods.clone()], &|m| m.workshop_id.clone(), true, None).unwrap();
    assert_eq!((rep.folders, rep.files), (1, 2));
    assert_eq!(rep.subscribe, ["555"]);
    assert_eq!(rep.missing, ["Gone"]);
    let mut z = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
    assert!(names.contains(&"mods/LocalMod/42/mod.info".to_string()) && names.contains(&"mods/LocalMod/readme.txt".to_string()));
    assert!(!names.iter().any(|n| n.contains("NotInPack")), "чужие моды не попадают в пакет");
    assert!(names.contains(&"modlist.txt".to_string()) && names.contains(&"INSTALL.txt".to_string()) && names.contains(&"pack.json".to_string()));
    let read = |z: &mut zip::ZipArchive<std::fs::File>, n: &str| {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut z.by_name(n).unwrap(), &mut s).unwrap();
        s
    };
    assert_eq!(read(&mut z, "modlist.txt"), "Mods=\\LocalMod;\\SteamMod;\\Gone\r\n");
    let inst_en = read(&mut z, "INSTALL.txt");
    assert!(inst_en.contains("HOW TO INSTALL") && inst_en.contains("?id=555"));
    let ru = packs::build_player_zip(&d.join("ru.zip"), &p, &inst, &[mods], &|m| m.workshop_id.clone(), false, None).unwrap();
    assert_eq!(ru.folders, 1);
    let mut z2 = zip::ZipArchive::new(std::fs::File::open(d.join("ru.zip")).unwrap()).unwrap();
    assert!(read(&mut z2, "INSTALL.txt").contains("КАК УСТАНОВИТЬ"));
}

// ---------- общий файл сборки .pzpack ----------

#[test]
fn pzpack_revision_roundtrip_and_extract() {
    let d = std::env::temp_dir().join(format!("pzmm-share-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let mods = d.join("mods");
    std::fs::create_dir_all(mods.join("LocalMod/42")).unwrap();
    std::fs::write(mods.join("LocalMod/42/mod.info"), "id=LocalMod\n").unwrap();
    let mut local = m("LocalMod", &[]);
    local.path = mods.join("LocalMod/42");
    let inst: HashMap<String, Mod> = [local].into_iter().map(|x| (x.id.clone(), x)).collect();
    let mut p = config::Profile { name: "Пак".into(), enabled: vec!["LocalMod".into(), "Steam".into()], ..Default::default() };
    let info = Some(packs::PackServer { host: "example.org".into(), port: "16261".into(), udp_port: "16262".into(), name: "s".into() });
    // первая запись: id и ревизия 1; без изменений ревизия не растёт
    assert!(share::bump_if_changed(&mut p, &info));
    assert_eq!((p.revision, p.pack_id.len() > 8), (1, true));
    let id = p.pack_id.clone();
    assert!(!share::bump_if_changed(&mut p, &info));
    // порядок поменялся: ревизия 2, id тот же
    p.enabled.reverse();
    assert!(share::bump_if_changed(&mut p, &info));
    assert_eq!((p.revision, p.pack_id.clone()), (2, id.clone()));
    // другой порт тоже считается изменением
    let info2 = Some(packs::PackServer { port: "16271".into(), ..info.clone().unwrap() });
    assert!(share::bump_if_changed(&mut p, &info2));
    assert_eq!(p.revision, 3);

    let dest = d.join("out/pack.pzpack");
    share::write_pack(&dest, &p, &inst, &[mods.clone()], &|_| None, info2.clone()).unwrap();
    assert!(!d.join("out/pack.pzpack.tmp").exists(), "временный файл переименован");
    let f = share::read_pack(&dest).unwrap();
    assert_eq!((f.pack_id.as_str(), f.revision), (id.as_str(), 3));
    assert_eq!(f.mods.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["Steam", "LocalMod"]);
    assert_eq!(f.server, info2);
    let (pid, rev, _) = share::peek(&dest).unwrap();
    assert_eq!((pid, rev), (id, 3));

    // получатель: локальный мод распаковывается в папку модов клиента
    let client = d.join("client/mods");
    assert_eq!(share::extract_local(&dest, &client).unwrap(), 1);
    assert!(client.join("LocalMod/42/mod.info").is_file());
    // обычный json без zip тоже читается
    let js = d.join("plain.json");
    std::fs::write(&js, packs::to_json(&f)).unwrap();
    assert_eq!(share::read_pack(&js).unwrap().revision, 3);
    assert_eq!(share::extract_local(&js, &client).unwrap(), 0);
    let dd = share::describe(&["LocalMod".into(), "Gone".into()], &f);
    assert_eq!((dd.added, dd.removed), (vec!["Steam".to_string()], vec!["Gone".to_string()]));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn pzpack_extract_rejects_path_traversal() {
    use std::io::Write;
    let d = std::env::temp_dir().join(format!("pzmm-slip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let zp = d.join("evil.pzpack");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        zw.start_file("mods/../../escaped.txt", o).unwrap();
        zw.write_all(b"x").unwrap();
        zw.start_file("mods/Ok/a.txt", o).unwrap();
        zw.write_all(b"y").unwrap();
        zw.finish().unwrap();
    }
    let dest = d.join("a/b/mods");
    assert_eq!(share::extract_local(&zp, &dest).unwrap(), 1);
    assert!(dest.join("Ok/a.txt").is_file());
    assert!(!d.join("a/escaped.txt").exists() && !d.join("escaped.txt").exists());
    let _ = std::fs::remove_dir_all(&d);
}

// ---------- локализация ----------

fn source_files() -> Vec<(String, String)> {
    std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .filter(|(n, _)| !matches!(n.as_str(), "tests_extra.rs" | "i18n.rs" | "i18n_en.rs"))
        .collect()
}

fn unescape(x: &str) -> String {
    x.replace("\\\"", "\"").replace("\\n", "\n").replace("\\'", "'").replace("\\\\", "\\")
}

#[test]
fn every_ui_string_has_an_english_translation() {
    let cyr = regex_lite_cyr();
    let call = |s: &str| -> Vec<String> {
        let mut out = vec![];
        let b: Vec<char> = s.chars().collect();
        // вызовы t("…") / tf!("…") / i18n::t("…") с литералом первым аргументом
        let text: String = b.iter().collect();
        for pat in ["t(\"", "tf!(\"", "i18n::t(\""] {
            let mut from = 0;
            while let Some(p) = text[from..].find(pat) {
                let start = from + p;
                let prev = text[..start].chars().last();
                from = start + pat.len();
                if pat == "t(\"" && prev.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':') {
                    continue;
                }
                let body_start = start + pat.len();
                let rest = &text[body_start..];
                let mut end = 0;
                let rb = rest.as_bytes();
                while end < rb.len() && rb[end] != b'"' {
                    end += if rb[end] == b'\\' { 2 } else { 1 };
                }
                out.push(unescape(&rest[..end.min(rest.len())]));
            }
        }
        out
    };
    let table = i18n::en_table();
    let mut missing = vec![];
    for (name, src) in source_files() {
        let src = if name == "main.rs" { src.split("#[cfg(test)]\nmod tests {").next().unwrap().to_string() } else { src };
        for k in call(&src) {
            if cyr(&k) && !table.contains_key(k.as_str()) {
                missing.push(format!("{name}: {k}"));
            }
        }
    }
    // метки и подсказки из таблиц команд
    for (label, _) in commands::SERVER_COMMANDS {
        assert!(table.contains_key(label), "нет перевода команды: {label}");
    }
    for tp in commands::PLAYER_TEMPLATES {
        assert!(!cyr(tp.label) || table.contains_key(tp.label), "нет перевода: {}", tp.label);
        if cyr(tp.hint) {
            assert!(table.contains_key(tp.hint), "нет перевода подсказки: {}", tp.hint);
        }
    }
    assert!(table.contains_key(sandbox::ROOT));
    assert!(missing.is_empty(), "строки без английского перевода:\n{}", missing.join("\n"));
}

fn regex_lite_cyr() -> impl Fn(&str) -> bool {
    |s: &str| s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

#[test]
fn english_translations_keep_placeholders_and_have_no_duplicates() {
    let mut seen = std::collections::HashSet::new();
    for (ru, en) in i18n_en::TABLE {
        assert!(seen.insert(*ru), "дубликат ключа: {ru}");
        assert_eq!(ru.matches("{}").count(), en.matches("{}").count(), "число {{}} не совпадает: {ru} / {en}");
        assert!(!en.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)), "в переводе осталась кириллица: {en}");
        assert!(!en.trim().is_empty());
    }
}

#[test]
fn translation_lookup_and_placeholder_order() {
    use i18n::{Lang, t_in};
    assert_eq!(t_in(Lang::En, "Моды"), "Mods");
    assert_eq!(t_in(Lang::Ru, "Моды"), "Моды");
    assert_eq!(t_in(Lang::En, "строки нет в словаре"), "строки нет в словаре", "без перевода показываем исходник");
    let en = t_in(Lang::En, "не хватает места: свободно {}, нужно {}");
    assert_eq!(i18n::fill(&en, &[&"2 GB", &"5 GB"]), "not enough space: 2 GB free, 5 GB needed");
    assert_eq!(i18n::fill("a {} b {} c {}", &[&1, &2]), "a 1 b 2 c {}", "лишний плейсхолдер не падает");
}

#[test]
fn steamcmd_log_lines_lose_terminal_color_codes() {
    assert_eq!(crate::steam::strip_ansi("\u{1b}[0mWaiting for user info...\u{1b}[0mOK"), "Waiting for user info...OK");
    assert_eq!(crate::steam::strip_ansi("\u{1b}[1;33mWarn\u{1b}[0m "), "Warn");
    assert_eq!(crate::steam::strip_ansi("plain"), "plain");
}
