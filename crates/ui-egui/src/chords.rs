//! The copy, cut and paste chords as a window really delivers them.
//!
//! egui-winit does not hand these chords over as key presses. On the press
//! of Mod+C, Mod+X or Mod+V — with any further modifier, so Mod+Shift+C
//! too — and of the Copy, Cut and Paste keys, and on Windows of Ctrl+Insert,
//! Shift+Delete and Shift+Insert, it pushes `Event::Copy`, `Event::Cut` or
//! `Event::Paste(text)` and drops the key. Only the key's release arrives
//! as a key event.
//!
//! `Event::Paste` comes only when the clipboard holds text. With files on
//! the clipboard and no text, or with nothing on it, the press of a paste
//! chord leaves no event at all. So a paste is told by its release: the
//! release of a paste key whose press never arrived is a paste chord. That
//! holds whatever the clipboard holds, and it is one paste per press where
//! a held key would repeat the event.
//!
//! [`ClipboardChords`] turns what arrives back into the chord that was
//! pressed, for the keymap to look up — so a chord the user has bound to
//! something else, or to nothing, does that.

use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers};

/// How long after Mod was last seen held a paste key's release still counts
/// as the chord: Mod is often let go a moment before the key.
const MOD_LINGERS: Duration = Duration::from_millis(1500);

/// A chord as the keymap knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chord {
    /// The modifiers held.
    pub(crate) modifiers: Modifiers,
    /// The key.
    pub(crate) key: Key,
}

/// The paste keys: their presses are tracked, so a release without one is
/// known for a swallowed press.
const PASTE_KEYS: [Key; 3] = [Key::V, Key::Insert, Key::Paste];

/// What one window has seen of the clipboard chords.
#[derive(Debug, Default)]
pub(crate) struct ClipboardChords {
    /// Per paste key: whether its press arrived as a key event.
    pressed: [bool; 3],
    /// The modifiers when Mod was last seen held, and when that was.
    with_mod: Option<(Instant, Modifiers)>,
    /// The paste chord completed in the frame last tracked.
    paste: Option<Chord>,
}

impl ClipboardChords {
    /// Follows the frame's events, whoever has the keyboard: which paste
    /// keys went down as keys, and whether a paste chord was completed.
    /// Nothing is taken from the events.
    pub(crate) fn track(&mut self, input: &egui::InputState, now: Instant) {
        self.paste = None;
        if input.modifiers.command {
            self.with_mod = Some((now, input.modifiers));
        }
        for event in &input.events {
            let Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            let Some(index) = PASTE_KEYS.iter().position(|k| k == key) else {
                continue;
            };
            if *pressed {
                self.pressed[index] = true;
                continue;
            }
            // A release: of a press that arrived, or of one that was
            // swallowed as a paste.
            if std::mem::take(&mut self.pressed[index]) {
                continue;
            }
            self.paste = self.paste_chord(*key, *modifiers, now);
        }
    }

    /// The chord a swallowed press of `key` stood for, from the modifiers
    /// at its release; `None` when it was no paste.
    fn paste_chord(&self, key: Key, at_release: Modifiers, now: Instant) -> Option<Chord> {
        let modifiers = match key {
            // Mod+V, with Shift or Alt as held. Mod may be up already.
            Key::V if at_release.command => at_release,
            Key::V => {
                let (seen, modifiers) = self.with_mod?;
                if now.duration_since(seen) > MOD_LINGERS {
                    return None;
                }
                modifiers
            }
            // Shift+Insert on Windows; Ctrl+Insert there is a copy.
            Key::Insert if at_release.shift && !at_release.command => Modifiers::COMMAND,
            Key::Insert => return None,
            _ => Modifiers::COMMAND,
        };
        Some(Chord {
            modifiers,
            key: Key::V,
        })
    }

