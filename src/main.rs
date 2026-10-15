//! Herdr plugin that replaces the "working" dot with a spinning OIIA cat.
//!
//! Herdr's `state_icon` glyph is built in, so the plugin reports a custom pane
//! token (`$oiia` by default) instead: an animated cat while the agent works and
//! a static glyph for every other state. Swapping `state_icon` for `$oiia` in
//! `[ui.sidebar.agents] rows` completes the replacement.
//!
//! Every status change runs `herdr-oiia-status run`. The first instance takes a
//! lock and animates all agents until none has worked for a while; the others
//! see the lock and exit immediately.

mod cat;
mod clients;
mod config;
mod font;
mod herdr;
mod kitty;
mod rhythm;
mod show;
mod sound;
mod terminals;

use std::collections::{HashMap, HashSet};
use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use config::{Config, SoundMode};
use herdr::{Client, Status};
use sound::Player;

/// How often the agent list is refreshed.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// The animator exits after this long without any working agent.
const IDLE_EXIT: Duration = Duration::from_secs(20);
/// Working frames expire on their own if the animator dies mid-spin.
const FRAME_TTL_MS: u64 = 3_000;
/// How often to re-check whether every client can draw the cat font.
const CLIENT_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Consecutive socket failures before assuming the server is gone.
const MAX_FAILURES: u32 = 20;

fn main() -> ExitCode {
    let env = Env::read();
    let config = Config::load(env.config_dir.as_deref(), env.state_dir.as_deref());
    let result = match std::env::args().nth(1).as_deref() {
        Some("run") => run(&env, config),
        Some("test-sound") => test_sound(&env, &config),
        Some("clear") => clear(&config),
        Some("install-font") => install_font(&env, &config, std::env::args().nth(2)),
        Some("pane") => pane(&env, std::env::args().nth(2)),
        Some("takes") => print_takes(&env, &config),
        Some("preview") => preview(&env, std::env::args().nth(2)),
        _ => {
            eprintln!("usage: herdr-oiia-status <run|test-sound|clear>");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("oiia: {err}");
            ExitCode::FAILURE
        }
    }
}

struct Env {
    config_dir: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    plugin_root: Option<PathBuf>,
}

impl Env {
    fn read() -> Self {
        let dir = |name| std::env::var_os(name).map(PathBuf::from);
        Self {
            config_dir: dir("HERDR_PLUGIN_CONFIG_DIR"),
            state_dir: dir("HERDR_PLUGIN_STATE_DIR"),
            plugin_root: dir("HERDR_PLUGIN_ROOT"),
        }
    }

    fn sound_file(&self, config: &Config) -> Option<PathBuf> {
        let file = config
            .sound
            .resolve_file(self.config_dir.as_deref(), self.plugin_root.as_deref());
        if file.is_none() {
            eprintln!(
                "oiia: sound file {} not found in the config dir or plugin root",
                config.sound.file.display()
            );
        }
        file
    }

    fn player(&self, config: &Config, files: &[PathBuf]) -> Option<Player> {
        if config.sound.mode == SoundMode::Off || files.is_empty() {
            return None;
        }
        let player = Player::new(files, &config.sound.player);
        if player.is_none() {
            eprintln!("oiia: no audio player found (install ffplay, mpv or pw-play, or set sound.player)");
        }
        player
    }
}

fn run(env: &Env, config: Config) -> std::io::Result<()> {
    let lock_path = env
        .state_dir
        .clone()
        .unwrap_or_else(std::env::temp_dir)
        .join("oiia-animator.lock");
    let mut lock = File::create(&lock_path)?;
    if !try_lock(&lock)? {
        return Ok(()); // Another instance is already animating.
    }

    let mut client = Client::from_env()?;
    let (show, takes) = build_show(env, &config);
    let player = env.player(&config, &takes);
    let mut animator = Animator::new(&config, show, player);
    loop {
        animator.run(&mut client)?;

        // Idle. Release the lock *before* the final check: a hook that fired
        // while we held it has already exited, but its agent's status is
        // visible to the check below, and any later hook can take the lock.
        drop(lock);
        let still_working = client
            .agents()?
            .iter()
            .any(|agent| agent.agent_status == Status::Working);
        if !still_working {
            return Ok(());
        }
        lock = File::create(&lock_path)?;
        if !try_lock(&lock)? {
            return Ok(());
        }
    }
}

