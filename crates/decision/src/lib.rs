//! The watcher's brain, kept free of Windows and serial code so it can be
//! tested exhaustively.
//!
//! Inputs each tick: does Windows report the Bluetooth link to the headphones
//! as up, and is the headphones' audio render endpoint present. Output: whether
//! to send the headphones a CONNECT for this machine right now.
//!
//! Rules:
//! - Never act while the link is down. Opening the serial port would page the
//!   headphones and yank them onto the desktop whenever they are in range.
//! - Wait `debounce` after first seeing link-up-without-audio; profiles
//!   normally arrive a few seconds after the link.
//! - Send CONNECT at most once per `cooldown`, so a headset that refuses is not
//!   spammed.
//! - Give up after `max_attempts` consecutive failures until the link goes down
//!   and comes back (a fresh session resets the budget).

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub link_up: bool,
    pub endpoint_present: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do.
    Idle,
    /// Send `DeviceManagement.Connect(local mac)` now.
    Connect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub debounce: Duration,
    pub cooldown: Duration,
    pub max_attempts: u32,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            debounce: Duration::from_secs(8),
            cooldown: Duration::from_secs(30),
            max_attempts: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Link down, or link up with audio: nothing wrong.
    Healthy,
    /// Link up, no audio, waiting out the debounce.
    Suspect { since: Instant },
    /// A CONNECT was sent; waiting out the cooldown to see whether audio appears.
    Nudged { at: Instant, attempts: u32 },
    /// Budget exhausted for this link session.
    GaveUp { attempts: u32 },
}

#[derive(Debug)]
pub struct Watcher {
    policy: Policy,
    phase: Phase,
    link_was_up: bool,
}

