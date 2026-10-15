//! What the cat does tick by tick: stand for a beat, then play a randomly
//! chosen take of the clip (spinning at that take's speed), and again.

/// One take: its frames per tick and the sound to start with it.
pub struct Segment {
    pub text: Vec<String>,
    pub font: Option<Vec<String>>,
    /// Index of the sound file to play, if any.
    pub sound: Option<usize>,
}

pub struct Show {
    segments: Vec<Segment>,
    stand_text: String,
    stand_font: Option<String>,
    /// Range of standing ticks between takes.
    pause: std::ops::RangeInclusive<usize>,
    state: State,
    last: Option<usize>,
    rng: u64,
}

enum State {
    Standing { left: usize },
    Playing { segment: usize, tick: usize },
}

/// One tick of the show: the frame to display and, when a take begins, the
/// sound to start.
pub struct Tick<'a> {
    pub text: &'a str,
    pub font: Option<&'a str>,
    pub start_sound: Option<usize>,
}

impl Show {
    pub fn new(
        segments: Vec<Segment>,
        stand_text: String,
        stand_font: Option<String>,
        pause: std::ops::RangeInclusive<usize>,
    ) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9);
        Self {
            segments,
            stand_text,
            stand_font,
            pause,
            state: State::Standing { left: 0 },
            last: None,
            rng: seed | 1,
        }
    }

    /// Start over with a short stand, as when an agent starts working.
    pub fn restart(&mut self) {
        self.state = State::Standing { left: *self.pause.start() };
    }

    /// Advance one tick. A new take only begins when `may_start` (the player
    /// is ready), so its sound and spin start together.
    pub fn tick(&mut self, may_start: bool) -> Tick<'_> {
        if let State::Standing { left } = &mut self.state {
            if *left > 0 {
                *left -= 1;
            } else if may_start && !self.segments.is_empty() {
                let segment = self.pick();
                self.state = State::Playing { segment, tick: 0 };
            }
        }
        match &mut self.state {
            State::Standing { .. } => Tick {
                text: &self.stand_text,
                font: self.stand_font.as_deref(),
                start_sound: None,
            },
            State::Playing { segment, tick } => {
                let index = *segment;
                let at = *tick;
                *tick += 1;
                let current = &self.segments[index];
                if *tick >= current.text.len() {
                    let left = self.random_in(self.pause.clone());
                    self.state = State::Standing { left };
                }
                let current = &self.segments[index];
                Tick {
                    text: &current.text[at],
                    font: current.font.as_ref().map(|frames| frames[at].as_str()),
                    start_sound: if at == 0 { current.sound } else { None },
                }
            }
        }
    }

    /// A random take. With three or more, never the one just played; with
    /// two, that rule would just alternate them, so repeats are allowed.
    fn pick(&mut self) -> usize {
        let count = self.segments.len();
        let mut choice = self.random_in(0..=count - 1);
        if count > 2 && Some(choice) == self.last {
            choice = (choice + 1 + self.random_in(0..=count - 2)) % count;
        }
        self.last = Some(choice);
        choice
    }

    fn random_in(&mut self, range: std::ops::RangeInclusive<usize>) -> usize {
        // xorshift64: plenty for picking takes.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let span = range.end() - range.start() + 1;
        range.start() + (self.rng % span as u64) as usize
    }
}