/// Text poses advanced per tick at full speed: one every two ticks.
const TEXT_MAX_STEP: f32 = 0.5;
/// GIF spin frames advanced per tick at full speed: a bit faster than the GIF.
const FONT_MAX_STEP: f32 = 2.0;

/// Standing time between takes, in seconds.
const PAUSE_SECS: (f32, f32) = (0.6, 1.6);

/// Build the show: one segment per take of the sound clip (each written to
/// its own WAV for the player), or, without a clip, the GIF's own rhythm.
/// Text and font frames of a segment have the same length, so switching
/// between them keeps the beat.
fn build_show(env: &Env, config: &Config) -> (show::Show, Vec<PathBuf>) {
    let tick_ms = 1_000 / config.fps;
    let poses = &config.frames;
    let font = config.font_poses.as_ref();
    let segment = |speeds: &[f32], sound| show::Segment {
        text: rhythm::sequence(&poses[0], poses, TEXT_MAX_STEP, speeds),
        font: font.map(|font| rhythm::sequence(&font.stand, &font.spin, FONT_MAX_STEP, speeds)),
        sound,
    };
    let stand_font = font.map(|font| font.stand.clone());

    if let Some((takes, files)) = split_takes(env, config, tick_ms) {
        let segments = takes.iter().enumerate().map(|(i, take)| segment(&take.speeds, Some(i))).collect();
        let ticks = |secs: f32| (secs * 1_000.0 / tick_ms as f32) as usize;
        let pause = ticks(PAUSE_SECS.0)..=ticks(PAUSE_SECS.1);
        return (show::Show::new(segments, poses[0].clone(), stand_font, pause), files);
    }

    // No clip: loop the GIF's stand/spin pattern, or just spin.
    let speeds = env
        .config_dir
        .as_ref()
        .and_then(|dir| cat::load(&dir.join("oiia.gif")).ok())
        .map(|frames| rhythm::from_gif(&cat::plan(&frames, tick_ms)))
        .unwrap_or_else(|| vec![1.0; 40]);
    let segments = vec![segment(&speeds, None)];
    (show::Show::new(segments, poses[0].clone(), stand_font, 0..=0), Vec::new())
}

/// Decode the sound clip, split it into takes and write each as a WAV in the
/// state dir.
fn split_takes(env: &Env, config: &Config, tick_ms: u32) -> Option<(Vec<rhythm::Burst>, Vec<PathBuf>)> {
    let file = env.sound_file(config)?;
    let (samples, rate) = rhythm::decode_mono(&file)?;
    let takes = rhythm::bursts(&samples, rate, tick_ms);
    if takes.is_empty() {
        return None;
    }
    let dir = env.state_dir.clone().unwrap_or_else(std::env::temp_dir);
    // Remove takes left over from a clip that split into more of them.
    for stale in takes.len().. {
        if std::fs::remove_file(dir.join(format!("oiia-take-{stale}.wav"))).is_err() {
            break;
        }
    }
    let mut files = Vec::new();
    for (i, take) in takes.iter().enumerate() {
        let path = dir.join(format!("oiia-take-{i}.wav"));
        rhythm::write_wav(&path, &samples[take.samples.clone()], rate).ok()?;
        files.push(path);
    }
    Some((takes, files))
}

fn try_lock(file: &File) -> std::io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(err)) => Err(err),
    }
}

struct Animator<'a> {
    config: &'a Config,
    show: show::Show,
    player: Option<Player>,
    /// Last status seen per agent pane.
    statuses: HashMap<String, Status>,
    /// Last static glyph reported per non-working pane.
    shown: HashMap<String, String>,
    /// Every attached client is in kitty or Ghostty, so font frames render.
    use_font: bool,
    last_client_check: Option<Instant>,
    /// Some agent was working on the previous tick.
    was_working: bool,
    /// When work last started; the first take waits a little for the player.
    started_working: Instant,
    /// `once` mode already played its sound since work started.
    played_once: bool,
}

