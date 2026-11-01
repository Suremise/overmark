//! What became of each hotkey the app tried to register, as the tray publishes it for the
//! Settings window (a process of its own): one line per setting, after a generation number
//! that goes up with each registration, so Settings can tell a fresh answer from the last one.
//!
//! ```text
//! gen=3
//! general.hotkey_draw=registered
//! general.hotkey_draw_frozen=taken
//! ```
//!
//! A setting with no line wasn't registered by this version.

/// What became of one hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Registered: it works.
    Registered,
    /// Another app (or Windows) has it.
    Taken,
    /// Windows refused it for another reason.
    Failed,
    /// The setting's text isn't a hotkey.
    Invalid,
    /// None set.
    Unset,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Registered => "registered",
            State::Taken => "taken",
            State::Failed => "failed",
            State::Invalid => "invalid",
            State::Unset => "unset",
        }
    }

    fn parse(text: &str) -> Option<State> {
        Some(match text {
            "registered" => State::Registered,
            "taken" => State::Taken,
            "failed" => State::Failed,
            "invalid" => State::Invalid,
            "unset" => State::Unset,
            _ => return None,
        })
    }

    /// Whether it means the hotkey doesn't work though one is set.
    pub fn is_problem(self) -> bool {
        matches!(self, State::Taken | State::Failed | State::Invalid)
    }
}

/// The published state: a generation and each setting's [`State`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub generation: u64,
    pub states: Vec<(String, State)>,
}

impl Status {
    /// The state of the setting `key` (`general.hotkey_draw`), if it was registered.
    pub fn get(&self, key: &str) -> Option<State> {
        self.states.iter().find(|(k, _)| k == key).map(|(_, s)| *s)
    }

    /// Sets the state of `key`.
    pub fn set(&mut self, key: &str, state: State) {
        match self.states.iter_mut().find(|(k, _)| k == key) {
            Some((_, s)) => *s = state,
            None => self.states.push((key.to_owned(), state)),
        }
    }

    /// The text the tray publishes.
    pub fn encode(&self) -> String {
        let mut out = format!("gen={}\n", self.generation);
        for (key, state) in &self.states {
            out.push_str(key);
            out.push('=');
            out.push_str(state.as_str());
            out.push('\n');
        }
        out
    }

    /// Reads [`Status::encode`]'s text; lines it doesn't know are skipped.
    pub fn decode(text: &str) -> Option<Status> {
        let mut lines = text.lines();
        let generation = lines.next()?.strip_prefix("gen=")?.parse().ok()?;
        let states = lines
            .filter_map(|l| {
                let (k, v) = l.split_once('=')?;
                Some((k.to_owned(), State::parse(v)?))
            })
            .collect();
        Some(Status { generation, states })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back_what_it_writes() {
        let mut s = Status {
            generation: 3,
            ..Status::default()
        };
        s.set("general.hotkey_draw", State::Registered);
        s.set("general.hotkey_draw_frozen", State::Taken);
        s.set("general.hotkey_draw", State::Failed);
        let text = s.encode();
        assert_eq!(
            text,
            "gen=3\ngeneral.hotkey_draw=failed\ngeneral.hotkey_draw_frozen=taken\n"
        );
        assert_eq!(Status::decode(&text), Some(s.clone()));
        assert_eq!(s.get("general.hotkey_draw_frozen"), Some(State::Taken));
        assert_eq!(s.get("backdrops.hotkey_whiteboard"), None);
    }

    #[test]
    fn skips_what_it_doesnt_know_and_needs_a_generation() {
        let s = Status::decode("gen=1\na=registered\nb=sideways\nnonsense\n").unwrap();
        assert_eq!(s.states, vec![("a".to_owned(), State::Registered)]);
        assert_eq!(Status::decode(""), None);
        assert_eq!(Status::decode("a=registered\n"), None);
        assert!(State::Taken.is_problem() && !State::Unset.is_problem());
    }
}
