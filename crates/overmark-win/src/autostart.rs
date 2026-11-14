//! Start with Windows: per-user entries are writable; machine entries are read-only.

use std::io;
use std::path::Path;

use crate::win32::registry;

/// The Run key, read under HKCU and HKLM; only HKCU is written.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Overmark's value in the Run key (matching the MSI's).
pub const VALUE_NAME: &str = "Overmark";

/// Where tests keep their own keys, under `HKEY_CURRENT_USER`: never the Run key.
pub const TEST_KEYS: &str = r"Software\OvermarkTest";

/// Observed startup sources. `None` means that source could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupState {
    pub per_user: Option<bool>,
    pub machine: Option<bool>,
}

impl StartupState {
    pub const OFF: Self = Self {
        per_user: Some(false),
        machine: Some(false),
    };
    pub const UNKNOWN: Self = Self {
        per_user: None,
        machine: None,
    };

    pub fn enabled(self) -> bool {
        self.per_user == Some(true) || self.machine == Some(true)
    }

    pub fn managed(self) -> bool {
        self.machine == Some(true)
    }

    pub fn changeable(self) -> bool {
        self.per_user.is_some() && self.machine == Some(false)
    }
}

trait Store {
    fn read(&self, machine: bool) -> io::Result<Option<String>>;
    fn write(&self, command: &str) -> io::Result<()>;
    fn remove(&self) -> io::Result<()>;
}

impl Store for AutoStart {
    fn read(&self, machine: bool) -> io::Result<Option<String>> {
        if machine {
            registry::get_machine_string(&self.key, &self.value)
        } else {
            self.command()
        }
    }
    fn write(&self, command: &str) -> io::Result<()> {
        registry::set_string(&self.key, &self.value, command)
    }
    fn remove(&self) -> io::Result<()> {
        self.disable()
    }
}

/// One app's entry in the Run key (or, in tests, in a key of their own).
#[derive(Debug, Clone)]
pub struct AutoStart {
    key: String,
    value: String,
    args: String,
}

impl AutoStart {
    /// The entry called `value` (`Overmark`), starting the exe with no arguments.
    pub fn new(value: &str) -> Self {
        Self {
            key: RUN_KEY.to_owned(),
            value: value.to_owned(),
            args: String::new(),
        }
    }

    /// Uses `key` in both hives instead of the Run key, for isolated tests.
    pub fn in_key(mut self, key: &str) -> Self {
        self.key = key.to_owned();
        self
    }

    /// That, starting the exe with `args` (`--frozen`).
    pub fn with_args(mut self, args: &str) -> Self {
        self.args = args.to_owned();
        self
    }

    /// The command it writes for `exe`.
    pub fn command_for(&self, exe: &Path) -> String {
        command_for(exe, &self.args)
    }

    /// The command it runs, if it's there.
    pub fn command(&self) -> io::Result<Option<String>> {
        registry::get_string(&self.key, &self.value)
    }

    /// The effective startup sources, preserving read failures as unknown.
    pub fn state(&self, exe: &Path) -> StartupState {
        self.state_in(exe, self)
    }

    fn state_in(&self, exe: &Path, store: &impl Store) -> StartupState {
        let matches = |machine| {
            store.read(machine).ok().map(|command| {
                command.is_some_and(|c| c.eq_ignore_ascii_case(&self.command_for(exe)))
            })
        };
        StartupState {
            per_user: matches(false),
            machine: matches(true),
        }
    }