impl<'a> Animator<'a> {
    fn new(config: &'a Config, show: show::Show, mut player: Option<Player>) -> Self {
        if let Some(player) = &mut player {
            player.warm_up();
        }
        Self {
            config,
            show,
            player,
            statuses: HashMap::new(),
            shown: HashMap::new(),
            use_font: false,
            last_client_check: None,
            was_working: false,
            started_working: Instant::now(),
            played_once: false,
        }
    }

    /// Animate until no agent has been working for `IDLE_EXIT`.
    fn run(&mut self, client: &mut Client) -> std::io::Result<()> {
        let tick = Duration::from_millis(1_000 / u64::from(self.config.fps));
        let mut last_poll: Option<Instant> = None;
        let mut last_working = Instant::now();
        let mut failures = 0;

        loop {
            let started = Instant::now();
            if last_poll.is_none_or(|at| at.elapsed() >= POLL_INTERVAL) {
                match client.agents() {
                    Ok(agents) => {
                        failures = 0;
                        self.update_statuses(client, agents.into_iter().map(|a| (a.pane_id, a.agent_status)));
                    }
                    Err(err) => {
                        failures += 1;
                        if failures >= MAX_FAILURES {
                            return Err(err);
                        }
                    }
                }
                last_poll = Some(started);
            }

            let working: Vec<&String> = self
                .statuses
                .iter()
                .filter(|(_, status)| **status == Status::Working)
                .map(|(pane, _)| pane)
                .collect();
            if working.is_empty() {
                if self.was_working {
                    self.was_working = false;
                    if let Some(player) = &mut self.player
                        && self.config.sound.mode == SoundMode::Loop
                    {
                        player.stop();
                    }
                }
                if last_working.elapsed() >= IDLE_EXIT {
                    return Ok(());
                }
            } else {
                last_working = started;
                if self.config.font_poses.is_some()
                    && self.last_client_check.is_none_or(|at| at.elapsed() >= CLIENT_CHECK_INTERVAL)
                {
                    self.use_font = clients::all_can_draw_font();
                    self.last_client_check = Some(started);
                }
                if !self.was_working {
                    self.was_working = true;
                    self.started_working = started;
                    self.played_once = false;
                    self.show.restart();
                }
                // A take starts only once the player can start on cue
                // (PowerShell needs a moment), waiting a few seconds at most.
                let ready = self.player.as_mut().is_none_or(Player::is_ready)
                    || self.started_working.elapsed() >= Duration::from_secs(4);
                let tick = self.show.tick(ready);
                let frame = match tick.font {
                    Some(font) if self.use_font => font,
                    _ => tick.text,
                }
                .to_string();
                let start_sound = tick.start_sound;
                if let Some(take) = start_sound
                    && let Some(player) = &mut self.player
                {
                    match self.config.sound.mode {
                        SoundMode::Loop => player.restart(take),
                        SoundMode::Once if !self.played_once => {
                            player.restart(take);
                            self.played_once = true;
                        }
                        _ => {}
                    }
                }
                let mut lines = frame.split('\n');
                let tokens: Vec<(String, Option<&str>)> = self
                    .token_names()
                    .into_iter()
                    .map(|name| (name, lines.next()))
                    .collect();
                for pane in working {
                    let _ = client.set_tokens(pane, &tokens, Some(FRAME_TTL_MS));
                }
            }

            std::thread::sleep(tick.saturating_sub(started.elapsed()));
        }
    }

    fn update_statuses(&mut self, client: &mut Client, agents: impl Iterator<Item = (String, Status)>) {
        let mut seen = HashSet::new();
        for (pane, status) in agents {
            seen.insert(pane.clone());
            self.statuses.insert(pane.clone(), status);

            if status == Status::Working {
                self.shown.remove(&pane);
                continue;
            }

            let glyph = self.glyph(status);
            if self.shown.get(&pane).map(String::as_str) != Some(glyph) {
                let value = (!glyph.is_empty()).then_some(glyph);
                // The state glyph sits on the first row; extra cat rows clear.
                let tokens: Vec<(String, Option<&str>)> = self
                    .token_names()
                    .into_iter()
                    .enumerate()
                    .map(|(row, name)| (name, if row == 0 { value } else { None }))
                    .collect();
                if client.set_tokens(&pane, &tokens, None).is_ok() {
                    self.shown.insert(pane, glyph.to_string());
                }
            }
        }
        self.statuses.retain(|pane, _| seen.contains(pane));
        self.shown.retain(|pane, _| seen.contains(pane));
    }

