//! The keymap. One table drives key dispatch, the hint line, `?` help and `outlay keys`; no other
//! module names a key.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::model::geometry::Dir;

/// A key after [`normalise`]: letters carry their case instead of SHIFT, and Shift-Tab is always
/// `BackTab`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Key {
    pub const fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self { code, mods }
    }

    pub const fn char(c: char) -> Self {
        Self::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
}

/// Turns a terminal key event into a [`Key`], or `None` for a key release.
///
/// - `Char('h')` with SHIFT becomes `Char('H')`, and SHIFT is dropped from every other character,
///   so `:` arrives the same whether or not the terminal reports the SHIFT it took to type it;
/// - `BackTab` and `Tab` with SHIFT are the same key;
/// - only SHIFT, CONTROL and ALT are kept.
pub fn normalise(ev: &KeyEvent) -> Option<Key> {
    if ev.kind == KeyEventKind::Release {
        return None;
    }
    let mut mods = ev.modifiers & (KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);
    let code = match ev.code {
        KeyCode::Char(c) => {
            mods.remove(KeyModifiers::SHIFT);
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) if shift && c.is_lowercase() => KeyCode::Char(u),
                _ => KeyCode::Char(c),
            }
        }
        KeyCode::Tab if shift => {
            mods.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        KeyCode::BackTab => {
            mods.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        other => other,
    };
    Some(Key { code, mods })
}

/// Where a binding applies: the editor itself or one of its modes and popups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    Normal,
    StickTarget,
    StickSide,
    Picker,
    Command,
    Confirm,
    Help,
}

impl Context {
    pub const ALL: [Context; 7] = [
        Context::Normal,
        Context::StickTarget,
        Context::StickSide,
        Context::Picker,
        Context::Command,
        Context::Confirm,
        Context::Help,
    ];

    /// The section title in `?` help and `outlay keys`.
    pub fn title(self) -> &'static str {
        match self {
            Context::Normal => "Layout",
            Context::StickTarget => "Stick, step 1: target",
            Context::StickSide => "Stick, step 2: side",
            Context::Picker => "Mode and rate pickers",
            Context::Command => "Command line",
            Context::Confirm => "Questions",
            Context::Help => "Help",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Focus(Dir),
    FocusNext,
    FocusPrev,
    FocusNumber(usize),
    Snap(Dir),
    Nudge(Dir),
    Stick,
    Unstick,
    ModePicker,
    Smaller,
    Larger,
    RatePicker,
    SlowerRate,
    FasterRate,
    RotateCw,
    RotateCcw,
    Primary,
    Toggle,
    Undo,
    Redo,
    Reload,
    Command,
    Refit,
    Details,
    Help,
    Quit,
    /// Stick flow: move the target selection spatially, cycle it, or pick it by number.
    Target(Dir),
    TargetNext,
    TargetPrev,
    TargetNumber(usize),
    /// Stick flow: choose the side, a mirror, or the alignment along the shared edge.
    Side(Dir),
    Mirror,
    AlignNext,
    AlignPrev,
    /// Move a list selection, or scroll help.
    Move(Dir),
    Accept,
    Cancel,
    Back,
}

/// A set of keys one binding answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    /// The four direction letters (`h j k l` by default). SHIFT means the upper-case letters.
    Letters(KeyModifiers),
    Arrows(KeyModifiers),
    /// The down and up letters only.
    VerticalLetters,
    VerticalArrows,
    /// `1` to `9`.
    Digits,
    One(KeyCode, KeyModifiers),
}

/// What a binding does with the key it matched.
#[derive(Clone, Copy, Debug)]
pub enum Does {
    Act(Action),
    /// Keys that name a direction.
    Dir(fn(Dir) -> Action),
    /// Digit keys.
    Num(fn(usize) -> Action),
}

#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub context: Context,
    pub keys: &'static [Keys],
    pub does: Does,
    /// The label in the hint line; bindings without one appear only in help.
    pub hint: Option<&'static str>,
    pub help: &'static str,
}

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const ALT: KeyModifiers = KeyModifiers::ALT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;

const fn ch(c: char) -> Keys {
    Keys::One(KeyCode::Char(c), NONE)
}

const fn code(code: KeyCode) -> Keys {
    Keys::One(code, NONE)
}

