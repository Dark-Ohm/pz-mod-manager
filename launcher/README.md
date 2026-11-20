# PZ Launcher — 10YL // Tactical Deployment

Тактический лаунчер модпака для Project Zomboid: показывает ревизию пака, проверяет
manifest, скачивает и устанавливает обновление через `pz-mod-manager`, запускает клиент
с отдельным `-cachedir` и `+connect`. Linux и Windows, один и тот же код.

## Сборка

```bash
# из корня репозитория pz-manager
cargo build --release -p pz-launcher
./target/release/pz-launcher
```

Готовые сборки лежат в [Releases](https://github.com/Dark-Ohm/pz-mod-manager/releases):
`pz-launcher-windows-x64.exe` и `pz-launcher-linux-x64`. GitHub Actions собирает их
вместе с менеджером. Положи лаунчер в ту же папку, что и `pz-mod-manager`: он ищет
менеджера рядом с собой.

Windows: `cargo build --release -p pz-launcher` (MSVC toolchain), готовый
`target\release\pz-launcher.exe`. Консоль в релизе не открывается (`windows_subsystem`).

## Что нужно на машине

1. **Project Zomboid** из Steam (лаунчер ищет стандартные пути):
   - Linux: `~/.local/share/Steam/steamapps/common/ProjectZomboid`
   - Windows: `%ProgramFiles(x86)%\Steam\steamapps\common\ProjectZomboid`
2. **pz-mod-manager** — лаунчер вызывает его `pack info` и `pack install`. Ищется в своей
   папке, в `../pz-manager/target/release/` и в `PATH`; либо укажи путь в настройках.
3. **.pzpack** — файл сборки. Путь задаётся в настройках.

Игра запускается с `-cachedir=<свой каталог>`, поэтому личная папка `~/Zomboid`
не затрагивается.

## Настройки и файлы

- Linux: `~/.config/pz-launcher/config.json`
- Windows: `%APPDATA%\pz-launcher\config.json`
- Скачанные паки: `~/.local/share/pz-launcher/packs/` (Windows: `%LOCALAPPDATA%`)
- Кэш клиента по умолчанию: `.../pz-launcher/client`

Поля: путь к менеджеру, `.pzpack`, URL манифеста, папка игры, cachedir, окружение
(`--env`), адрес сервера и порт, автопроверка, закрытие после запуска.

## Манифест

Лаунчер понимает плоский и вложенный вид; `revision` обязателен:

```json
{
  "pack_id": "1bee5251007c2644ad023bf8",
  "revision": 12,
  "changelog": "DLZ: UV-проверка 0.25 с, чистка кэша",
  "server": { "host": "37.142.197.188", "port": 27016 },
  "pack": {
    "url": "https://cdn.example.com/packs/rev-12.pzpack",
    "sha256": "…",
    "size": 6710886400
  }
}
```

`pack` можно заменить на `file` или на плоские `url`/`sha256`/`size`.
`server.connect` тоже принимается, если адрес задан одной строкой.

Обновление: манифест → сравнение `revision` с `pack info` → скачивание в
`packs/rev-N.pzpack` с проверкой `sha256` → `pack install` → повторное чтение пака.
Если в манифесте нет `url`, кнопка установки работает с уже выбранным `.pzpack`.

## Проверки

```bash
# из корня репозитория pz-manager
cargo test -p pz-launcher
```

Тесты покрывают разбор манифеста, выбор URL/адреса сервера, сравнение ревизий и
хелперы. Сетевой путь и запуск игры проверяются вручную: нужен реальный пак и игра.

## Ограничения

- Пак скачивается целиком; дельта-обновления (по объектам в облаке) — следующий шаг.
- Прогресс установки идёт строками `pack install`; если менеджер печатает JSON в конце,
  он попадёт в журнал как обычная строка.
- Steam-клиент должен быть запущен: лаунчер не логинится в Steam сам.
- `+connect` — штатный аргумент клиента PZ; при смене билда проверь его.