    /// `oiia`, `oiia2`, `oiia3`, ... one per row of the tallest pose of either
    /// cat, so switching to a shorter cat clears the rows it does not use.
    fn token_names(&self) -> Vec<String> {
        let rows = self
            .config
            .frames
            .iter()
            .chain(self.config.font_poses.iter().map(|poses| &poses.stand))
            .map(|f| f.split('\n').count())
            .max()
            .unwrap_or(1);
        (0..rows)
            .map(|row| match row {
                0 => self.config.token.clone(),
                n => format!("{}{}", self.config.token, n + 1),
            })
            .collect()
    }

    fn glyph(&self, status: Status) -> &str {
        let states = &self.config.states;
        match status {
            Status::Idle => &states.idle,
            Status::Blocked => &states.blocked,
            Status::Done => &states.done,
            Status::Unknown => &states.unknown,
            Status::Working => self.config.frames[0].split('\n').next().unwrap_or_default(),
        }
    }
}

fn test_sound(env: &Env, config: &Config) -> std::io::Result<()> {
    let file = env.sound_file(config).ok_or_else(|| std::io::Error::other("no sound file"))?;
    let mut player = Player::one_shot(&file, &config.sound.player)
        .ok_or_else(|| std::io::Error::other("no audio player found"))?;
    player.restart(0);
    player.wait();
    Ok(())
}

/// Print the GIF as braille strips at sidebar sizes, plus a true-color render.
fn preview(env: &Env, gif: Option<String>) -> std::io::Result<()> {
    let path = gif.map(PathBuf::from).unwrap_or_else(|| {
        env.config_dir.clone().unwrap_or_default().join("oiia.gif")
    });
    let frames = cat::load(&path)?;
    let area = cat::bounds(&frames);
    let total: u32 = frames.iter().map(|f| f.delay_ms).sum();
    println!("{} frames, {} ms per loop, cat bounds {area:?}\n", frames.len(), total);
    let motion: Vec<String> = frames
        .windows(2)
        .map(|pair| format!("{:.0}", cat::motion(&pair[0], &pair[1]) * 1000.0))
        .collect();
    println!("motion per frame (0 = still): {}\n", motion.join(" "));

    for (cols, rows) in [(3, 1), (4, 1), (5, 1), (6, 1), (4, 2), (6, 3)] {
        println!("{cols}x{rows} cells:");
        let rendered: Vec<Vec<String>> =
            frames.iter().map(|f| cat::braille(f, area, cols, rows)).collect();
        for chunk in rendered.chunks(16) {
            for row in 0..rows {
                let line: Vec<&str> = chunk.iter().map(|f| f[row].as_str()).collect();
                println!("  {}", line.join(" "));
            }
            println!();
        }
    }

    let step = (frames.len() / 4).max(1);
    for frame in frames.iter().step_by(step).take(4) {
        println!("{}", cat::halfblock(frame, area, 40, 20));
    }
    Ok(())
}

/// Play the GIF in the pane: real pixels through Kitty graphics when the
/// terminal supports them, otherwise true-color half blocks with the braille
/// sidebar sizes underneath. Any key exits.
fn pane(env: &Env, gif: Option<String>) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};

    let path = gif.map(PathBuf::from).unwrap_or_else(|| {
        env.config_dir.clone().unwrap_or_default().join("oiia.gif")
    });
    let frames = cat::load(&path)?;
    let area = cat::bounds(&frames);

    let _raw = term::RawMode::enable();
    let graphics = kitty::supported();
    static QUIT: AtomicBool = AtomicBool::new(false);
    std::thread::spawn(|| {
        let mut byte = [0u8; 1];
        let _ = std::io::Read::read(&mut std::io::stdin(), &mut byte);
        QUIT.store(true, Ordering::Relaxed);
    });

    let mut out = std::io::stdout().lock();
    write!(out, "\x1b[?1049h\x1b[?25l")?;
    let result = if graphics {
        pane_graphics(&mut out, &frames, area, &QUIT)
    } else {
        pane_text(&mut out, &frames, area, &QUIT)
    };
    write!(out, "\x1b[?25h\x1b[?1049l")?;
    out.flush()?;
    result
}