const fn bind(
    context: Context,
    keys: &'static [Keys],
    does: Does,
    hint: Option<&'static str>,
    help: &'static str,
) -> Binding {
    Binding {
        context,
        keys,
        does,
        hint,
        help,
    }
}

use Action as A;
use Context as C;

const fn act(a: Action) -> Does {
    Does::Act(a)
}

/// Every key binding. The order is the order of the hint line and of help.
#[rustfmt::skip]
pub const TABLE: &[Binding] = &[
    bind(C::Normal, &[Keys::Letters(NONE), Keys::Arrows(NONE)], Does::Dir(A::Focus),
        Some("focus"), "Focus the nearest display in that direction"),
    bind(C::Normal, &[Keys::Letters(SHIFT), Keys::Arrows(SHIFT)], Does::Dir(A::Snap),
        Some("move"), "Snap-move: swap with a neighbour or slide to the next stop"),
    bind(C::Normal, &[Keys::Letters(ALT), Keys::Arrows(ALT)], Does::Dir(A::Nudge),
        None, "Nudge by the step, ignoring gaps and overlaps"),
    bind(C::Normal, &[ch('s')], act(A::Stick), Some("stick"), "Stick to a side of another display"),
    bind(C::Normal, &[ch('S')], act(A::Unstick), None, "Unstick: stay in place, follow nothing"),
    bind(C::Normal, &[ch('m')], act(A::ModePicker), Some("mode"), "Pick a resolution"),
    bind(C::Normal, &[ch('[')], act(A::Smaller), None, "Next smaller resolution"),
    bind(C::Normal, &[ch(']')], act(A::Larger), None, "Next larger resolution"),
    bind(C::Normal, &[ch('r')], act(A::RatePicker), Some("rate"), "Pick a refresh rate"),
    bind(C::Normal, &[ch('{')], act(A::SlowerRate), None, "Next lower refresh rate"),
    bind(C::Normal, &[ch('}')], act(A::FasterRate), None, "Next higher refresh rate"),
    bind(C::Normal, &[ch('o')], act(A::RotateCw), Some("rotate"), "Rotate clockwise"),
    bind(C::Normal, &[ch('O')], act(A::RotateCcw), None, "Rotate counter-clockwise"),
    bind(C::Normal, &[ch('p')], act(A::Primary), None, "Make primary"),
    bind(C::Normal, &[ch(' ')], act(A::Toggle), Some("on/off"), "Turn the display on or off"),
    bind(C::Normal, &[ch('u')], act(A::Undo), Some("undo"), "Undo"),
    bind(C::Normal, &[Keys::One(KeyCode::Char('r'), CTRL)], act(A::Redo), None, "Redo"),
    bind(C::Normal, &[code(KeyCode::Tab)], act(A::FocusNext), None, "Focus the next display"),
    bind(C::Normal, &[code(KeyCode::BackTab)], act(A::FocusPrev), None,
        "Focus the previous display"),
    bind(C::Normal, &[Keys::Digits], Does::Num(A::FocusNumber), None,
        "Focus display N, on or off"),
    bind(C::Normal, &[ch(':')], act(A::Command), None, "Command line"),
    bind(C::Normal, &[ch('R')], act(A::Reload), None, "Reload the live state"),
    bind(C::Normal, &[ch('z')], act(A::Refit), None, "Fit the view to the layout"),
    bind(C::Normal, &[ch('i')], act(A::Details), None, "Show or hide the details panel"),
    bind(C::Normal, &[ch('?')], act(A::Help), Some("help"), "Help"),
    bind(C::Normal, &[ch('q'), Keys::One(KeyCode::Char('c'), CTRL)], act(A::Quit), None,
        "Quit (asks first when changes are pending)"),

    bind(C::StickTarget, &[Keys::Digits], Does::Num(A::TargetNumber), Some("target"),
        "Stick to display N and go on to the side"),
    bind(C::StickTarget, &[Keys::Letters(NONE), Keys::Arrows(NONE)], Does::Dir(A::Target),
        Some("select"), "Select the nearest display in that direction"),
    bind(C::StickTarget, &[code(KeyCode::Tab)], act(A::TargetNext), None,
        "Select the next display"),
    bind(C::StickTarget, &[code(KeyCode::BackTab)], act(A::TargetPrev), None,
        "Select the previous display"),
    bind(C::StickTarget, &[code(KeyCode::Enter)], act(A::Accept), Some("choose"),
        "Go on to the side"),
    bind(C::StickTarget, &[code(KeyCode::Esc)], act(A::Cancel), Some("cancel"), "Cancel"),

    bind(C::StickSide, &[Keys::Letters(NONE), Keys::Arrows(NONE)], Does::Dir(A::Side),
        Some("side"), "Left of, below, above or right of the target"),
    bind(C::StickSide, &[ch('=')], act(A::Mirror), Some("mirror"), "Mirror the target (same-as)"),
    bind(C::StickSide, &[code(KeyCode::Tab)], act(A::AlignNext), Some("align"),
        "Next alignment along the shared edge"),
    bind(C::StickSide, &[code(KeyCode::BackTab)], act(A::AlignPrev), None,
        "Previous alignment"),
    bind(C::StickSide, &[code(KeyCode::Enter)], act(A::Accept), Some("stick"), "Stick"),
    bind(C::StickSide, &[code(KeyCode::Backspace)], act(A::Back), Some("back"),
        "Back to the target"),
    bind(C::StickSide, &[code(KeyCode::Esc)], act(A::Cancel), Some("cancel"), "Cancel"),

    bind(C::Picker, &[Keys::VerticalLetters, Keys::VerticalArrows], Does::Dir(A::Move),
        Some("select"), "Move the selection"),
    bind(C::Picker, &[code(KeyCode::Enter)], act(A::Accept), Some("choose"), "Choose"),
    bind(C::Picker, &[code(KeyCode::Esc)], act(A::Cancel), Some("cancel"), "Cancel"),

    bind(C::Command, &[code(KeyCode::Enter)], act(A::Accept), Some("run"), "Run the command"),
    bind(C::Command, &[code(KeyCode::Backspace)], act(A::Back), None,
        "Delete a character; on an empty line, cancel"),
    bind(C::Command, &[code(KeyCode::Esc)], act(A::Cancel), Some("cancel"), "Cancel"),

    bind(C::Confirm, &[code(KeyCode::Enter), ch('y')], act(A::Accept), Some("yes"), "Yes"),
    bind(C::Confirm, &[code(KeyCode::Esc), ch('n')], act(A::Cancel), Some("no"), "No"),

    bind(C::Help, &[Keys::VerticalLetters, Keys::VerticalArrows], Does::Dir(A::Move),
        Some("scroll"), "Scroll"),
    bind(C::Help, &[code(KeyCode::Esc), ch('q'), ch('?')], act(A::Cancel), Some("close"),
        "Close help"),
];

