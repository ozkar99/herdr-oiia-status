//! Plays the OIIA sound through whatever audio player the machine has.
//!
//! Herdr plugins have no audio API, and linking an audio stack (ALSA headers
//! etc.) would make `herdr plugin install` fail on machines without them, so we
//! shell out to a player instead.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Plays one of a set of clips from the start on demand, so the animator can
/// keep the sound and the spin in step.
pub struct Player {
    kind: Kind,
    child: Option<Child>,
}

enum Kind {
    /// A new process per play (native players start in milliseconds); one
    /// argv per clip.
    Spawn(Vec<Vec<String>>),
    /// One long-lived PowerShell process taking `play N`/`stop` lines on stdin;
    /// starting PowerShell takes seconds, far too slow to restart per loop.
    Controlled { argv: Vec<String>, stdin: Option<ChildStdin>, ready: Arc<AtomicBool> },
}

impl Player {
    pub fn new(files: &[PathBuf], custom: &[String]) -> Option<Self> {
        let kind = if !custom.is_empty() {
            Kind::Spawn(
                files
                    .iter()
                    .map(|file| {
                        let file = file.to_string_lossy();
                        custom.iter().map(|arg| arg.replace("{file}", &file)).collect()
                    })
                    .collect(),
            )
        } else if let Some(argvs) = files.iter().map(|file| detect(file)).collect::<Option<Vec<_>>>() {
            Kind::Spawn(argvs)
        } else {
            Kind::Controlled {
                argv: wsl_powershell(files, PowerShellMode::Controlled)?,
                stdin: None,
                ready: Arc::new(AtomicBool::new(false)),
            }
        };
        Some(Self { kind, child: None })
    }

    /// A player that plays one clip once and exits, for `wait`.
    pub fn one_shot(file: &Path, custom: &[String]) -> Option<Self> {
        let files = [file.to_path_buf()];
        let mut player = Self::new(&files, custom)?;
        if let Kind::Controlled { .. } = player.kind {
            player.kind = Kind::Spawn(vec![wsl_powershell(&files, PowerShellMode::Once)?]);
        }
        Some(player)
    }

    /// Start a controlled player ahead of time so the first play is instant.
    pub fn warm_up(&mut self) {
        if let Kind::Controlled { argv, stdin, ready } = &mut self.kind
            && self.child.is_none()
        {
            let Some(mut child) = spawn(argv, Stdio::piped(), Stdio::piped()) else { return };
            *stdin = child.stdin.take();
            if let Some(stdout) = child.stdout.take() {
                let ready = Arc::clone(ready);
                std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        if line.trim() == "ready" {
                            ready.store(true, Ordering::Relaxed);
                        }
                    }
                });
            }
            self.child = Some(child);
        }
    }

    /// Whether `restart` will start sound right away.
    pub fn is_ready(&mut self) -> bool {
        match &self.kind {
            Kind::Spawn(_) => true,
            Kind::Controlled { ready, .. } => ready.load(Ordering::Relaxed),
        }
    }

    /// Play clip `index` from the beginning, cutting off anything playing.
    pub fn restart(&mut self, index: usize) {
        match &mut self.kind {
            Kind::Spawn(argvs) => {
                if let Some(mut child) = self.child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                if let Some(argv) = argvs.get(index) {
                    self.child = spawn(argv, Stdio::null(), Stdio::null());
                }
            }
            Kind::Controlled { .. } => {
                self.warm_up();
                self.send(&format!("play {index}"));
            }
        }
    }

    pub fn stop(&mut self) {
        match &self.kind {
            Kind::Spawn(_) => {
                if let Some(mut child) = self.child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            Kind::Controlled { .. } => self.send("stop"),
        }
    }

    /// Block until a one-shot clip finishes.
    pub fn wait(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }

    fn send(&mut self, command: &str) {
        if let Kind::Controlled { stdin: Some(stdin), .. } = &mut self.kind
            && writeln!(stdin, "{command}").and_then(|()| stdin.flush()).is_err()
        {
            // PowerShell died; start over on the next play.
            self.child = None;
            if let Kind::Controlled { stdin, ready, .. } = &mut self.kind {
                *stdin = None;
                ready.store(false, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn(argv: &[String], stdin: Stdio, stdout: Stdio) -> Option<Child> {
    let mut command = Command::new(&argv[0]);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // The player must not outlive us, even if we are SIGKILLed.
        // SAFETY: prctl is async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    command
        .args(&argv[1..])
        .stdin(stdin)
        .stdout(stdout)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| eprintln!("oiia: could not start {}: {err}", argv[0]))
        .ok()
}

fn detect(file: &Path) -> Option<Vec<String>> {
    let path = file.to_string_lossy().into_owned();
    let ext = file
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    // libsndfile-based players cannot decode mp3.
    let sndfile_ok = matches!(ext.as_str(), "wav" | "ogg" | "oga" | "flac");

    let candidates: [(&str, &[&str], bool); 8] = [
        ("afplay", &[], true),
        ("pw-play", &[], sndfile_ok),
        ("paplay", &[], sndfile_ok),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"], true),
        ("mpv", &["--no-video", "--really-quiet"], true),
        ("cvlc", &["--play-and-exit", "--quiet"], true),
        ("play", &["-q"], true),
        ("aplay", &["-q"], ext == "wav"),
    ];
    for (program, args, usable) in candidates {
        if usable && which(program).is_some() {
            let mut argv = vec![program.to_string()];
            argv.extend(args.iter().map(|arg| arg.to_string()));
            argv.push(path);
            return Some(argv);
        }
    }
    None
}

enum PowerShellMode {
    /// Play once and exit.
    Once,
    /// Print `ready`, then obey `play N`/`stop` lines until stdin closes.
    Controlled,
}

/// On WSL without a Linux player, play through Windows' MediaPlayer.
fn wsl_powershell(files: &[PathBuf], mode: PowerShellMode) -> Option<Vec<String>> {
    std::env::var_os("WSL_DISTRO_NAME")?;
    let powershell = which("powershell.exe")?;
    let mut uris = Vec::new();
    for file in files {
        let output = Command::new("wslpath").arg("-w").arg(file).output().ok()?;
        let path = String::from_utf8(output.stdout).ok()?.trim().replace('\'', "''");
        uris.push(format!("[uri]'{path}'"));
    }
    let body = match mode {
        PowerShellMode::Once => "$ps[0].Play(); Start-Sleep -Milliseconds ([int]$ps[0].NaturalDuration.TimeSpan.TotalMilliseconds)",
        PowerShellMode::Controlled => {
            "[Console]::Out.WriteLine('ready'); [Console]::Out.Flush(); \
             while (($line = [Console]::In.ReadLine()) -ne $null) { \
               foreach ($p in $ps) { $p.Stop() }; \
               if ($line -match '^play (\\d+)$') { $p = $ps[[int]$Matches[1]]; $p.Position = [TimeSpan]::Zero; $p.Play() } }"
        }
    };
    // Open every clip up front so a play starts instantly.
    let script = format!(
        "Add-Type -AssemblyName PresentationCore; \
         $ps = @({uris}) | ForEach-Object {{ $p = New-Object System.Windows.Media.MediaPlayer; $p.Open($_); $p }}; \
         $t = 0; while (@($ps | Where-Object {{ -not $_.NaturalDuration.HasTimeSpan }}).Count -gt 0 -and $t -lt 100) {{ Start-Sleep -Milliseconds 50; $t++ }}; \
         {body}",
        uris = uris.join(", "),
    );
    Some(vec![
        powershell.to_string_lossy().into_owned(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        script,
    ])
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}
