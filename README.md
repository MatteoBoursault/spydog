# spydog

spydog logs a user's actions (shell commands, editor shortcuts, file navigation…) into a SQLite database for later analysis: spotting recurring patterns, measuring real usage, automating frequent gestures.

## How it works

Two binaries:

- **`spydog`** — the daemon: listens on a Unix datagram socket, batches events and writes them to SQLite.
- **`spydog-log`** — the client: sends a JSON event to the daemon and returns immediately.

```bash
spydog &                                    # start the daemon
spydog-log shell command '{"cmd": "ls"}'    # log a command
```

An event is a timestamp, a working directory, a source, a type and a free-form payload. The daemon enriches it with the git project and branch of the directory, plus a session id.

## Installation

```bash
cargo install spydog
```

## Configuration

`~/.config/spydog/config.toml` (generated on first run):

```toml
db_path = "$HOME/.local/share/spydog/events.db"
batch_size = 100
flush_interval_ms = 1000
```

- `db_path`: database location (`$VAR` variables are expanded).
- `batch_size`: maximum batch size before a write.
- `flush_interval_ms`: maximum delay before writing a non-full batch.

## Use case: nvim hook

Log every file save:

```lua
vim.api.nvim_create_autocmd("BufWritePost", {
  callback = function()
    vim.fn.jobstart({
      "spydog-log", "nvim", "save",
      vim.fn.json_encode({ file = vim.fn.expand("%") }),
    }, { detach = true })
  end,
})
```

Then query:

```bash
sqlite3 ~/.local/share/spydog/events.db 'SELECT * FROM events WHERE type = "save"'
```

The principle is the same for any tool: call `spydog-log <source> <type> [payload-json]` from a hook, a shortcut or an event.
