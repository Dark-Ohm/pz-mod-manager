# PZ Mod Manager: guide for agents

The manager handles Project Zomboid servers, game clients and mods. It is one binary with three modes:

| Mode | Command | For |
|---|---|---|
| GUI | `pz-mod-manager` (no arguments) | a person |
| CLI | `pz-mod-manager <group> <action> …` | agents and scripts working in a shell |
| MCP | `pz-mod-manager mcp` | agents that use MCP tools (stdio JSON-RPC) |

All three modes share the same state, so a person, the GUI and several agents can work at the same time:
- `config.toml` holds environments, profiles and settings. Writes are locked and atomic, and the GUI reloads the file when it changes. Run `pz-mod-manager config show` to see where it is.
- The manager's data folder holds pid files, logs and the server console FIFO.
- A running server or client is found by its `-cachedir=`, so the manager also sees one started outside it.

The binary is `target/release/pz-mod-manager`. Nothing in this folder is tied to a particular machine: paths come from the config.

## First run on a new machine

1. **Get the binary.** Ready builds for Windows and Linux are on the Releases page of https://github.com/Dark-Ohm/pz-mod-manager (a single file, nothing to install). To build from source instead:
   - Linux: `scripts/install.sh` installs the system packages and Rust, then builds. If Rust is already installed, `scripts/build.sh` is enough.
   - Windows: `scripts\install.bat`, or `scripts\build.bat`.
2. **Look around.**
   ```bash
   pz-mod-manager config show
   pz-mod-manager env list
   ```
   A fresh config has one environment, `default`, whose server cachedir is `~/Zomboid`. That is the game's default user folder, with the person's own saves and servers. Commands that change server files or processes refuse to work there: that is deliberate, see the rules below.
3. **Ask the person where the game and the data should live.** Do not guess the folders. Then set them up:
   ```bash
   # game installs (defaults are the standard Steam locations)
   pz-mod-manager config set --client-game-dir "<…>/steamapps/common/ProjectZomboid"
   pz-mod-manager env add stable --cache-dir <data>/pz-server --client-cache-dir <data>/pz-client \
       --name servertest --game-dir "<…>/steamapps/common/Project Zomboid Dedicated Server"
   pz-mod-manager env add lab --copy-from stable --cache-dir <data>/pz-lab-server \
       --client-cache-dir <data>/pz-lab-client --name labtest
   pz-mod-manager env use stable
   pz-mod-manager env remove default            # optional: drop the default entry (files are not touched)
   ```
   On Windows the server runs from the game folder itself (`--game-dir …\ProjectZomboid`), and commands go through RCON, so set `RCONPassword` in the ini after the first start.
4. **First server start.** It creates the ini and needs an admin password: ask the person for it, or generate one and tell them.
   ```bash
   pz-mod-manager server start lab --admin-password <pw> --wait
   pz-mod-manager server stop lab
   ```
   To run two environments at the same time, give them different ports:
   ```bash
   pz-mod-manager ini set DefaultPort=16271 UDPPort=16272 --env lab
   ```

## Concepts

- **Environment (`env`)**: one server and one game client. All environments share one game install, but each has its own data folders (server and client `-cachedir`). A common pair is `stable` (players use it) and `lab` (experiments). Most commands take `--env <id>`; without it they use the active environment (`env use`).
- **Profile**: an ordered list of mod ids, shared by all environments. `ini apply` writes a profile into an environment's server ini (`Mods=` / `WorkshopItems=`).
- **No-steam mode** (`nosteam=true` by default): mods come from `<cachedir>/mods`, not from the Steam workshop.

## Output contract

The CLI always prints a single JSON object to stdout:
- success: `{"ok":true,"result":…}`, exit code 0;
- failure: `{"ok":false,"error":"…"}`, exit code 1.

MCP tool results carry the same JSON as text, with `isError` set on failure. Messages are in English; set `PZMM_LANG=ru` for Russian.

## Discovering commands

```bash
pz-mod-manager help                 # all commands
pz-mod-manager help server start    # parameters of one command
pz-mod-manager schema               # every command with its JSON schema
```

Arguments can be passed three ways:
- positionally: `profile enable ModA ModB`, `server start lab`;
- as options: `--env lab`, `--timeout 30`, `--wait` / `--no-wait`;
- as one JSON object: `--json '{"env":"lab","wait":true}'`.

MCP tool names are the command names with `_`: `server_start`, `mods_check`, and so on.

## Typical flows

Inspect:
```bash
pz-mod-manager env list                      # environments, paths, what is running
pz-mod-manager mods check --env lab          # problems with the active profile's mod order
pz-mod-manager ini show --env lab            # ini vs profile: in_sync, added/removed/moved
```

Change mods:
```bash
pz-mod-manager workshop download 2392709985          # SteamCMD into the cache (blocks)
pz-mod-manager workshop install 2392709985 --env lab --server --client
pz-mod-manager profile enable SomeModId --env lab
pz-mod-manager profile sort --env lab                # dependencies first
pz-mod-manager mods check --env lab                  # must show 0 errors
pz-mod-manager ini apply --env lab                   # refuses on errors unless --force
```