    /// Whether either readable source starts this executable.
    pub fn starts(&self, exe: &Path) -> io::Result<bool> {
        let state = self.state(exe);
        if state.enabled() {
            Ok(true)
        } else if state.per_user.is_none() || state.machine.is_none() {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                overmark_core::i18n::tr("settings-autostart-unavailable"),
            ))
        } else {
            Ok(false)
        }
    }

    /// Starts `exe` when the user logs on.
    pub fn enable(&self, exe: &Path) -> io::Result<()> {
        registry::set_string(&self.key, &self.value, &self.command_for(exe))
    }

    /// Removes the per-user entry only. Not there already is fine.
    pub fn disable(&self) -> io::Result<()> {
        registry::delete_value(&self.key, &self.value).map(|_| ())
    }

    /// Makes the entry match `on`: there and starting `exe` with its arguments (rewritten if a
    /// moved copy runs now, or an older version wrote other arguments), or gone. It only ever
    /// removes an entry that starts an exe of `exe`'s file name, so an entry of the same name
    /// that belongs to something else is left alone.
    pub fn apply(&self, on: bool, exe: &Path) -> io::Result<()> {
        self.apply_in(on, exe, self)
    }

    fn apply_in(&self, on: bool, exe: &Path, store: &impl Store) -> io::Result<()> {
        // Read both sources before writing anything. Never claim to disable a machine entry.
        let machine = store.read(true)?;
        let user = store.read(false)?;
        let command = self.command_for(exe);
        if machine.is_some_and(|c| c.eq_ignore_ascii_case(&command)) {
            return if on {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    overmark_core::i18n::tr("settings-autostart-managed"),
                ))
            };
        }
        match (on, user) {
            (true, Some(c)) if c.eq_ignore_ascii_case(&command) => Ok(()),
            (true, _) => store.write(&command),
            (false, Some(c)) if same_file_name(&c, exe) => store.remove(),
            _ => Ok(()),
        }
    }
}

/// Whether `command` (as the Run key holds it, quoted or not) starts an exe named like `exe`.
fn same_file_name(command: &str, exe: &Path) -> bool {
    let path = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => command.split(' ').next().unwrap_or(""),
    };
    match (Path::new(path).file_name(), exe.file_name()) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// The exe, quoted (its folder may have spaces), then `args` if there are any.
pub fn command_for(exe: &Path, args: &str) -> String {
    match args {
        "" => format!("\"{}\"", exe.display()),
        _ => format!("\"{}\" {args}", exe.display()),
    }
}

/// A test's own key, `<TEST_KEYS>\<name>`, for [`AutoStart::in_key`].
pub fn test_key(name: &str) -> String {
    format!(r"{TEST_KEYS}\{name}")
}

