# herdr-oiia-status

A [herdr](https://github.com/herdrdev/herdr) plugin that replaces the yellow
"working" dot in the agent sidebar with a spinning OIIA cat, and plays the
sound while it spins.

https://github.com/user-attachments/assets/9b389b12-b8e7-45f4-932a-65f15f997607

> [!WARNING]
> **This plugin was completely vibecoded.** It was written by an AI coding
> assistant in one long chat, with a human directing it and testing every
> step. Expect rough edges, and read it before you run it. It is provided as
> is, without warranty of any kind; see [LICENSE](LICENSE).

## How it works

Herdr's `state_icon` glyph is built into herdr, and plugins have no UI hooks, so
the plugin can't swap that glyph directly. Instead it reports a custom pane
token, `$oiia`, through `pane.report_metadata`:

- **working**: the cat, animated on the GIF's own timing (20 updates a second)
- **idle / blocked / done / unknown**: `○ × ✓ ·`

Then you put `$oiia` where `state_icon` was in your sidebar layout.

The cat dances to the sound. The plugin splits the clip into its bursts
("takes"), picks one at random, and plays it while the cat spins: faster on
loud parts and on quick takes, slower on drawn-out ones. Then the cat stands for
a beat before the next random take. `herdr-oiia-status takes` lists what your
clip splits into.

Each `pane.agent_status_changed` event runs the plugin binary. The first
instance takes a lock in the plugin state dir and animates every agent. It exits
after 20 s with no working agent. Any instance that finds the lock taken exits
right away. Working frames carry a 3 s TTL, so a crash never leaves a frozen cat.

## Install

Requires herdr ≥ 0.9.0, Linux or macOS, and `cargo`.

```bash
herdr plugin install ozkar99/herdr-oiia-status
```

That's it. The first time the plugin runs (on the next agent status change or
herdr start), it sets itself up:

- **Sidebar:** adds a layout to herdr's `config.toml` that puts the cat where
  the status dot was, colored to match your herdr theme, and reloads herdr.
  If you already have your own `[ui.sidebar.agents]`, it leaves yours alone;
  add `$oiia` (and `$oiia2` for the cat's second row) to it yourself.
- **Sound:** the OIIA clip is bundled and plays while agents work.
- **Real GIF cat (kitty or Ghostty):** if either terminal is installed, it
  builds a color font from the bundled GIF, installs it (`~/.local/share/fonts`
  on Linux, `~/Library/Fonts` on macOS) and maps it in the terminal's config.
  Restart the terminal once to load it. Other terminals show a text cat
  instead, automatically.

Everything it writes into other configs sits between
`# BEGIN oiia.status` / `# END oiia.status` markers. To manage it all
yourself, set `auto_setup = false` in
`$(herdr plugin config-dir oiia.status)/config.toml`.

Tested on Linux with kitty. Ghostty and macOS are implemented but untested;
reports welcome.

To use your own media, drop an `oiia.mp3` and/or a green-screen `oiia.gif` into
`$(herdr plugin config-dir oiia.status)/`; they take priority over the bundled
ones (the font rebuilds automatically).

To watch the full-size GIF in a popup (real pixels where the terminal supports
Kitty graphics, colored half blocks elsewhere):

```bash
herdr plugin pane open --plugin oiia.status --entrypoint cat
```

## Configure

Copy `config.example.toml` to `$(herdr plugin config-dir oiia.status)/config.toml`.
You can change the frames, fps, the glyphs for other states, and the sound mode
(`once`, `loop` for the full oiiaoiia experience, or `off`).

## Uninstall

```bash
herdr plugin action invoke oiia.status.unconfigure   # removes layout, font, mappings
herdr plugin uninstall oiia.status   # or: herdr plugin unlink oiia.status
```

