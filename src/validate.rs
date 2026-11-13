//! Проверка ввода перед отправкой команд серверу (защита от инъекции второй команды).
use crate::i18n::t;

pub fn username(u: &str) -> Result<(), String> {
    let u = u.trim();
    if u.is_empty() {
        return Err(t("имя игрока пустое").into());
    }
    if u.len() > 64 {
        return Err(t("имя игрока слишком длинное").into());
    }
    if u.contains(['"', '\'', '\n', '\r', ';', '&', '|', '<', '>', '`', '$', '\\']) {
        return Err(t("имя игрока содержит запрещённые символы").into());
    }
    Ok(())
}

/// Готовая команда: одна строка, без управляющих символов
pub fn command(c: &str) -> Result<(), String> {
    let c = c.trim();
    if c.is_empty() {
        return Err(t("команда пустая").into());
    }
    if c.len() > 500 {
        return Err(t("команда слишком длинная").into());
    }
    if c.contains(['\n', '\r', '\0']) {
        return Err(t("команда должна быть одной строкой").into());
    }
    Ok(())
}

pub fn server_name(n: &str) -> Result<(), String> {
    let n = n.trim();
    if n.is_empty() {
        return Err(t("имя сервера пустое").into());
    }
    if n.contains(['/', '\\', '\0']) || n.contains("..") {
        return Err(t("имя сервера содержит запрещённые символы").into());
    }
    Ok(())
}