/// Removes a test's key ([`test_key`]) with everything under it, and nothing else.
pub fn delete_test_key(name: &str) -> io::Result<()> {
    if name.is_empty() || name.contains(['\\', '/']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a test key's name",
        ));
    }
    registry::delete_tree(&test_key(name)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeStore {
        user: std::cell::RefCell<Option<String>>,
        machine: Option<String>,
        denied: Option<bool>,
        writes: std::cell::Cell<usize>,
    }
    impl Store for FakeStore {
        fn read(&self, machine: bool) -> io::Result<Option<String>> {
            if self.denied == Some(machine) {
                return Err(io::Error::from_raw_os_error(5));
            }
            Ok(if machine {
                self.machine.clone()
            } else {
                self.user.borrow().clone()
            })
        }
        fn write(&self, command: &str) -> io::Result<()> {
            self.writes.set(self.writes.get() + 1);
            *self.user.borrow_mut() = Some(command.to_owned());
            Ok(())
        }
        fn remove(&self) -> io::Result<()> {
            self.writes.set(self.writes.get() + 1);
            *self.user.borrow_mut() = None;
            Ok(())
        }
    }

    #[test]
    fn startup_sources_and_writes_use_both_hives_without_touching_machine_entries() {
        let entry = AutoStart::new(VALUE_NAME);
        let exe = Path::new(r"C:\Program Files\Overmark\overmark.exe");
        let command = entry.command_for(exe);
        let unrelated = r#""C:\Other\other.exe""#.to_owned();
        for (user, machine, expected) in [
            (None, None, StartupState::OFF),
            (
                Some(command.clone()),
                None,
                StartupState {
                    per_user: Some(true),
                    machine: Some(false),
                },
            ),
            (
                None,
                Some(command.clone()),
                StartupState {
                    per_user: Some(false),
                    machine: Some(true),
                },
            ),
            (
                Some(command.clone()),
                Some(command.to_uppercase()),
                StartupState {
                    per_user: Some(true),
                    machine: Some(true),
                },
            ),
            (
                Some(unrelated.clone()),
                Some(unrelated.clone()),
                StartupState::OFF,
            ),
        ] {
            let store = FakeStore {
                user: user.clone().into(),
                machine: machine.clone(),
                denied: None,
                writes: 0.into(),
            };
            assert_eq!(entry.state_in(exe, &store), expected);
            let disabled = entry.apply_in(false, exe, &store);
            assert_eq!(disabled.is_err(), expected.managed());
            if expected.managed() {
                assert_eq!(*store.user.borrow(), user);
                assert_eq!(store.writes.get(), 0);
                assert!(entry.state_in(exe, &store).enabled());
            } else {
                assert!(!entry.state_in(exe, &store).enabled());
                if user.as_ref() == Some(&unrelated) {
                    assert_eq!(*store.user.borrow(), user);
                }
            }
            entry.apply_in(true, exe, &store).unwrap();
            assert!(entry.state_in(exe, &store).enabled());
            assert_eq!(store.machine, machine);
            if expected.managed() {
                assert_eq!(store.writes.get(), 0);
            }
        }
    }

    #[test]
    fn denied_reads_are_unknown_and_prevent_writes_even_when_other_source_is_enabled() {
        let entry = AutoStart::new(VALUE_NAME);
        let exe = Path::new(r"C:\Overmark\overmark.exe");
        for denied in [false, true] {
            for other_enabled in [false, true] {
                let command = other_enabled.then(|| entry.command_for(exe));
                let store = FakeStore {
                    user: command.clone().into(),
                    machine: command,
                    denied: Some(denied),
                    writes: 0.into(),
                };
                let state = entry.state_in(exe, &store);
                assert!(!state.changeable());
                assert_eq!(state.enabled(), other_enabled);
                assert_eq!(
                    if denied {
                        state.machine
                    } else {
                        state.per_user
                    },
                    None
                );
                assert!(entry.apply_in(false, exe, &store).is_err());
                assert!(entry.apply_in(true, exe, &store).is_err());
                assert_eq!(store.writes.get(), 0);
            }
        }
    }

    /// A test's own key, removed when dropped (also in a failed assertion's unwinding).
    struct Own(String);
    impl Drop for Own {
        fn drop(&mut self) {
            let _ = delete_test_key(&self.0);
        }
    }

    /// The entry is the MSI's (the same name, the quoted exe, no arguments), turns on and off,
    /// is rewritten for a moved copy, and an entry of the same name for another exe is left
    /// alone. In a test-only key, never the Run key.
    #[test]
    fn turns_on_and_off_in_a_key_of_its_own() {
        let own = Own(format!("{}-autostart", std::process::id()));
        let entry = AutoStart::new(VALUE_NAME).in_key(&test_key(&own.0));
        let exe = Path::new(r"C:\Program Files\Overmark\overmark.exe");
        assert_eq!(
            entry.command_for(exe),
            r#""C:\Program Files\Overmark\overmark.exe""#
        );

        assert_eq!(entry.command().unwrap(), None);
        assert!(!entry.starts(exe).unwrap());
        entry.apply(true, exe).unwrap();
        assert!(entry.starts(exe).unwrap());
        // A moved portable copy: the entry follows it.
        let moved = Path::new(r"D:\Tools\Overmark\overmark.exe");
        assert!(!entry.starts(moved).unwrap());
        entry.apply(true, moved).unwrap();
        assert!(entry.starts(moved).unwrap());
        entry.apply(false, moved).unwrap();
        assert_eq!(entry.command().unwrap(), None);
        // Off when it's already off is fine.
        entry.apply(false, moved).unwrap();

        // Something else's entry of the same name stays.
        registry::set_string(&test_key(&own.0), VALUE_NAME, r#""C:\Other\other.exe""#).unwrap();
        entry.apply(false, exe).unwrap();
        assert_eq!(
            entry.command().unwrap().as_deref(),
            Some(r#""C:\Other\other.exe""#)
        );
    }

    #[test]
    fn only_test_keys_can_be_removed() {
        assert!(delete_test_key("").is_err());
        assert!(delete_test_key(r"..\Run").is_err());
        assert!(delete_test_key("a/b").is_err());
    }
}