/// The keymap with the configured direction letters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    /// Left, down, up, right.
    letters: [char; 4],
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            letters: ['h', 'j', 'k', 'l'],
        }
    }
}

impl Keymap {
    /// A keymap with other direction letters (left, down, up, right). They must be four distinct
    /// lower-case ASCII letters that no other binding uses.
    pub fn new(letters: &str) -> Result<Self, String> {
        let chars: Vec<char> = letters.chars().collect();
        let [l, d, u, r] = chars[..] else {
            return Err(format!(
                "directions must be four letters (left, down, up, right), not {letters:?}"
            ));
        };
        let letters = [l, d, u, r];
        if let Some(c) = letters.iter().find(|c| !c.is_ascii_lowercase()) {
            return Err(format!(
                "directions must be lower-case letters; {c:?} is not"
            ));
        }
        if (1..4).any(|k| letters[..k].contains(&letters[k])) {
            return Err(format!("directions repeats a letter: {letters:?}"));
        }
        let map = Self { letters };
        for ctx in Context::ALL {
            let bindings = || TABLE.iter().filter(move |b| b.context == ctx);
            let taken: Vec<Key> = bindings()
                .flat_map(|b| b.keys.iter())
                .filter(|k| matches!(k, Keys::Letters(_) | Keys::VerticalLetters))
                .flat_map(|k| map.expand(k))
                .map(|(key, _)| key)
                .collect();
            for b in bindings() {
                for keys in b.keys {
                    if let Keys::One(code, mods) = *keys
                        && taken.contains(&Key::new(code, mods))
                    {
                        return Err(format!(
                            "directions: {} is already bound to \"{}\"",
                            long_key(code, mods),
                            b.help
                        ));
                    }
                }
            }
        }
        Ok(map)
    }

    pub fn letter(&self, dir: Dir) -> char {
        self.letters[Dir::ALL.iter().position(|&d| d == dir).expect("four dirs")]
    }