Server:
```bash
pz-mod-manager server start lab --wait                     # blocks until ready
pz-mod-manager server cmd 'servermsg "Restart in 5 minutes"' --env lab
pz-mod-manager server log lab --lines 50 --grep error
pz-mod-manager server stop lab                             # quit → saves the world → waits
```

Game client of an environment:
```bash
pz-mod-manager client start lab              # needs Steam running, or pass --nosteam
pz-mod-manager client status lab
```

`scripts/pz.sh` provides the same operations as short commands: `pz.sh server lab`, `pz.sh stop lab`, `pz.sh client stable`, `pz.sh status lab`.

## Rules for agents

- Read before you write. Run `env list`, `server status` and `ini show` first: another agent or the person may have changed something.
- **Never work in `~/Zomboid` on your own.** It is the game's default folder with the person's saves. Commands that change server files or processes refuse it unless `--allow-user-dir`; pass that flag only if the person explicitly asked. Otherwise create a separate environment.
- Ask the person before choosing folders, passwords or ports that will stay.
- Run `backup create --env <env>` before anything risky: a backup restore, mass mod changes, or sandbox changes on a world that matters. `backup restore` itself takes a safety backup of the current state first.
- If there is a player-facing environment (usually `stable`), experiment in another one (`lab`). Change the player-facing one only when the person asks.
- `server stop` is graceful and saves the world. `server stop --force` and `server kill` can lose progress, so use them only when `quit` does not work.
- `sandbox set` and `backup restore` refuse while the server is running. Stop it first.
- `server cmd` replies through RCON when the ini has `RCONPassword`. Otherwise it writes to the console FIFO and returns the new log lines; that only works for a server the manager started (`console: true` in `server status`).
- `ini apply` and `ini set` take effect after a server restart. `reloadoptions` reloads only some ini options.
- Secrets are masked: `ini get` hides passwords unless `--show-secrets`, and the admin password is never logged.

## Sharing a pack with a friend (`.pzpack`)

A pack (profile) travels as one `.pzpack` file: a zip with `pack.json` (mod order, workshop ids, server address and ports, `pack_id`, `revision`), `mods/<folder>` for local mods, and `AGENT.md` with these same instructions for whoever receives it.

You received a `.pzpack` (the person says "install this" or "sort this out"):
```bash
pz-mod-manager pack info <file.pzpack>       # mods, server address, what is missing here, what changed
pz-mod-manager pack install <file.pzpack>    # unpack local mods, download workshop items, copy the mod order
pz-mod-manager pack status                   # later: update_available when the file has a newer revision
```
`pack install` puts mods on the client of the active environment (`--env` for another), creates or updates a profile with the pack's order, and links it to the file. It is safe on a fresh machine with only the `default` environment: it writes to the client's `mods` folder only. `complete: true` means every mod is on disk; `server.connect` is the address to join. Run it again on a newer file to apply an update.

You are the one sharing:
```bash
pz-mod-manager env set stable --public-host <ip or domain>     # address players connect to
pz-mod-manager pack save <file.pzpack> --profile <name> --env stable --sync
pz-mod-manager pack save --profile <name>                      # after changes: rewrites the remembered file
```
The revision grows only when the name, notes, mod order or server address changed. With `--sync` the GUI also rewrites the file by itself on every change (while that environment is active). If the file sits in a shared folder (Syncthing, Dropbox), the other side sees updates without resending.

In the GUI the same lives in Packs → "Share with a friend" and "Open .pzpack…"; on Windows `scripts\register-pzpack.bat` makes a double-click open the file in the manager.

## Registering the MCP server

Claude Code, user-wide:
```bash
claude mcp add pz-manager -- <absolute path>/target/release/pz-mod-manager mcp
```

Claude Code, per project: put a `.mcp.json` in the project root.
```json
{"mcpServers": {"pz-manager": {"type": "stdio", "command": "<absolute path>/target/release/pz-mod-manager", "args": ["mcp"]}}}
```

Any other MCP client: command `pz-mod-manager`, args `["mcp"]`, transport stdio.

## Testing in isolation

`PZMM_CONFIG_DIR` and `PZMM_DATA_DIR` point the manager at another config and data folder, so experiments never touch the real ones:
```bash
export PZMM_CONFIG_DIR=/tmp/pzmm/cfg PZMM_DATA_DIR=/tmp/pzmm/data
pz-mod-manager env add t --cache-dir /tmp/pzmm/srv --client-cache-dir /tmp/pzmm/cli --name ttest
pz-mod-manager env use t
```

## Development

- `cargo test`: unit tests, including the CLI parser, the MCP handshake and the console FIFO.
- Every user-facing Russian string in the GUI needs an English entry in `src/i18n_en.rs`; a test checks this. CLI and MCP messages are written in English directly.
- Code map:

  | File | What |
  |---|---|
  | `src/cli.rs` | command table, argv parsing, JSON output |
  | `src/mcp.rs` | MCP over stdio |
  | `src/runtime.rs` | starting, stopping and finding processes, logs, console |
  | `src/config.rs` | `config.toml`: environments, profiles, locking |

## Platform notes

- **Linux**: full support. The console FIFO is available, and processes are found through `/proc`.
- **Windows**: commands go through RCON only. Processes are tracked through pid files. The CLI attaches to the parent console.
