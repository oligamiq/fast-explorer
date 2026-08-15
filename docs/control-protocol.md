# FastExplorer control protocol

Protocol version: `fast-explorer/1`.

The transport is a local byte stream using one JSON object per line (JSON Lines). Requests and responses are UTF-8 and must end with `\n`. Unix uses a Unix domain stream socket; Windows uses a Named Pipe.

## Local endpoint

Unix default path, in order:

1. `$XDG_RUNTIME_DIR/fast-explorer/control.sock`
2. `$XDG_STATE_HOME/fast-explorer/control.sock`
3. `~/.local/state/fast-explorer/control.sock`

Windows defaults to a SID-and-nonce-scoped pipe name such as `\\.\pipe\FastExplorer-control-S-1-5-21-...-<nonce>`, rejects remote pipe clients, and creates the pipe with a protected ACL granting access only to the current Windows user, SYSTEM, and local Administrators. A fresh 128-bit nonce is generated on every launch. FastExplorer first binds the unpredictable first pipe instance and only then publishes its name to `%LOCALAPPDATA%\FastExplorer\control-endpoint`; that directory and file receive the same protected ACL. Publishing after the bind closes the nonce-observation/pipe-squatting window. The endpoint file is removed on clean shutdown when it still points to that FastExplorer instance. Use `--ipc-socket <path>` to override the Unix socket path or Windows pipe name, or `--no-ipc` to disable the server.

On Unix the socket is created with mode `0600`. A parent directory created by FastExplorer is set to `0700`; an existing parent directory is never chmodded. A stale socket may be replaced, but a live socket or non-socket path is never removed.

A single request line is limited to 64 KiB. Oversized requests receive `request_too_large`; an unterminated oversized line closes that client connection after the error response.
## Request envelope

```json
{"protocol":"fast-explorer/1","id":1,"method":"get_settings","params":{}}
```

`protocol` and `method` are required. `id` is optional and is copied to the response. `params` defaults to `{}`.

## Response envelope

Success:

```json
{"protocol":"fast-explorer/1","id":1,"ok":true,"result":{}}
```

Failure:

```json
{"protocol":"fast-explorer/1","id":1,"ok":false,"error":{"code":"invalid_params","message":"..."}}
```
## Methods

`ping` — verifies connectivity and protocol version.

`get_settings` — returns both effective runtime settings and saved config settings.

`set_settings` — accepts any subset of `appearance`, `color`, `intensity`, `search_mode` (`default` or `everything`), `ui_font` (`system`, `sans`, `serif`, `monospace`, or `rounded`), and `tailscale_profiles`. Each Tailnet profile has `id`, `label`, and `enabled`. The legacy `tailscale_enabled` boolean is still accepted as an all-profiles compatibility switch. `persist` defaults to `false`.

```json
{"protocol":"fast-explorer/1","id":2,"method":"set_settings","params":{"color":"red","intensity":90,"search_mode":"everything","persist":false}}
```

With `persist:false`, only the running process changes. With `persist:true`, supplied fields are written to `config.json` and become persistent.

`reload_settings` — rereads `config.json`; startup CLI overrides are reapplied if still active.

`get_state` — returns active tab index, tab count, active path, selected path, search query, and whether search results are active.

`navigate` — navigates the active tab: `{"path":"/some/directory"}`.

`search` — runs the configured search backend in the active tab: `{"query":"report"}`.

`clear_search` — clears the active tab search and restores its directory listing.

`refresh` — refreshes the active directory, or reruns the active search.

`new_tab` — creates and activates a new tab.