    /// The concrete keys a key set stands for, each with the direction or digit it names.
    fn expand(&self, keys: &Keys) -> Vec<(Key, Arg)> {
        let letters = |mods: KeyModifiers, only: &[Dir]| {
            only.iter()
                .map(|&d| {
                    let c = self.letter(d);
                    let key = if mods.contains(SHIFT) {
                        Key::new(KeyCode::Char(c.to_ascii_uppercase()), mods - SHIFT)
                    } else {
                        Key::new(KeyCode::Char(c), mods)
                    };
                    (key, Arg::Dir(d))
                })
                .collect::<Vec<_>>()
        };
        let arrows = |mods: KeyModifiers, only: &[Dir]| {
            only.iter()
                .map(|&d| (Key::new(arrow(d), mods), Arg::Dir(d)))
                .collect::<Vec<_>>()
        };
        const VERTICAL: [Dir; 2] = [Dir::Down, Dir::Up];
        match *keys {
            Keys::Letters(mods) => letters(mods, &Dir::ALL),
            Keys::Arrows(mods) => arrows(mods, &Dir::ALL),
            Keys::VerticalLetters => letters(NONE, &VERTICAL),
            Keys::VerticalArrows => arrows(NONE, &VERTICAL),
            Keys::Digits => (1..=9)
                .map(|n| {
                    let c = char::from_digit(n, 10).expect("a digit");
                    (Key::char(c), Arg::Num(n as usize))
                })
                .collect(),
            Keys::One(code, mods) => vec![(Key::new(code, mods), Arg::None)],
        }
    }

    /// The action `key` triggers in `context`.
    pub fn lookup(&self, context: Context, key: Key) -> Option<Action> {
        TABLE.iter().filter(|b| b.context == context).find_map(|b| {
            b.keys.iter().find_map(|keys| {
                let (_, arg) = self.expand(keys).into_iter().find(|(k, _)| *k == key)?;
                Some(match (b.does, arg) {
                    (Does::Act(a), _) => a,
                    (Does::Dir(f), Arg::Dir(d)) => f(d),
                    (Does::Num(f), Arg::Num(n)) => f(n),
                    _ => return None,
                })
            })
        })
    }

    /// The short label of a key set, as the hint line shows it: `hjkl`, `S-←↓↑→`, `␣`.
    pub fn short(&self, keys: &Keys) -> String {
        let letters: String = self.letters.iter().collect();
        match *keys {
            Keys::Letters(mods) if mods.contains(SHIFT) => letters.to_ascii_uppercase(),
            Keys::Letters(mods) => format!("{}{letters}", short_mods(mods)),
            Keys::Arrows(mods) => format!("{}←↓↑→", short_mods(mods)),
            Keys::VerticalLetters => [self.letter(Dir::Down), self.letter(Dir::Up)]
                .iter()
                .collect(),
            Keys::VerticalArrows => "↓↑".to_owned(),
            Keys::Digits => "1-9".to_owned(),
            Keys::One(code, mods) => format!("{}{}", short_mods(mods), short_code(code)),
        }
    }

    /// The long label of a key set, as help shows it: `h j k l`, `Shift-arrows`, `Space`.
    pub fn long(&self, keys: &Keys) -> String {
        let spaced = |cs: &[char]| cs.iter().map(char::to_string).collect::<Vec<_>>().join(" ");
        match *keys {
            Keys::Letters(mods) if mods.contains(SHIFT) => {
                spaced(&self.letters.map(|c| c.to_ascii_uppercase()))
            }
            Keys::Letters(mods) => format!("{}{}", long_mods(mods), spaced(&self.letters)),
            Keys::Arrows(mods) => format!("{}arrows", long_mods(mods)),
            Keys::VerticalLetters => spaced(&[self.letter(Dir::Down), self.letter(Dir::Up)]),
            Keys::VerticalArrows => "↓ ↑".to_owned(),
            Keys::Digits => "1-9".to_owned(),
            Keys::One(code, mods) => long_key(code, mods),
        }
    }

    /// The hint line entries of `context`: the first key set of each binding with a hint label,
    /// for the actions `show` accepts.
    pub fn hints(
        &self,
        context: Context,
        show: impl Fn(&Binding) -> bool,
    ) -> Vec<(String, &'static str)> {
        TABLE
            .iter()
            .filter(|b| b.context == context && show(b))
            .filter_map(|b| b.hint.map(|h| (self.short(&b.keys[0]), h)))
            .collect()
    }

