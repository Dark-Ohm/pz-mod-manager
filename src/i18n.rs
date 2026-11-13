//! Локализация RU/EN. Ключи словаря — русские строки из кода; `t("…")` возвращает перевод
//! для текущего языка, а при отсутствии перевода — исходный текст.
//! Строки с подстановками: `tf!("Найдено: {}", n)` — подстановки только `{}`, по порядку.
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ru,
    En,
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(l: Lang) {
    LANG.store(if l == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 { Lang::En } else { Lang::Ru }
}

/// Язык системы: ru* → русский, иначе английский
pub fn detect() -> Lang {
    let v = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.is_empty()).unwrap_or_default();
    if v.to_lowercase().starts_with("ru") { Lang::Ru } else { Lang::En }
}

/// "" — как в системе, "ru", "en"
pub fn apply_setting(setting: &str) {
    if cfg!(test) {
        // тесты сравнивают русские сообщения и идут параллельно — язык не переключаем
        return set(Lang::Ru);
    }
    set(match setting {
        "ru" => Lang::Ru,
        "en" => Lang::En,
        _ => detect(),
    });
}

static EN: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| crate::i18n_en::TABLE.iter().copied().collect());

pub fn en_table() -> &'static HashMap<&'static str, &'static str> {
    &EN
}

pub fn t<S: AsRef<str>>(s: S) -> String {
    t_in(get(), s.as_ref())
}

/// Перевод на указанный язык (без обращения к глобальной настройке)
pub fn t_in(l: Lang, s: &str) -> String {
    if l == Lang::Ru {
        return s.to_string();
    }
    EN.get(s).map_or_else(|| s.to_string(), |x| x.to_string())
}

/// Подставляет аргументы в `{}` переведённого шаблона
pub fn tf(template: &str, args: &[&dyn Display]) -> String {
    fill(&t(template), args)
}

pub fn fill(tpl: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(tpl.len() + 16);
    let mut it = args.iter();
    let mut rest = tpl;
    while let Some(p) = rest.find("{}") {
        out.push_str(&rest[..p]);
        match it.next() {
            Some(a) => out.push_str(&a.to_string()),
            None => out.push_str("{}"),
        }
        rest = &rest[p + 2..];
    }
    out.push_str(rest);
    out
}

#[macro_export]
macro_rules! tf {
    ($t:expr $(, $a:expr)* $(,)?) => {
        $crate::i18n::tf($t, &[$(&$a as &dyn std::fmt::Display),*])
    };
}
