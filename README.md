# herdr-oiia-status

A [herdr](https://github.com/herdrdev/herdr) plugin that replaces the yellow
"working" dot in the agent sidebar with a spinning OIIA cat, and plays the
sound while it spins.

```
ᓚᘏᗢ · my-project · main      →      ⠀ᗢ⠀ · my-project · main      →      ᗢᘏᓗ · …
```

> [!WARNING]
> **This plugin was completely vibecoded.** It was written by an AI coding
> assistant in one long chat, with a human mostly saying "make the cat
> bigger". Read it before you run it, expect rough edges, and don't use it for
> anything that matters. It is provided as is, without warranty of any kind;
> see [LICENSE](LICENSE). If it breaks your terminal, that's on you.

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
# or, from a local checkout:
cargo build --release && herdr plugin link .
```

### 1. Put the cat in the sidebar

Add this to `~/.config/herdr/config.toml`, then run `herdr server reload-config`.
It is the default layout with `state_icon` swapped for a colored `$oiia`. The
colors shown are herdr's Dracula palette; adjust them for your theme.

```toml
[ui.sidebar.agents]
rows = [
  [{ token = "$oiia", fg = "#f1fa8c", rules = [
      { equals = "○", fg = "#50fa7b" },
      { equals = "×", fg = "#ff5555" },
      { equals = "✓", fg = "#8be9fd" },
      { equals = "·", fg = "#6272a4" },
  ] }, "machine", "workspace", "tab"],
  ["agent"],
]
```

### 2. Sound

The OIIA clip and GIF are bundled in `assets/`, so sound works out of the box:

```bash
herdr plugin action invoke oiia.status.test-sound
```

To use your own, drop an `oiia.mp3` (or set `sound.file`) and/or an `oiia.gif`
into `$(herdr plugin config-dir oiia.status)/`; those take priority.

Sound plays through the first player found: `afplay`, `pw-play`, `paplay`,
`ffplay`, `mpv`, `cvlc`, `play`, or `aplay`. On WSL with none of those, it plays
through Windows' MediaPlayer via `powershell.exe`. Set `sound.player` to use
something else.

### 3. The real GIF in the sidebar (kitty or Ghostty)

Build the cat font from the bundled GIF (or your own green-screen GIF in the
config dir):

```bash
herdr plugin action invoke oiia.status.install-font
```

This builds a color font from the GIF frames (each frame cut into cell-sized
pieces) and installs it: `~/.local/share/fonts` on Linux, `~/Library/Fonts` on
macOS (macOS gets Apple's `sbix` bitmap format, Linux the CBDT format of Noto
Color Emoji). It then adds a managed block to each installed terminal's config:
`symbol_map` + `narrow_symbols` in `kitty.conf`, `font-codepoint-map` in
Ghostty's `config`. Restart the terminal afterwards.

While every attached herdr client runs inside kitty or Ghostty, `$oiia` uses
those characters and the sidebar shows the actual cat. If any client is in
another terminal, it falls back to the text cat automatically (checked every
2 s).

Tested on Linux with kitty. Ghostty and macOS are implemented but untested;
reports welcome. For a bigger cat, set `font_grid = [4, 2]`, re-run
`install-font`, and add a `["$oiia2"]` row under the first row.

To watch the full-size GIF in a popup (real pixels where the terminal supports
Kitty graphics, colored half blocks elsewhere):

```bash
herdr plugin pane open --plugin oiia.status --entrypoint cat
```

## Configure

Copy `config.example.toml` to `$(herdr plugin config-dir oiia.status)/config.toml`.
You can change the frames, fps, the glyphs for other states, and the sound mode
(`once`, `loop` for the full oiiaoiia experience, or `off`).

## Credits

The bundled OIIA cat media are copies of the viral meme, included for fun:

- `assets/oiia.gif`: the green-screen spinning cat, from
  [Tenor](https://tenor.com/view/meme-uiia-gif-2363911505930596945)
- `assets/oiia.mp3`: the "OIIAIOIIIAI cat" clip, from
  [myinstants](https://www.myinstants.com/en/instant/oiiaioiiiai-cat-57035/)

All rights to the original cat video and audio belong to their creators. If
you own them and want them removed, open an issue.

## Uninstall

```bash
herdr plugin action invoke oiia.status.clear
rm ~/.local/share/fonts/oiia-cat.ttf   # macOS: ~/Library/Fonts/oiia-cat.ttf
# and delete the oiia.status block in kitty.conf / Ghostty's config
herdr plugin uninstall oiia.status   # or: herdr plugin unlink oiia.status
```

Then put `state_icon` back in your sidebar rows.
