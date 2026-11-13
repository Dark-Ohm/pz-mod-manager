//! Серверные команды Project Zomboid. Имена сверены со списком классов
//! `zombie/commands/serverCommands/*` в projectzomboid.jar (Build 42).
//! Синтаксис аргументов — по PZ wiki; шаблон подставляется в поле команды, чтобы его можно было проверить и поправить.

pub struct Tpl {
    pub label: &'static str,
    /// %U — имя выбранного игрока
    pub text: &'static str,
    pub needs_player: bool,
    pub hint: &'static str,
}

pub const PLAYER_TEMPLATES: &[Tpl] = &[
    Tpl { label: "Кик", text: "kickuser \"%U\" -r \"причина\"", needs_player: true, hint: "выкинуть игрока" },
    Tpl { label: "Бан", text: "banuser \"%U\" -r \"причина\"", needs_player: true, hint: "забанить аккаунт" },
    Tpl { label: "Снять бан", text: "unbanuser \"%U\"", needs_player: true, hint: "" },
    Tpl { label: "Уровень доступа", text: "setaccesslevel \"%U\" admin", needs_player: true, hint: "admin / moderator / overseer / gm / observer / none" },
    Tpl { label: "Выдать предмет", text: "additem \"%U\" \"Base.Axe\" 1", needs_player: true, hint: "Модуль.Предмет и количество" },
    Tpl { label: "Опыт", text: "addxp \"%U\" Aiming=100", needs_player: true, hint: "Перк=очки" },
    Tpl { label: "Годмод", text: "godmode \"%U\"", needs_player: true, hint: "переключает бессмертие" },
    Tpl { label: "Невидимость", text: "invisible \"%U\"", needs_player: true, hint: "" },
    Tpl { label: "NoClip", text: "noclip \"%U\"", needs_player: true, hint: "" },
    Tpl { label: "Телепорт к игроку", text: "teleport \"%U\" \"другой_игрок\"", needs_player: true, hint: "первый игрок переносится ко второму" },
];

/// Команды без выбора игрока: одним нажатием
pub const SERVER_COMMANDS: &[(&str, &str)] = &[
    ("Игроки", "players"),
    ("Сохранить мир", "save"),
    ("Перечитать опции", "reloadoptions"),
    ("Показать опции", "showoptions"),
    ("Вертолёт", "chopper"),
    ("Гроза", "thunder"),
    ("Начать дождь", "startrain"),
    ("Остановить дождь", "stoprain"),
    ("Остановить погоду", "stopweather"),
    ("Убрать зомби", "removezombies"),
];

pub fn servermsg(text: &str) -> String {
    format!("servermsg \"{}\"", text.replace('"', "'"))
}

/// Подставляет имя игрока в шаблон
pub fn fill(tpl: &Tpl, user: &str) -> Result<String, String> {
    if tpl.needs_player {
        crate::validate::username(user)?;
    }
    Ok(tpl.text.replace("%U", user.trim()))
}