fn pane_graphics(
    out: &mut impl std::io::Write,
    frames: &[cat::Frame],
    area: (usize, usize, usize, usize),
    quit: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    const FIRST_ID: u32 = 0x0a1a_0000;
    for (n, frame) in frames.iter().enumerate() {
        let (width, height, rgba) = cat::rgba(frame, area, 320);
        kitty::upload(out, FIRST_ID + n as u32, width, height, &rgba)?;
    }
    out.flush()?;

    let aspect = (area.2 - area.0) as f32 / (area.3 - area.1) as f32;
    let mut size = (0, 0);
    let mut previous = None;
    let mut i = 0;
    while !quit.load(std::sync::atomic::Ordering::Relaxed) {
        let (width, height) = term::size();
        let rows = height.saturating_sub(2).max(2);
        // Cells are about twice as tall as wide.
        let cols = ((rows * 2) as f32 * aspect) as usize;
        if (width, height) != size {
            size = (width, height);
            write!(out, "\x1b[2J\x1b[{height};1H\x1b[2mpress any key to close\x1b[0m")?;
        }
        let n = i % frames.len();
        let id = FIRST_ID + n as u32;
        // Place the new frame before hiding the old one so nothing blinks.
        write!(out, "\x1b[1;{}H", width.saturating_sub(cols) / 2 + 1)?;
        kitty::place(out, id, rows)?;
        if let Some(old) = previous.filter(|&old| old != id) {
            kitty::hide(out, old)?;
        }
        out.flush()?;
        previous = Some(id);
        std::thread::sleep(std::time::Duration::from_millis(u64::from(frames[n].delay_ms)));
        i += 1;
    }
    for n in 0..frames.len() {
        kitty::free(out, FIRST_ID + n as u32)?;
    }
    Ok(())
}

fn pane_text(
    out: &mut impl std::io::Write,
    frames: &[cat::Frame],
    area: (usize, usize, usize, usize),
    quit: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    let sizes = [(3, 1), (4, 1), (6, 1), (4, 2), (6, 3)];
    let strips: Vec<Vec<Vec<String>>> = sizes
        .iter()
        .map(|&(cols, rows)| frames.iter().map(|f| cat::braille(f, area, cols, rows)).collect())
        .collect();
    let (aspect_w, aspect_h) = ((area.2 - area.0) as f32, (area.3 - area.1) as f32);
    let mut i = 0;
    while !quit.load(std::sync::atomic::Ordering::Relaxed) {
        let (width, height) = term::size();
        let legend = sizes.iter().map(|&(_, rows)| rows + 1).sum::<usize>() + 2;
        let max_rows = height.saturating_sub(legend).max(4);
        let max_cols = width.saturating_sub(2).max(8);
        // Half blocks give square pixels: 1 col = 1 px wide, 1 row = 2 px tall.
        let rows = max_rows.min((max_cols as f32 * aspect_h / aspect_w / 2.0) as usize).max(2);
        let cols = ((rows * 2) as f32 * aspect_w / aspect_h) as usize;
        let pad = " ".repeat(width.saturating_sub(cols) / 2);

        let frame = &frames[i % frames.len()];
        write!(out, "\x1b[H\x1b[2J")?;
        for line in cat::halfblock(frame, area, cols, rows).lines() {
            write!(out, "{pad}{line}\r\n")?;
        }
        for (&(cols, rows), strip) in sizes.iter().zip(&strips) {
            write!(out, "\x1b[33m")?;
            for (row, line) in strip[i % frames.len()].iter().enumerate() {
                let label = if row == 0 { format!("{cols}x{rows}") } else { String::new() };
                write!(out, "  {label:>4}  {line}\r\n")?;
            }
            write!(out, "\x1b[0m\r\n")?;
        }
        write!(out, "\x1b[2mpress any key to close (no Kitty graphics: text mode)\x1b[0m")?;
        out.flush()?;
        std::thread::sleep(std::time::Duration::from_millis(u64::from(frame.delay_ms)));
        i += 1;
    }
    Ok(())
}

mod term {
    pub struct RawMode(Option<libc::termios>);

