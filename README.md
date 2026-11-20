# PZ Mod Manager

A manager for Project Zomboid servers, game clients and mods: one binary with a window for people and a JSON command line plus an MCP server for AI agents.

- **Mods**: load order with dependency checks, workshop downloads through SteamCMD, updates, profiles ("packs").
- **Environments**: several server + client pairs (for example `stable` and `lab`) on one game install, each with its own data folders.
- **Server**: start, stop, console, sandbox options, players, map, backups, network ports.
- **Sharing**: save a pack as a `.pzpack` file; the other player's manager installs it on their client with the same mod order and follows later revisions.

## Get it

Download a ready build from [Releases](https://github.com/Dark-Ohm/pz-mod-manager/releases): `pz-mod-manager-windows-x64.exe` or `pz-mod-manager-linux-x64`.

The same release carries `pz-launcher-windows-x64.exe` and `pz-launcher-linux-x64`: a small tactical launcher for a `.pzpack` mod pack. Put it in the same folder as the manager.

Or build from source: `scripts\install.bat` on Windows, `scripts/install.sh` on Linux (installs Rust and system packages, then builds `target/release/pz-mod-manager`).

## Use it

- Run without arguments for the window (Russian or English, follows the system language).
- Got a `.pzpack` from a friend: open it in Packs → "Open .pzpack…", or run `pz-mod-manager pack install <file.pzpack>`.
- For agents and scripts: `pz-mod-manager help`. Every command prints one JSON object. `pz-mod-manager mcp` serves the same commands as MCP tools over stdio.

[AGENTS.md](AGENTS.md) is the full guide: first run, concepts, typical flows, rules for agents.

---

Менеджер серверов, клиентов и модов Project Zomboid: окно для человека, команды с JSON-ответом и MCP-сервер для ИИ-агентов. Готовые сборки лежат в [Releases](https://github.com/Dark-Ohm/pz-mod-manager/releases), там же — лаунчер сборки `pz-launcher-windows-x64.exe` (и Linux-версия); положи его рядом с менеджером. Получил от друга файл `.pzpack` — открой его в «Сборки» → «Открыть .pzpack…».

## License

[MIT](LICENSE)