impl Watcher {
    pub fn new(policy: Policy) -> Watcher {
        Watcher {
            policy,
            phase: Phase::Healthy,
            link_was_up: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// One tick. `now` is injected so tests do not sleep.
    pub fn observe(&mut self, obs: Observation, now: Instant) -> Action {
        // A link that went down resets everything: the next connection is a new session.
        if !obs.link_up {
            self.link_was_up = false;
            self.phase = Phase::Healthy;
            return Action::Idle;
        }
        self.link_was_up = true;

        if obs.endpoint_present {
            self.phase = Phase::Healthy;
            return Action::Idle;
        }

        // Link up, no audio.
        match self.phase {
            Phase::Healthy => {
                self.phase = Phase::Suspect { since: now };
                Action::Idle
            }
            Phase::Suspect { since } => {
                if now.duration_since(since) >= self.policy.debounce {
                    self.phase = Phase::Nudged { at: now, attempts: 1 };
                    Action::Connect
                } else {
                    Action::Idle
                }
            }
            Phase::Nudged { at, attempts } => {
                if now.duration_since(at) < self.policy.cooldown {
                    Action::Idle
                } else if attempts >= self.policy.max_attempts {
                    self.phase = Phase::GaveUp { attempts };
                    Action::Idle
                } else {
                    self.phase = Phase::Nudged {
                        at: now,
                        attempts: attempts + 1,
                    };
                    Action::Connect
                }
            }
            Phase::GaveUp { .. } => Action::Idle,
        }
    }

    /// A manual "connect now" from the tray or CLI. Resets the budget so the
    /// automatic path may try again afterwards.
    pub fn manual_nudge(&mut self, now: Instant) {
        self.phase = Phase::Nudged { at: now, attempts: 0 };
    }

    /// One-line human summary for the tray tooltip and the log.
    pub fn describe(&self, now: Instant) -> String {
        match self.phase {
            Phase::Healthy => {
                if self.link_was_up {
                    "link up, audio present".to_string()
                } else {
                    "headphones not connected".to_string()
                }
            }
            Phase::Suspect { since } => {
                format!("link up but no audio for {}s", now.duration_since(since).as_secs())
            }
            Phase::Nudged { at, attempts } => {
                format!(
                    "sent connect {}s ago (attempt {})",
                    now.duration_since(at).as_secs(),
                    attempts
                )
            }
            Phase::GaveUp { attempts } => format!("gave up after {attempts} attempts; reconnect the headphones"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Policy {
        Policy {
            debounce: Duration::from_secs(8),
            cooldown: Duration::from_secs(30),
            max_attempts: 3,
        }
    }

    const UP_NO_AUDIO: Observation = Observation {
        link_up: true,
        endpoint_present: false,
    };
    const UP_AUDIO: Observation = Observation {
        link_up: true,
        endpoint_present: true,
    };
    const DOWN: Observation = Observation {
        link_up: false,
        endpoint_present: false,
    };

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + Duration::from_secs(secs)
    }

    #[test]
    fn never_acts_while_link_is_down() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        for s in 0..100 {
            assert_eq!(w.observe(DOWN, at(t0, s)), Action::Idle);
        }
        assert_eq!(w.phase(), Phase::Healthy);
        assert_eq!(w.describe(t0), "headphones not connected");
    }

    #[test]
    fn healthy_when_audio_present() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        assert_eq!(w.observe(UP_AUDIO, t0), Action::Idle);
        assert_eq!(w.describe(t0), "link up, audio present");
    }

    #[test]
    fn nudges_after_debounce_only() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        assert_eq!(w.observe(UP_NO_AUDIO, t0), Action::Idle);
        assert!(matches!(w.phase(), Phase::Suspect { .. }));
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 3)), Action::Idle);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 7)), Action::Idle);
        assert_eq!(w.describe(at(t0, 7)), "link up but no audio for 7s");
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 8)), Action::Connect);
        assert!(matches!(w.phase(), Phase::Nudged { attempts: 1, .. }));
    }

    #[test]
    fn audio_arriving_during_debounce_cancels() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        w.observe(UP_NO_AUDIO, t0);
        assert_eq!(w.observe(UP_AUDIO, at(t0, 4)), Action::Idle);
        assert_eq!(w.phase(), Phase::Healthy);
        // A later dropout starts a fresh debounce window.
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 10)), Action::Idle);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 17)), Action::Idle);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 18)), Action::Connect);
    }

    #[test]
    fn respects_cooldown_and_gives_up() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        w.observe(UP_NO_AUDIO, t0);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 8)), Action::Connect);
        // Inside the cooldown: quiet.
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 20)), Action::Idle);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 37)), Action::Idle);
        assert_eq!(w.describe(at(t0, 20)), "sent connect 12s ago (attempt 1)");
        // Second and third attempts.
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 38)), Action::Connect);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 68)), Action::Connect);
        assert!(matches!(w.phase(), Phase::Nudged { attempts: 3, .. }));
        // Budget spent: the next cooldown expiry gives up instead of nudging.
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 98)), Action::Idle);
        assert_eq!(w.phase(), Phase::GaveUp { attempts: 3 });
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 500)), Action::Idle);
        assert!(w.describe(at(t0, 500)).starts_with("gave up after 3 attempts"));
    }

    #[test]
    fn success_after_nudge_returns_to_healthy() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        w.observe(UP_NO_AUDIO, t0);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 8)), Action::Connect);
        assert_eq!(w.observe(UP_AUDIO, at(t0, 12)), Action::Idle);
        assert_eq!(w.phase(), Phase::Healthy);
    }

    #[test]
    fn link_drop_resets_budget() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        w.observe(UP_NO_AUDIO, t0);
        w.observe(UP_NO_AUDIO, at(t0, 8));
        w.observe(UP_NO_AUDIO, at(t0, 38));
        w.observe(UP_NO_AUDIO, at(t0, 68));
        w.observe(UP_NO_AUDIO, at(t0, 98));
        assert!(matches!(w.phase(), Phase::GaveUp { .. }));
        assert_eq!(w.observe(DOWN, at(t0, 100)), Action::Idle);
        assert_eq!(w.phase(), Phase::Healthy);
        w.observe(UP_NO_AUDIO, at(t0, 200));
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 208)), Action::Connect);
    }

    #[test]
    fn manual_nudge_resets_attempts_and_starts_cooldown() {
        let mut w = Watcher::new(p());
        let t0 = Instant::now();
        w.observe(UP_NO_AUDIO, t0);
        w.observe(UP_NO_AUDIO, at(t0, 8));
        w.observe(UP_NO_AUDIO, at(t0, 38));
        w.manual_nudge(at(t0, 40));
        assert!(matches!(w.phase(), Phase::Nudged { attempts: 0, .. }));
        // Cooldown from the manual nudge, then automatic attempt 1 again.
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 50)), Action::Idle);
        assert_eq!(w.observe(UP_NO_AUDIO, at(t0, 70)), Action::Connect);
        assert!(matches!(w.phase(), Phase::Nudged { attempts: 1, .. }));
    }

    #[test]
    fn default_policy_is_sane() {
        let d = Policy::default();
        assert_eq!(d.debounce, Duration::from_secs(8));
        assert_eq!(d.cooldown, Duration::from_secs(30));
        assert_eq!(d.max_attempts, 4);
        let w = Watcher::new(d);
        assert_eq!(w.phase(), Phase::Healthy);
        assert!(format!("{w:?}").contains("Healthy"));
    }
}