    /// The chords of the frame, for a list that has the keyboard: the copy
    /// and cut events as the chords they stand for, and the paste chord
    /// [`track`](Self::track) found. The three kinds of event are taken out
    /// of `input`; a paste event is dropped, its release being the chord.
    pub(crate) fn take(&mut self, input: &mut egui::InputState) -> Vec<Chord> {
        let held = input.modifiers;
        let mut chords = Vec::new();
        input.events.retain(|event| match event {
            Event::Copy => {
                chords.push(Chord {
                    modifiers: with_mod(held),
                    key: Key::C,
                });
                false
            }
            Event::Cut => {
                // Shift+Delete on Windows arrives as a cut.
                let chord = if !held.command && held.shift {
                    Chord {
                        modifiers: held,
                        key: Key::Delete,
                    }
                } else {
                    Chord {
                        modifiers: with_mod(held),
                        key: Key::X,
                    }
                };
                chords.push(chord);
                false
            }
            Event::Paste(_) => false,
            _ => true,
        });
        chords.extend(self.paste.take());
        chords
    }
}

/// `held` when Mod is in it, else Mod alone: the Copy and Cut keys, which
/// need no modifier.
fn with_mod(held: Modifiers) -> Modifiers {
    if held.command {
        held
    } else {
        Modifiers::COMMAND
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(modifiers: Modifiers, events: Vec<Event>) -> egui::InputState {
        let mut input = egui::InputState::default();
        input.modifiers = modifiers;
        input.events = events;
        input
    }

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn paste(modifiers: Modifiers) -> Chord {
        Chord {
            modifiers,
            key: Key::V,
        }
    }

    /// One frame: tracked, then taken as the list would.
    fn frame(
        chords: &mut ClipboardChords,
        now: Instant,
        modifiers: Modifiers,
        events: Vec<Event>,
    ) -> (Vec<Chord>, Vec<Event>) {
        let mut input = input(modifiers, events);
        chords.track(&input, now);
        let taken = chords.take(&mut input);
        (taken, input.events)
    }

    #[test]
    fn copy_and_cut_events_are_the_chords_they_stand_for() {
        let mut chords = ClipboardChords::default();
        let now = Instant::now();
        let (taken, left) = frame(&mut chords, now, Modifiers::COMMAND, vec![Event::Copy]);
        assert_eq!(
            taken,
            [Chord {
                modifiers: Modifiers::COMMAND,
                key: Key::C
            }]
        );
        assert_eq!(left, Vec::new(), "the event is taken");

        // Mod+Shift+C is swallowed as a copy too: the modifiers tell it.
        let both = Modifiers::COMMAND | Modifiers::SHIFT;
        let (taken, _) = frame(&mut chords, now, both, vec![Event::Copy]);
        assert_eq!(
            taken,
            [Chord {
                modifiers: both,
                key: Key::C
            }]
        );

        let (taken, _) = frame(&mut chords, now, Modifiers::COMMAND, vec![Event::Cut]);
        assert_eq!(
            taken,
            [Chord {
                modifiers: Modifiers::COMMAND,
                key: Key::X
            }]
        );
        // Shift+Delete on Windows.
        let (taken, _) = frame(&mut chords, now, Modifiers::SHIFT, vec![Event::Cut]);
        assert_eq!(
            taken,
            [Chord {
                modifiers: Modifiers::SHIFT,
                key: Key::Delete
            }]
        );
        // The Copy key, with nothing held.
        let (taken, _) = frame(&mut chords, now, Modifiers::NONE, vec![Event::Copy]);
        assert_eq!(
            taken,
            [Chord {
                modifiers: Modifiers::COMMAND,
                key: Key::C
            }]
        );
    }

    #[test]
    fn a_paste_is_the_release_of_a_press_that_never_arrived() {
        let mut chords = ClipboardChords::default();
        let now = Instant::now();
        // The clipboard holds no text: the press leaves nothing.
        let (taken, _) = frame(&mut chords, now, Modifiers::COMMAND, Vec::new());
        assert_eq!(taken, Vec::new());
        let release = key(Key::V, false, Modifiers::COMMAND);
        let (taken, left) = frame(&mut chords, now, Modifiers::COMMAND, vec![release.clone()]);
        assert_eq!(taken, [paste(Modifiers::COMMAND)]);
        assert_eq!(left, [release], "the release stays a key event");
        // And nothing lingers into the next frame.
        let (taken, _) = frame(&mut chords, now, Modifiers::NONE, Vec::new());
        assert_eq!(taken, Vec::new());
    }

    #[test]
    fn a_paste_event_is_dropped_and_the_release_pastes_once() {
        let mut chords = ClipboardChords::default();
        let now = Instant::now();
        // The clipboard holds text, and the key is held: the event repeats.
        for _ in 0..3 {
            let (taken, left) = frame(
                &mut chords,
                now,
                Modifiers::COMMAND,
                vec![Event::Paste("some text".to_owned())],
            );
            assert_eq!(taken, Vec::new(), "the event is not the chord");
            assert_eq!(left, Vec::new(), "and is taken");
        }
        let (taken, _) = frame(
            &mut chords,
            now,
            Modifiers::COMMAND,
            vec![key(Key::V, false, Modifiers::COMMAND)],
        );
        assert_eq!(taken, [paste(Modifiers::COMMAND)], "one paste per press");
    }

    #[test]
    fn mod_let_go_before_the_key_is_still_the_chord() {
        let mut chords = ClipboardChords::default();
        let start = Instant::now();
        let both = Modifiers::COMMAND | Modifiers::SHIFT;
        // Mod+Shift held, V pressed (swallowed), both let go, then V.
        frame(&mut chords, start, both, Vec::new());
        let later = start + Duration::from_millis(120);
        let (taken, _) = frame(
            &mut chords,
            later,
            Modifiers::NONE,
            vec![key(Key::V, false, Modifiers::NONE)],
        );
        assert_eq!(taken, [paste(both)], "with the modifiers Mod was held with");

        // Long after Mod was held, a lone release is no paste: a key that
        // went down in another window and came up in this one.
        let much_later = later + Duration::from_secs(5);
        let (taken, _) = frame(
            &mut chords,
            much_later,
            Modifiers::NONE,
            vec![key(Key::V, false, Modifiers::NONE)],
        );
        assert_eq!(taken, Vec::new());
    }

    #[test]
    fn a_typed_v_is_no_paste() {
        let mut chords = ClipboardChords::default();
        let now = Instant::now();
        // Mod was held a moment ago, for another chord.
        frame(&mut chords, now, Modifiers::COMMAND, Vec::new());
        let press = key(Key::V, true, Modifiers::NONE);
        let (taken, left) = frame(&mut chords, now, Modifiers::NONE, vec![press.clone()]);
        assert_eq!(taken, Vec::new());
        assert_eq!(left, [press], "the press is the list's to match");
        let (taken, _) = frame(
            &mut chords,
            now,
            Modifiers::NONE,
            vec![key(Key::V, false, Modifiers::NONE)],
        );
        assert_eq!(taken, Vec::new(), "its press arrived");
    }

    #[test]
    fn a_press_tracked_while_a_field_had_the_keyboard_still_counts() {
        let mut chords = ClipboardChords::default();
        let now = Instant::now();
        // A text field has the keyboard: the frame is tracked, not taken.
        let typed = input(Modifiers::NONE, vec![key(Key::V, true, Modifiers::NONE)]);
        chords.track(&typed, now);
        // The field loses it; the release reaches the list.
        let (taken, _) = frame(
            &mut chords,
            now,
            Modifiers::NONE,
            vec![key(Key::V, false, Modifiers::NONE)],
        );
        assert_eq!(taken, Vec::new());
    }

    #[test]
    fn shift_insert_pastes_and_ctrl_insert_does_not() {
        let mut chords = ClipboardChords::default();
        let now = Instant::now();
        // Windows: Shift+Insert is swallowed as a paste.
        let (taken, _) = frame(
            &mut chords,
            now,
            Modifiers::SHIFT,
            vec![key(Key::Insert, false, Modifiers::SHIFT)],
        );
        assert_eq!(taken, [paste(Modifiers::COMMAND)]);
        // Ctrl+Insert is swallowed as a copy: its release is no paste.
        let (taken, _) = frame(
            &mut chords,
            now,
            Modifiers::COMMAND,
            vec![Event::Copy, key(Key::Insert, false, Modifiers::COMMAND)],
        );
        assert_eq!(
            taken,
            [Chord {
                modifiers: Modifiers::COMMAND,
                key: Key::C
            }]
        );
        // Elsewhere Shift+Insert arrives as a key: press and release.
        frame(
            &mut chords,
            now,
            Modifiers::SHIFT,
            vec![key(Key::Insert, true, Modifiers::SHIFT)],
        );
        let (taken, _) = frame(
            &mut chords,
            now,
            Modifiers::SHIFT,
            vec![key(Key::Insert, false, Modifiers::SHIFT)],
        );
        assert_eq!(taken, Vec::new());
    }
}
