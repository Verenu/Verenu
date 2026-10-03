//! Physical key ownership and dictation gestures for the Windows hook.
const CHORD_TAP_MAX_HOLD_MS: u64 = 200;
const CHORD_DOUBLE_TAP_WINDOW_MS: u64 = 300;
const STALE_KEY_RECONCILIATION_GRACE_MS: u64 = 100;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ChordKey(pub(super) usize);
#[allow(non_upper_case_globals)]
#[cfg(test)]
impl ChordKey {
    pub(super) const Key1: Self = Self(0);
    pub(super) const Key2: Self = Self(1);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyEdge {
    Down,
    Up,
}

// The shared "Fire" prefix reads clearly as "fire this callback" at each call
// site; not worth losing that for clippy's glob-import naming heuristic.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ChordAction {
    FirePress,
    FireRelease,
    FireCancel,
    FireHandless,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum KeyDisposition {
    Suppress,
    Passthrough,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ChordOutcome {
    pub(super) action: Option<ChordAction>,
    pub(super) disposition: KeyDisposition,
}

impl ChordOutcome {
    fn suppress(action: Option<ChordAction>) -> Self {
        Self {
            action,
            disposition: KeyDisposition::Suppress,
        }
    }
    fn passthrough() -> Self {
        Self {
            action: None,
            disposition: KeyDisposition::Passthrough,
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum TapState {
    #[default]
    None,
    WaitingForFullRelease,
    AwaitingSecondTap {
        deadline_start_ms: u64,
    },
}

/// Chord/handsfree gesture tracking. Owned exclusively by the hook thread
/// (accessed only via the `CHORD_MACHINE` thread-local below) so it never
/// needs locking inside `hook_proc`, which must return quickly or Windows
/// silently unhooks it.
pub(super) struct ChordStateMachine {
    pub(super) keys: Vec<KeyState>,
    pub(super) chord_down: bool,
    chord_first_down_ms: u64,
    tap: TapState,
    space_down: bool,
    space_passed_through: bool,
    handless_from_chord: bool,
}

#[derive(Default, Clone)]
pub(super) struct KeyState {
    pub(super) down: bool,
    pub(super) down_since_ms: u64,
    pub(super) passed_through: bool,
    pub(super) was_chord: bool,
}
impl Default for ChordStateMachine {
    fn default() -> Self {
        Self::with_key_count(2)
    }
}
impl ChordStateMachine {
    pub(super) fn with_key_count(count: usize) -> Self {
        Self {
            keys: vec![KeyState::default(); count],
            chord_down: false,
            chord_first_down_ms: 0,
            tap: TapState::None,
            space_down: false,
            space_passed_through: false,
            handless_from_chord: false,
        }
    }
    fn key_down_mut(&mut self, key: ChordKey) -> &mut bool {
        &mut self.keys[key.0].down
    }
    fn key_passed_through_mut(&mut self, key: ChordKey) -> &mut bool {
        &mut self.keys[key.0].passed_through
    }
    pub(super) fn mark_key_passed_through(&mut self, key: ChordKey) {
        self.keys[key.0].passed_through = true;
    }
    fn key_down_since_mut(&mut self, key: ChordKey) -> &mut u64 {
        &mut self.keys[key.0].down_since_ms
    }
    /// Corrects stale ownership bookkeeping against live OS key state. A
    /// keyup can occasionally never reach this hook (e.g. swallowed by
    /// another low-level hook, or eaten by the OS's own Start-menu handling
    /// of a bare Win press) which leaves `key_down` stuck true forever —
    /// after that, the next lone press of the *other* key looks like a
    /// chord-forming edge and fires dictation off a single key. Called with
    /// the live `GetAsyncKeyState` read for the *other* key (never the one
    /// whose edge is currently being processed — its own down/up handling
    /// already reconciles itself).
    pub(super) fn reconcile_stale_key(&mut self, key: ChordKey, os_held: bool, now_ms: u64) {
        if os_held || !*self.key_down_mut(key) {
            return;
        }
        if now_ms.saturating_sub(*self.key_down_since_mut(key)) < STALE_KEY_RECONCILIATION_GRACE_MS
        {
            return;
        }
        *self.key_down_mut(key) = false;
        *self.key_down_since_mut(key) = 0;
        *self.key_passed_through_mut(key) = false;
        self.set_key_was_chord(key, false);
        if self.keys.iter().all(|key| !key.down) {
            self.chord_down = false;
            self.handless_from_chord = false;
        }
    }

    fn key_was_chord(&self, key: ChordKey) -> bool {
        self.keys[key.0].was_chord
    }
    fn set_key_was_chord(&mut self, key: ChordKey, value: bool) {
        self.keys[key.0].was_chord = value;
    }
    pub(super) fn clear_physical_keys(&mut self) {
        self.keys.fill(KeyState::default());
        self.chord_down = false;
    }
    /// Clears gesture/timing state only — never physical-key or ownership
    /// state. A full reset here would forget a currently-suppressed chord's
    /// keys are still Verenu-owned, letting a bare Ctrl-up/Win-up leak to the
    /// OS (Start menu) if a reset lands between a chord's two keyups.
    pub(super) fn reset_gesture_state(&mut self) {
        self.chord_down = false;
        self.chord_first_down_ms = 0;
        self.tap = TapState::None;
        self.space_down = false;
        self.space_passed_through = false;
        self.handless_from_chord = false;
    }

    pub(super) fn on_key_event(
        &mut self,
        key: ChordKey,
        edge: KeyEdge,
        now_ms: u64,
    ) -> ChordOutcome {
        match edge {
            KeyEdge::Down => self.on_key_down(key, now_ms),
            KeyEdge::Up => self.on_key_up(key, now_ms),
        }
    }

    fn on_key_down(&mut self, key: ChordKey, now_ms: u64) -> ChordOutcome {
        if self.keys.iter().all(|key| !key.down) {
            self.handless_from_chord = false;
        }

        let was_down = std::mem::replace(self.key_down_mut(key), true);
        if was_down {
            // Autorepeat. If this key is chord-owned, Verenu already claimed
            // it and must keep suppressing it (this is the actual fix for the
            // original bug: a handsfree trigger deliberately leaves
            // `chord_down` false while the keys may still be held, so without
            // this explicit already-down check a repeat could otherwise look
            // like a fresh edge and re-enter chord-formed handling). If it's
            // not chord-owned, it was never ours to begin with.
            return if self.key_was_chord(key) {
                ChordOutcome::suppress(None)
            } else {
                ChordOutcome::passthrough()
            };
        }
        *self.key_down_since_mut(key) = now_ms;

        if !self.keys.iter().all(|key| key.down) {
            self.mark_key_passed_through(key);
            // Only one key down so far — not our gesture yet, let it through
            // untouched (so a lone Ctrl or Win press still behaves normally).
            return ChordOutcome::passthrough();
        }

        // Chord-formed edge: both keys just became down together, regardless
        // of press order. This is the second key's down-edge; the first
        // already passed through above.
        for key in &mut self.keys {
            key.was_chord = true;
        }

        if self.chord_down {
            // Re-formation, not a new press — reclaim ownership and stop.
            //
            // A chord-forming edge normally implies a key went up and back
            // down, and that keyup would have cleared `chord_down`. So finding
            // it still set means something cleared a key's bookkeeping while it
            // was still physically held: `reconcile_stale_key` doing its job on
            // a keyup the hook never received, or `force_release_win_key`
            // wiping all of it before a paste. The key then autorepeats (it IS
            // still down), and that repeat arrives here looking like a fresh
            // chord.
            //
            // Restoring ownership above is right. Restarting the hold clock is
            // not: `held_ms` in on_key_up is measured from `chord_first_down_ms`,
            // so a chord held for seconds would be measured from this repeat,
            // come out under CHORD_TAP_MAX_HOLD_MS, and be classified as a tap
            // — firing FireCancel and throwing the dictation away. That is the
            // "every dictation gets cancelled" bug.
            return ChordOutcome::suppress(None);
        }

        self.chord_first_down_ms = now_ms;

        if let TapState::AwaitingSecondTap { deadline_start_ms } = self.tap {
            if now_ms.saturating_sub(deadline_start_ms) <= CHORD_DOUBLE_TAP_WINDOW_MS {
                self.tap = TapState::None;
                // Handsfree is a discrete toggle, not a held chord — leave
                // chord_down false so the user's fingers coming off both keys
                // afterward needs no further chord bookkeeping here.
                return ChordOutcome::suppress(Some(ChordAction::FireHandless));
            }
        }

        self.tap = TapState::None;
        self.chord_down = true;
        ChordOutcome::suppress(Some(ChordAction::FirePress))
    }

    pub(super) fn on_space_event(&mut self, edge: KeyEdge) -> ChordOutcome {
        match edge {
            KeyEdge::Down => {
                if self.space_down {
                    return if self.space_passed_through {
                        ChordOutcome::passthrough()
                    } else if self.handless_from_chord {
                        ChordOutcome::suppress(None)
                    } else {
                        ChordOutcome::passthrough()
                    };
                }

                if self.chord_down && !self.handless_from_chord {
                    self.space_down = true;
                    self.space_passed_through = false;
                    self.chord_down = false;
                    self.tap = TapState::None;
                    self.handless_from_chord = true;
                    ChordOutcome::suppress(Some(ChordAction::FireHandless))
                } else if self.handless_from_chord {
                    self.space_down = true;
                    self.space_passed_through = false;
                    ChordOutcome::suppress(None)
                } else {
                    self.space_down = true;
                    self.space_passed_through = true;
                    ChordOutcome::passthrough()
                }
            }
            KeyEdge::Up => {
                let was_down = std::mem::replace(&mut self.space_down, false);
                let passed_through = std::mem::replace(&mut self.space_passed_through, false);
                if was_down && !passed_through {
                    ChordOutcome::suppress(None)
                } else {
                    ChordOutcome::passthrough()
                }
            }
        }
    }

    fn on_key_up(&mut self, key: ChordKey, now_ms: u64) -> ChordOutcome {
        let _was_down = std::mem::replace(self.key_down_mut(key), false);
        *self.key_down_since_mut(key) = 0;
        let key_passed_through = std::mem::replace(self.key_passed_through_mut(key), false);

        if !self.key_was_chord(key) {
            // Never claimed as part of a chord — always pass through, or an
            // ordinary Ctrl/Windows release could get silently swallowed.
            return ChordOutcome::passthrough();
        }
        self.set_key_was_chord(key, false);

        let mut action = None;
        if self.chord_down {
            self.chord_down = false;
            let held_ms = now_ms.saturating_sub(self.chord_first_down_ms);
            if held_ms >= CHORD_TAP_MAX_HOLD_MS {
                action = Some(ChordAction::FireRelease);
            } else {
                action = Some(ChordAction::FireCancel);
                self.tap = TapState::WaitingForFullRelease;
            }
        }

        // The double-tap clock starts at the first release following a quick
        // chord tap. This deliberately does NOT require the *other* key to
        // also be up: holding one key down continuously and quick-tapping the
        // other twice (e.g. hold Ctrl, double-click Win) must also arm the
        // second tap, since a remapped mouse button can only ever send taps
        // of a single key, never hold one key while tapping another. When
        // both keys happen to release together (the classic "double-tap the
        // whole chord" gesture) this still starts the window at the first of
        // the two releases, which are normally only a few ms apart.
        if matches!(self.tap, TapState::WaitingForFullRelease) {
            self.tap = TapState::AwaitingSecondTap {
                deadline_start_ms: now_ms,
            };
        }

        if self.keys.iter().all(|key| !key.down) {
            self.handless_from_chord = false;
        }

        if key_passed_through {
            ChordOutcome {
                action,
                disposition: KeyDisposition::Passthrough,
            }
        } else {
            ChordOutcome::suppress(action)
        }
    }
}

#[cfg(test)]
mod chord_tests {
    use super::*;

    fn fresh() -> ChordStateMachine {
        ChordStateMachine::default()
    }

    #[test]
    fn press_order_independent() {
        for order in [
            [ChordKey::Key1, ChordKey::Key2],
            [ChordKey::Key2, ChordKey::Key1],
        ] {
            let mut m = fresh();
            let first = m.on_key_event(order[0], KeyEdge::Down, 0);
            assert_eq!(first.action, None);
            assert_eq!(first.disposition, KeyDisposition::Passthrough);
            let second = m.on_key_event(order[1], KeyEdge::Down, 10);
            assert_eq!(second.action, Some(ChordAction::FirePress));
            assert_eq!(second.disposition, KeyDisposition::Suppress);
            assert!(m.chord_down);
        }
    }

    #[test]
    fn long_hold_still_releases_after_ownership_is_reconciled_away() {
        // Regression: every dictation got cancelled instead of transcribed.
        //
        // `reconcile_stale_key` (and `force_release_win_key`, which does the
        // same thing wholesale before a paste) can clear a key's down/ownership
        // bookkeeping while the user is still physically holding the chord —
        // that is the entire point of it, for keyups the hook never received.
        // But the key keeps autorepeating, and the next repeat then looked like
        // a brand-new chord-forming edge, which reset `chord_first_down_ms` to
        // "now". The hold length is measured from that field, so a chord held
        // for seconds measured as a sub-200ms tap and fired FireCancel.
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        assert_eq!(m.chord_first_down_ms, 5);

        // Something reconciles key2's ownership away mid-hold.
        m.reconcile_stale_key(ChordKey::Key2, false, 3_000);
        // ...and key2 autorepeats, since it is still physically down.
        let repeat = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 3000);
        assert_eq!(
            repeat.action, None,
            "a repeat mid-hold must not re-fire a press"
        );
        assert_eq!(
            m.chord_first_down_ms, 5,
            "re-forming the chord mid-hold must not restart the hold clock"
        );

        // The real release, seconds after the real press, is a hold.
        let up = m.on_key_event(ChordKey::Key1, KeyEdge::Up, 4000);
        assert_eq!(up.action, Some(ChordAction::FireRelease));
    }

    #[test]
    fn autorepeat_while_chord_owned_is_suppressed_with_no_action() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 10);
        let repeat = m.on_key_event(ChordKey::Key1, KeyEdge::Down, 40);
        assert_eq!(repeat.action, None);
        assert_eq!(repeat.disposition, KeyDisposition::Suppress);
    }

    #[test]
    fn autorepeat_after_handless_trigger_does_not_refire() {
        let mut m = fresh();
        // First tap.
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        m.on_key_event(ChordKey::Key1, KeyEdge::Up, 50);
        m.on_key_event(ChordKey::Key2, KeyEdge::Up, 55);
        // Second tap within window -> handsfree.
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 100);
        let second = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 105);
        assert_eq!(second.action, Some(ChordAction::FireHandless));
        assert!(!m.chord_down);
        // Both physical keys still held -> autorepeat must not refire anything.
        let repeat1 = m.on_key_event(ChordKey::Key1, KeyEdge::Down, 130);
        let repeat2 = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 160);
        assert_eq!(repeat1.action, None);
        assert_eq!(repeat1.disposition, KeyDisposition::Suppress);
        assert_eq!(repeat2.action, None);
        assert_eq!(repeat2.disposition, KeyDisposition::Suppress);
    }

    #[test]
    fn duplicate_keyup_not_owned_passes_through() {
        let mut m = fresh();
        let outcome = m.on_key_event(ChordKey::Key1, KeyEdge::Up, 0);
        assert_eq!(outcome.action, None);
        assert_eq!(outcome.disposition, KeyDisposition::Passthrough);
    }

    #[test]
    fn standalone_key2_keyup_passes_through() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 0);
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Up, 10);
        assert_eq!(outcome.action, None);
        assert_eq!(outcome.disposition, KeyDisposition::Passthrough);
    }

    #[test]
    fn owned_keyup_is_suppressed() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        let up = m.on_key_event(ChordKey::Key2, KeyEdge::Up, 500);
        assert_eq!(up.disposition, KeyDisposition::Suppress);
    }

    #[test]
    fn first_key2_keyup_matches_its_passthrough_down() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 5);
        let up = m.on_key_event(ChordKey::Key2, KeyEdge::Up, 500);
        assert_eq!(up.action, Some(ChordAction::FireRelease));
        assert_eq!(up.disposition, KeyDisposition::Passthrough);
    }

    #[test]
    fn held_key_double_tap_of_other_key_triggers_handless() {
        // Hold key1 continuously (e.g. Ctrl), quick-tap key2 (e.g. Win) twice
        // without ever releasing key1 — the gesture a mouse button remapped
        // to double-click a single key needs, since it can never hold one
        // key while tapping another.
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        let cancel = m.on_key_event(ChordKey::Key2, KeyEdge::Up, 50);
        assert_eq!(cancel.action, Some(ChordAction::FireCancel));
        // key1 never released; key2 taps down again within the window.
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 100);
        assert_eq!(outcome.action, Some(ChordAction::FireHandless));
        assert!(!m.chord_down);
        assert!(m.keys[0].down); // key1 still physically held throughout
    }

    #[test]
    fn full_release_then_second_tap_within_window_triggers_handless_once() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        m.on_key_event(ChordKey::Key1, KeyEdge::Up, 50);
        m.on_key_event(ChordKey::Key2, KeyEdge::Up, 55); // full release at 55
        let second = m.on_key_event(ChordKey::Key1, KeyEdge::Down, 300);
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 340); // 340-50=290 <= 300
        assert_eq!(second.action, None);
        assert_eq!(outcome.action, Some(ChordAction::FireHandless));
    }

    #[test]
    fn window_measured_from_first_release() {
        // The double-tap clock starts at the FIRST release of the pair (not
        // the last), so the solo-hold gesture above has a well-defined start
        // point even though the held key may never release at all.
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        m.on_key_event(ChordKey::Key1, KeyEdge::Up, 50); // first release -> deadline starts here
        m.on_key_event(ChordKey::Key2, KeyEdge::Up, 130); // second released 80ms later
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 340);
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 345); // 345-50=295 <= 300
        assert_eq!(outcome.action, Some(ChordAction::FireHandless));
    }

    #[test]
    fn second_tap_outside_window_starts_fresh_press() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        m.on_key_event(ChordKey::Key1, KeyEdge::Up, 50);
        m.on_key_event(ChordKey::Key2, KeyEdge::Up, 55);
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 1000);
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 1010);
        assert_eq!(outcome.action, Some(ChordAction::FirePress));
        assert!(m.chord_down);
    }

    #[test]
    fn solo_key_tap_never_fires_anything() {
        let mut m = fresh();
        let down = m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        let up = m.on_key_event(ChordKey::Key1, KeyEdge::Up, 50);
        assert_eq!(down.action, None);
        assert_eq!(down.disposition, KeyDisposition::Passthrough);
        assert_eq!(up.action, None);
        assert_eq!(up.disposition, KeyDisposition::Passthrough);
    }

    #[test]
    fn space_while_chord_is_held_converts_to_handless_and_stays_suppressed() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);

        let trigger = m.on_space_event(KeyEdge::Down);
        assert_eq!(trigger.action, Some(ChordAction::FireHandless));
        assert_eq!(trigger.disposition, KeyDisposition::Suppress);
        assert!(!m.chord_down);

        let repeat = m.on_space_event(KeyEdge::Down);
        assert_eq!(repeat.action, None);
        assert_eq!(repeat.disposition, KeyDisposition::Suppress);

        let space_up = m.on_space_event(KeyEdge::Up);
        assert_eq!(space_up.action, None);
        assert_eq!(space_up.disposition, KeyDisposition::Suppress);

        // Releasing the original hold keys must not fire the normal release
        // action after Space has converted the session.
        let key1_up = m.on_key_event(ChordKey::Key1, KeyEdge::Up, 100);
        let key2_up = m.on_key_event(ChordKey::Key2, KeyEdge::Up, 105);
        assert_eq!(key1_up.action, None);
        assert_eq!(key2_up.action, None);
    }

    #[test]
    fn space_outside_active_chord_passes_through() {
        let mut m = fresh();
        let down = m.on_space_event(KeyEdge::Down);
        let up = m.on_space_event(KeyEdge::Up);
        assert_eq!(down.disposition, KeyDisposition::Passthrough);
        assert_eq!(up.disposition, KeyDisposition::Passthrough);
    }

    #[test]
    fn reset_gesture_state_clears_space_conversion_state() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        let trigger = m.on_space_event(KeyEdge::Down);
        assert_eq!(trigger.action, Some(ChordAction::FireHandless));

        m.reset_gesture_state();

        let down = m.on_space_event(KeyEdge::Down);
        let up = m.on_space_event(KeyEdge::Up);
        assert_eq!(down.disposition, KeyDisposition::Passthrough);
        assert_eq!(up.disposition, KeyDisposition::Passthrough);
    }

    #[test]
    fn stale_key_bookkeeping_does_not_fire_on_lone_press() {
        let mut m = fresh();
        // Simulate a missed keyup: key1 (e.g. Ctrl) is marked down internally
        // but the OS no longer reports it held.
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.reconcile_stale_key(ChordKey::Key1, false, 1_000);
        assert!(!m.keys[0].down);
        // A lone press of key2 must not look like a chord-forming edge.
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 100);
        assert_eq!(outcome.action, None);
        assert_eq!(outcome.disposition, KeyDisposition::Passthrough);
    }

    #[test]
    fn reconcile_leaves_genuinely_held_key_untouched() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.reconcile_stale_key(ChordKey::Key1, true, 10);
        assert!(m.keys[0].down);
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 10);
        assert_eq!(outcome.action, Some(ChordAction::FirePress));
    }

    #[test]
    fn near_simultaneous_modifiers_form_a_chord_before_async_state_catches_up() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 10);

        // A mouse remapping button can deliver Ctrl's hook event before the
        // async-state query made while processing Win sees that first edge.
        m.reconcile_stale_key(ChordKey::Key1, false, 11);
        let outcome = m.on_key_event(ChordKey::Key2, KeyEdge::Down, 11);
        assert_eq!(outcome.action, Some(ChordAction::FirePress));
        assert!(m.chord_down);
    }

    #[test]
    fn reset_gesture_state_preserves_key_ownership() {
        let mut m = fresh();
        m.on_key_event(ChordKey::Key1, KeyEdge::Down, 0);
        m.on_key_event(ChordKey::Key2, KeyEdge::Down, 5);
        assert!(m.chord_down);
        m.reset_gesture_state();
        assert!(!m.chord_down);
        assert_eq!(m.tap, TapState::None);
        // Physical/ownership state must survive the reset.
        assert!(m.keys[0].down);
        assert!(m.keys[1].down);
        assert!(m.keys[0].was_chord);
        assert!(m.keys[1].was_chord);
        // The first key's down-edge was passed through before the chord formed,
        // so its matching keyup must pass through too. The second key remains
        // fully owned by Verenu.
        let up1 = m.on_key_event(ChordKey::Key1, KeyEdge::Up, 10);
        let up2 = m.on_key_event(ChordKey::Key2, KeyEdge::Up, 15);
        assert_eq!(up1.disposition, KeyDisposition::Passthrough);
        assert_eq!(up2.disposition, KeyDisposition::Suppress);
    }

    #[test]
    fn all_keys_are_required_and_any_release_ends_a_large_chord() {
        for first_release in 0..5 {
            let mut m = ChordStateMachine::with_key_count(5);
            for index in [4, 1, 3, 0] {
                assert_eq!(
                    m.on_key_event(ChordKey(index), KeyEdge::Down, 0).action,
                    None
                );
            }
            assert_eq!(
                m.on_key_event(ChordKey(2), KeyEdge::Down, 20).action,
                Some(ChordAction::FirePress)
            );
            assert_eq!(m.on_key_event(ChordKey(2), KeyEdge::Down, 30).action, None);
            assert_eq!(
                m.on_key_event(ChordKey(first_release), KeyEdge::Up, 500)
                    .action,
                Some(ChordAction::FireRelease)
            );
            for index in 0..5 {
                assert_eq!(
                    m.on_key_event(ChordKey(index), KeyEdge::Up, 510).action,
                    None
                );
            }
        }
    }
    #[test]
    fn single_key_and_large_double_taps_preserve_gestures() {
        for count in [1, 3, 6] {
            let mut m = ChordStateMachine::with_key_count(count);
            for index in 0..count {
                m.on_key_event(ChordKey(index), KeyEdge::Down, 0);
            }
            assert_eq!(
                m.on_key_event(ChordKey(count - 1), KeyEdge::Up, 50).action,
                Some(ChordAction::FireCancel)
            );
            assert_eq!(
                m.on_key_event(ChordKey(count - 1), KeyEdge::Down, 100)
                    .action,
                Some(ChordAction::FireHandless)
            );
            assert_eq!(
                m.on_key_event(ChordKey(count - 1), KeyEdge::Down, 110)
                    .action,
                None
            );
            assert_eq!(
                m.on_key_event(ChordKey(count - 1), KeyEdge::Up, 150).action,
                None
            );
        }
    }
}