    impl RawMode {
        pub fn enable() -> Self {
            // SAFETY: plain termios calls on stdin with a zeroed struct.
            unsafe {
                let mut original: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut original) != 0 {
                    return Self(None);
                }
                let mut raw = original;
                raw.c_lflag &= !(libc::ICANON | libc::ECHO);
                libc::tcsetattr(0, libc::TCSANOW, &raw);
                Self(Some(original))
            }
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            if let Some(original) = &self.0 {
                // SAFETY: restores the settings read in `enable`.
                unsafe { libc::tcsetattr(0, libc::TCSANOW, original) };
            }
        }
    }

    pub fn size() -> (usize, usize) {
        // SAFETY: TIOCGWINSZ fills a winsize struct.
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                (usize::from(ws.ws_col), usize::from(ws.ws_row))
            } else {
                (80, 24)
            }
        }
    }
}

/// Build the GIF font, install it for this user, and map its code points in
/// kitty, which then draws the `$oiia` token as the real cat.
fn install_font(env: &Env, config: &Config, gif: Option<String>) -> std::io::Result<()> {
    let path = gif.map(PathBuf::from).unwrap_or_else(|| {
        env.config_dir.clone().unwrap_or_default().join("oiia.gif")
    });
    let frames = cat::load(&path)?;
    let area = cat::bounds(&frames);
    // One glyph per terminal cell, so the terminal never has to squeeze or
    // crop a wide picture into one character.
    let [cols, rows] = config.font_grid.map(|n| n.clamp(1, 8) as usize);
    let (stand, spin) = cat::poses(&frames);
    let mut images = Vec::new();
    for &index in std::iter::once(&stand).chain(&spin) {
        images.extend(cat::tiles(&frames[index], area, cols, rows, font::TILE_WIDTH, font::TILE_HEIGHT));
    }
    let bytes = font::build(&images, font::Format::native())?;

    let home = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| std::io::Error::other("HOME is not set"))?);
    let font_dir = if cfg!(target_os = "macos") {
        home.join("Library/Fonts")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("fonts")
    };
    std::fs::create_dir_all(&font_dir)?;
    let font_path = font_dir.join("oiia-cat.ttf");
    std::fs::write(&font_path, bytes)?;
    if !cfg!(target_os = "macos") {
        let _ = std::process::Command::new("fc-cache").arg("-f").arg(&font_dir).status();
    }
    println!("installed {}", font_path.display());

    let state_dir = env.state_dir.clone().unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&state_dir)?;
    std::fs::write(state_dir.join(config::FONT_FRAMES_FILE), format!("{cols} {rows}\n{}\n", spin.len()))?;
    println!("1 standing pose + {} spin poses", spin.len());

    // Never signal a terminal to reload: older kitty exits on SIGUSR1.
    let last = font::FIRST_CODEPOINT + images.len() as u32 - 1;
    for note in terminals::configure(&home, font::FAMILY, font::FIRST_CODEPOINT, last)? {
        println!("{note}");
    }
    Ok(())
}

/// List the takes the sound clip splits into, with their spin speed.
fn print_takes(env: &Env, config: &Config) -> std::io::Result<()> {
    let tick_ms = 1_000 / config.fps;
    let (takes, files) =
        split_takes(env, config, tick_ms).ok_or_else(|| std::io::Error::other("no takes in the sound clip"))?;
    for ((i, take), file) in takes.iter().enumerate().zip(files) {
        let spinning: Vec<f32> = take.speeds.iter().copied().filter(|s| *s > 0.0).collect();
        let average = spinning.iter().sum::<f32>() / spinning.len().max(1) as f32;
        println!(
            "take {i}: {:.2}s, average spin speed {average:.2} -> {}",
            (take.speeds.len() as u32 * tick_ms) as f32 / 1_000.0,
            file.display()
        );
    }
    Ok(())
}

fn clear(config: &Config) -> std::io::Result<()> {
    let mut client = Client::from_env()?;
    let tokens: Vec<(String, Option<&str>)> = std::iter::once(config.token.clone())
        .chain((2..=8).map(|n| format!("{}{n}", config.token)))
        .map(|name| (name, None))
        .collect();
    for agent in client.agents()? {
        client.set_tokens(&agent.pane_id, &tokens, None)?;
    }
    Ok(())
}