    /// Help rows of `context`: every key set of a binding, then what it does.
    pub fn help(&self, context: Context) -> Vec<(String, &'static str)> {
        TABLE
            .iter()
            .filter(|b| b.context == context)
            .map(|b| {
                let keys: Vec<String> = b.keys.iter().map(|k| self.long(k)).collect();
                (keys.join(" / "), b.help)
            })
            .collect()
    }

    /// The short label of the key bound to `action` in `context`, for messages such as
    /// "Alt-l nudges freely".
    pub fn key_for(&self, context: Context, action: Action) -> Option<String> {
        TABLE.iter().filter(|b| b.context == context).find_map(|b| {
            b.keys.iter().find_map(|keys| {
                self.expand(keys).into_iter().find_map(|(key, arg)| {
                    let a = match (b.does, arg) {
                        (Does::Act(a), _) => a,
                        (Does::Dir(f), Arg::Dir(d)) => f(d),
                        (Does::Num(f), Arg::Num(n)) => f(n),
                        _ => return None,
                    };
                    (a == action).then(|| long_key(key.code, key.mods))
                })
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arg {
    None,
    Dir(Dir),
    Num(usize),
}

fn arrow(dir: Dir) -> KeyCode {
    match dir {
        Dir::Left => KeyCode::Left,
        Dir::Down => KeyCode::Down,
        Dir::Up => KeyCode::Up,
        Dir::Right => KeyCode::Right,
    }
}

fn short_mods(mods: KeyModifiers) -> String {
    let mut s = String::new();
    if mods.contains(CTRL) {
        s.push_str("C-");
    }
    if mods.contains(ALT) {
        s.push_str("A-");
    }
    if mods.contains(SHIFT) {
        s.push_str("S-");
    }
    s
}

fn long_mods(mods: KeyModifiers) -> String {
    let mut s = String::new();
    if mods.contains(CTRL) {
        s.push_str("Ctrl-");
    }
    if mods.contains(ALT) {
        s.push_str("Alt-");
    }
    if mods.contains(SHIFT) {
        s.push_str("Shift-");
    }
    s
}

fn short_code(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "␣".to_owned(),
        KeyCode::Enter => "⏎".to_owned(),
        KeyCode::Backspace => "⌫".to_owned(),
        KeyCode::BackTab => "S-Tab".to_owned(),
        other => long_code(other),
    }
}

fn long_code(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::BackTab => "Shift-Tab".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Left => "←".to_owned(),
        KeyCode::Right => "→".to_owned(),
        KeyCode::Up => "↑".to_owned(),
        KeyCode::Down => "↓".to_owned(),
        other => format!("{other:?}"),
    }
}

fn long_key(code: KeyCode, mods: KeyModifiers) -> String {
    format!("{}{}", long_mods(mods), long_code(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn normaliser_folds_shift_into_the_character() {
        let n = |code, mods| normalise(&ev(code, mods)).unwrap();
        assert_eq!(n(KeyCode::Char('h'), SHIFT), Key::char('H'));
        assert_eq!(n(KeyCode::Char('H'), SHIFT), Key::char('H'));
        assert_eq!(n(KeyCode::Char('H'), NONE), Key::char('H'));
        assert_eq!(n(KeyCode::Char(':'), SHIFT), Key::char(':'));
        assert_eq!(n(KeyCode::Char('{'), SHIFT), Key::char('{'));
        assert_eq!(
            n(KeyCode::Char('h'), ALT),
            Key::new(KeyCode::Char('h'), ALT)
        );
        assert_eq!(n(KeyCode::Tab, SHIFT), Key::new(KeyCode::BackTab, NONE));
        assert_eq!(n(KeyCode::BackTab, SHIFT), Key::new(KeyCode::BackTab, NONE));
        assert_eq!(
            n(KeyCode::Left, SHIFT),
            Key::new(KeyCode::Left, SHIFT),
            "Shift-arrows keep SHIFT"
        );
        assert_eq!(
            n(KeyCode::Char('r'), CTRL | KeyModifiers::SUPER),
            Key::new(KeyCode::Char('r'), CTRL)
        );
    }

    #[test]
    fn press_and_repeat_count_but_release_does_not() {
        let mut e = ev(KeyCode::Char('j'), NONE);
        e.kind = KeyEventKind::Repeat;
        assert_eq!(normalise(&e), Some(Key::char('j')));
        e.kind = KeyEventKind::Release;
        assert_eq!(normalise(&e), None);
    }

    #[test]
    fn lookup_expands_letters_arrows_and_digits() {
        let map = Keymap::default();
        let look = |ctx, key| map.lookup(ctx, key);
        assert_eq!(
            look(C::Normal, Key::char('h')),
            Some(Action::Focus(Dir::Left))
        );
        assert_eq!(
            look(C::Normal, Key::char('L')),
            Some(Action::Snap(Dir::Right))
        );
        assert_eq!(
            look(C::Normal, Key::new(KeyCode::Char('j'), ALT)),
            Some(Action::Nudge(Dir::Down))
        );
        assert_eq!(
            look(C::Normal, Key::new(KeyCode::Up, SHIFT)),
            Some(Action::Snap(Dir::Up))
        );
        assert_eq!(
            look(C::Normal, Key::char('7')),
            Some(Action::FocusNumber(7))
        );
        assert_eq!(
            look(C::StickTarget, Key::char('2')),
            Some(Action::TargetNumber(2))
        );
        assert_eq!(
            look(C::StickSide, Key::char('h')),
            Some(Action::Side(Dir::Left))
        );
        assert_eq!(look(C::Picker, Key::char('h')), None, "only j and k");
        assert_eq!(look(C::Picker, Key::char('k')), Some(Action::Move(Dir::Up)));
        assert_eq!(look(C::Normal, Key::char('x')), None);
    }

    #[test]
    fn no_key_is_bound_twice_in_one_context() {
        let map = Keymap::default();
        for ctx in Context::ALL {
            let mut seen = Vec::new();
            for b in TABLE.iter().filter(|b| b.context == ctx) {
                for keys in b.keys {
                    for (key, _) in map.expand(keys) {
                        assert!(!seen.contains(&key), "{key:?} twice in {ctx:?}");
                        seen.push(key);
                    }
                }
            }
        }
    }

    #[test]
    fn control_keys_that_alias_backspace_enter_and_tab_stay_unbound() {
        let map = Keymap::default();
        for ctx in Context::ALL {
            for c in ['h', 'j', 'i', 'm'] {
                assert_eq!(map.lookup(ctx, Key::new(KeyCode::Char(c), CTRL)), None);
            }
        }
    }

    #[test]
    fn direction_letters_can_change() {
        let map = Keymap::new("fgnt").unwrap();
        assert_eq!(
            map.lookup(C::Normal, Key::char('n')),
            Some(Action::Focus(Dir::Up))
        );
        assert_eq!(
            map.lookup(C::Normal, Key::char('F')),
            Some(Action::Snap(Dir::Left))
        );
        assert_eq!(map.lookup(C::Normal, Key::char('h')), None);
        assert_eq!(map.short(&Keys::Letters(NONE)), "fgnt");
        assert!(Keymap::new("hjk").unwrap_err().contains("four letters"));
        assert!(Keymap::new("hjkh").unwrap_err().contains("repeats"));
        assert!(Keymap::new("hjK;").unwrap_err().contains("lower-case"));
        assert_eq!(
            Keymap::new("jkil").unwrap_err(),
            "directions: i is already bound to \"Show or hide the details panel\""
        );
        let clash = Keymap::new("hjkq").unwrap_err();
        assert!(clash.contains("q is already bound"), "{clash}");
    }

    #[test]
    fn labels_for_hints_help_and_messages() {
        let map = Keymap::default();
        let hints = map.hints(C::Normal, |_| true);
        let line: Vec<String> = hints.iter().map(|(k, h)| format!("{k} {h}")).collect();
        assert_eq!(
            line.join(" · "),
            "hjkl focus · HJKL move · s stick · m mode · r rate · o rotate · ␣ on/off · u undo · ? help"
        );
        let help = map.help(C::Normal);
        assert_eq!(
            help[0],
            (
                "h j k l / arrows".to_owned(),
                "Focus the nearest display in that direction"
            )
        );
        assert_eq!(help[2].0, "Alt-h j k l / Alt-arrows");
        assert!(help.iter().any(|(k, _)| k == "q / Ctrl-c"));
        assert_eq!(
            map.key_for(C::Normal, Action::Nudge(Dir::Right)).as_deref(),
            Some("Alt-l")
        );
    }
}
