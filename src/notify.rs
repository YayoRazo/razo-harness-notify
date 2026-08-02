use crate::config::Config;
use crate::events::CanonicalEvent;
use chrono::NaiveTime;
#[cfg(test)]
use std::cell::RefCell;

pub trait Notifier {
    fn fire(&self, title: &str, message: &str, sound: bool) -> Result<(), String>;
}

pub struct RealNotifier;

impl Notifier for RealNotifier {
    fn fire(&self, title: &str, message: &str, sound: bool) -> Result<(), String> {
        let mut n = notify_rust::Notification::new();
        n.appname("Razo Notifier").summary(title).body(message);
        if sound {
            // Windows builds a silent toast unless a sound name is set
            // explicitly; "Default" selects the system default notification
            // sound. Linux daemons already apply their own default. macOS
            // stays soundless either way: an audible notification there
            // needs a verified sound name plus a real bundle identifier,
            // the same constraint that keeps os_check unimplemented on it.
            #[cfg(target_os = "windows")]
            n.sound_name("Default");
        } else {
            // Windows is already silent without a sound name; freedesktop
            // daemons honor the standard suppress-sound hint.
            #[cfg(all(unix, not(target_os = "macos")))]
            n.hint(notify_rust::Hint::SuppressSound(true));
        }
        n.show().map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Test-only in-memory notifier that records every fire() call instead of
/// showing an OS notification. Gated to test builds so it is not dead code in
/// a released binary.
#[cfg(test)]
#[derive(Default)]
pub struct FakeNotifier {
    pub calls: RefCell<Vec<(String, String, bool)>>,
}

#[cfg(test)]
impl Notifier for FakeNotifier {
    fn fire(&self, title: &str, message: &str, sound: bool) -> Result<(), String> {
        self.calls.borrow_mut().push((title.to_string(), message.to_string(), sound));
        Ok(())
    }
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

fn in_quiet_hours(cfg: &Config, now: NaiveTime) -> bool {
    let start = match parse_time(&cfg.dnd.start) {
        Some(t) => t,
        None => return false,
    };
    let end = match parse_time(&cfg.dnd.end) {
        Some(t) => t,
        None => return false,
    };
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

pub fn should_fire(cfg: &Config, event: CanonicalEvent, now: NaiveTime) -> bool {
    let event_enabled = match event {
        CanonicalEvent::Done => cfg.events.done,
        CanonicalEvent::Attention => cfg.events.attention,
        CanonicalEvent::SubagentDone => cfg.events.subagent_done,
    };
    if !event_enabled {
        return false;
    }
    if cfg.dnd.enabled && in_quiet_hours(cfg, now) {
        return false;
    }
    true
}

fn default_title(event: CanonicalEvent) -> &'static str {
    match event {
        CanonicalEvent::Done => "Task complete",
        CanonicalEvent::Attention => "Needs your input",
        CanonicalEvent::SubagentDone => "Subagent finished",
    }
}

/// Which window sent the notification. Prefers the real project directory
/// (from the hook's payload or --cwd) over the harness id, so multiple
/// windows of the SAME harness are actually distinguishable - showing
/// "claude-code" on every notification tells you nothing when three
/// Claude Code windows are open at once.
fn session_label(cfg: &Config, harness: &str, cwd: Option<&str>) -> Option<String> {
    if !cfg.session.include_name {
        return None;
    }
    match cwd {
        Some(dir) if cfg.session.format == "path" => Some(dir.to_string()),
        Some(dir) => {
            let name = std::path::Path::new(dir)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(dir);
            Some(name.to_string())
        }
        None => {
            let harness_label = if harness.is_empty() { "unknown" } else { harness };
            Some(harness_label.to_string())
        }
    }
}

/// What to say and where it came from - grouped together because they are
/// always supplied (or omitted) as a set from a single hook invocation,
/// keeping `handle_notify`'s own argument count small.
#[derive(Default)]
pub struct NotifyContext<'a> {
    pub title: Option<&'a str>,
    pub message: Option<&'a str>,
    pub cwd: Option<&'a str>,
}

fn sanitize(s: &str) -> &str {
    // Strip ASCII control characters (below 0x20, except tab and newline
    // which are harmless in plain-text notification bodies) and reject any
    // string containing angle brackets - desktop notification bodies are
    // plain text, and some Linux daemons (dunst with markup=yes) render a
    // subset of HTML, so brackets could be interpreted as markup.
    if s.chars().any(|c| c == '<' || c == '>' || (c as u32) < 0x20 && c != '\t' && c != '\n') {
        ""
    } else {
        s
    }
}

/// Returns whether a notification was actually shown: `Ok(true)` fired,
/// `Ok(false)` suppressed by config (event disabled or quiet hours), `Err`
/// when the OS notifier call itself failed. Hook-driven callers ignore the
/// outcome (they must never block the harness); `test` reports it.
pub fn handle_notify(
    cfg: &Config,
    harness: &str,
    event: CanonicalEvent,
    ctx: NotifyContext,
    notifier: &dyn Notifier,
    now: NaiveTime,
) -> Result<bool, String> {
    if !should_fire(cfg, event, now) {
        return Ok(false);
    }
    let title = sanitize(ctx.title.unwrap_or_else(|| default_title(event)));
    let label = session_label(cfg, harness, ctx.cwd);
    let message = match (ctx.message, label) {
        (Some(m), Some(l)) => format!("{} ({})", sanitize(m), l),
        (Some(m), None) => sanitize(m).to_string(),
        (None, Some(l)) => l,
        (None, None) => String::new(),
    };
    notifier.fire(title, &message, cfg.sound.enabled).map(|()| true)
}

/// Maps Claude Code's/Kimi's Notification `notification_type` payload field
/// to the canonical event that should actually fire, refining the static
/// event the hook was installed with. `permission_prompt` is a genuine
/// blocking request; `agent_needs_input`/`agent_completed` are subagent
/// lifecycle notifications routed through the same hook, which should
/// respect `events.subagent_done` (off by default) instead of always
/// reading as "needs your input". An unrecognized or absent type keeps
/// whatever event the hook was statically installed with.
pub fn refine_event_from_notification_type(notification_type: &str, fallback: CanonicalEvent) -> CanonicalEvent {
    match notification_type {
        "agent_needs_input" | "agent_completed" => CanonicalEvent::SubagentDone,
        "permission_prompt"
        | "idle_prompt"
        | "auth_success"
        | "elicitation_dialog"
        | "elicitation_complete"
        | "elicitation_response" => CanonicalEvent::Attention,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use chrono::NaiveTime;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn fires_when_event_enabled_and_no_quiet_hours() {
        let cfg = Config::default();
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(14, 0)));
        assert!(should_fire(&cfg, CanonicalEvent::Attention, t(14, 0)));
    }

    #[test]
    fn does_not_fire_when_attention_disabled() {
        let mut cfg = Config::default();
        cfg.events.attention = false;
        assert!(!should_fire(&cfg, CanonicalEvent::Attention, t(14, 0)));
    }

    #[test]
    fn does_not_fire_when_event_disabled() {
        let mut cfg = Config::default();
        cfg.events.subagent_done = false;
        assert!(!should_fire(&cfg, CanonicalEvent::SubagentDone, t(14, 0)));
    }

    #[test]
    fn respects_quiet_hours_crossing_midnight() {
        let mut cfg = Config::default();
        cfg.dnd.enabled = true;
        cfg.dnd.start = "22:00".to_string();
        cfg.dnd.end = "08:00".to_string();
        assert!(!should_fire(&cfg, CanonicalEvent::Done, t(23, 0)));
        assert!(!should_fire(&cfg, CanonicalEvent::Done, t(3, 0)));
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(12, 0)));
    }

    #[test]
    fn quiet_hours_start_is_inclusive_and_end_is_exclusive() {
        let mut cfg = Config::default();
        cfg.dnd.enabled = true;
        cfg.dnd.start = "22:00".to_string();
        cfg.dnd.end = "08:00".to_string();
        assert!(!should_fire(&cfg, CanonicalEvent::Done, t(22, 0)), "start boundary suppresses");
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(8, 0)), "end boundary fires again");
    }

    #[test]
    fn equal_start_and_end_is_an_empty_window_that_never_suppresses() {
        let mut cfg = Config::default();
        cfg.dnd.enabled = true;
        cfg.dnd.start = "10:00".to_string();
        cfg.dnd.end = "10:00".to_string();
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(10, 0)));
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(23, 0)));
    }

    #[test]
    fn unparsable_time_is_conservative_and_does_not_suppress() {
        // Reachable through a hand-edited config.toml; config set rejects it.
        // An unparseable boundary means the window is undefined, so we do not
        // suppress rather than guessing midnight (which could produce the wrong
        // eight-hour DND block the operator never intended).
        let mut cfg = Config::default();
        cfg.dnd.enabled = true;
        cfg.dnd.start = "10pm".to_string();
        cfg.dnd.end = "08:00".to_string();
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(3, 0)), "unparseable start → don't suppress");
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(12, 0)));

        cfg.dnd.start = "22:00".to_string();
        cfg.dnd.end = "not-valid".to_string();
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(3, 0)), "unparseable end → don't suppress");

        cfg.dnd.start = "broken".to_string();
        cfg.dnd.end = "also-broken".to_string();
        assert!(should_fire(&cfg, CanonicalEvent::Done, t(3, 0)), "both unparseable → don't suppress");
    }

    #[test]
    fn handle_notify_calls_the_notifier_when_it_should_fire() {
        let cfg = Config::default();
        let notifier = FakeNotifier::default();
        let result = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, NotifyContext::default(), &notifier, t(12, 0));
        assert_eq!(result, Ok(true));
        assert_eq!(notifier.calls.borrow().len(), 1);
    }

    #[test]
    fn handle_notify_skips_the_notifier_when_muted_by_quiet_hours() {
        let mut cfg = Config::default();
        cfg.dnd.enabled = true;
        cfg.dnd.start = "00:00".to_string();
        cfg.dnd.end = "23:59".to_string();
        let notifier = FakeNotifier::default();
        let result = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, NotifyContext::default(), &notifier, t(12, 0));
        assert_eq!(result, Ok(false));
        assert_eq!(notifier.calls.borrow().len(), 0);
    }

    #[test]
    fn handle_notify_surfaces_a_notifier_failure() {
        struct FailingNotifier;
        impl Notifier for FailingNotifier {
            fn fire(&self, _: &str, _: &str, _: bool) -> Result<(), String> {
                Err("no daemon".to_string())
            }
        }
        let cfg = Config::default();
        let result = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, NotifyContext::default(), &FailingNotifier, t(12, 0));
        assert_eq!(result, Err("no daemon".to_string()));
    }

    #[test]
    fn sound_enabled_by_default_reaches_the_notifier() {
        let cfg = Config::default();
        let notifier = FakeNotifier::default();
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, NotifyContext::default(), &notifier, t(12, 0));
        assert!(notifier.calls.borrow()[0].2);
    }

    #[test]
    fn sound_disabled_reaches_the_notifier_as_a_silent_fire() {
        let mut cfg = Config::default();
        cfg.sound.enabled = false;
        let notifier = FakeNotifier::default();
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, NotifyContext::default(), &notifier, t(12, 0));
        assert!(!notifier.calls.borrow()[0].2);
    }

    #[test]
    fn session_label_uses_cwd_basename_by_default_when_enabled() {
        let mut cfg = Config::default();
        cfg.session.include_name = true;
        let notifier = FakeNotifier::default();
        let ctx = NotifyContext { cwd: Some("/home/op/harness-notify"), ..Default::default() };
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, ctx, &notifier, t(12, 0));
        let calls = notifier.calls.borrow();
        assert_eq!(calls[0].1, "harness-notify");
    }

    #[test]
    fn session_label_uses_full_path_when_format_is_path() {
        let mut cfg = Config::default();
        cfg.session.include_name = true;
        cfg.session.format = "path".to_string();
        let notifier = FakeNotifier::default();
        let ctx = NotifyContext { cwd: Some("/home/op/harness-notify"), ..Default::default() };
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, ctx, &notifier, t(12, 0));
        let calls = notifier.calls.borrow();
        assert_eq!(calls[0].1, "/home/op/harness-notify");
    }

    #[test]
    fn session_label_falls_back_to_unknown_when_harness_is_empty() {
        let mut cfg = Config::default();
        cfg.session.include_name = true;
        let notifier = FakeNotifier::default();
        let _ = handle_notify(&cfg, "", CanonicalEvent::Done, NotifyContext::default(), &notifier, t(12, 0));
        assert_eq!(notifier.calls.borrow()[0].1, "unknown");
    }

    #[test]
    fn handle_notify_uses_custom_title_when_provided() {
        let cfg = Config::default();
        let notifier = FakeNotifier::default();
        let ctx = NotifyContext { title: Some("Custom"), ..Default::default() };
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, ctx, &notifier, t(12, 0));
        assert_eq!(notifier.calls.borrow()[0].0, "Custom");
    }

    #[test]
    fn handle_notify_with_no_message_no_label_sends_empty_body() {
        let cfg = Config::default();
        let notifier = FakeNotifier::default();
        let ctx = NotifyContext::default();
        let _ = handle_notify(&cfg, "", CanonicalEvent::Done, ctx, &notifier, t(12, 0));
        assert_eq!(notifier.calls.borrow()[0].1, "");
    }

    #[test]
    fn handle_notify_with_label_only_sends_just_the_label() {
        let mut cfg = Config::default();
        cfg.session.include_name = true;
        let notifier = FakeNotifier::default();
        let ctx = NotifyContext { cwd: Some("/proj"), ..Default::default() };
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, ctx, &notifier, t(12, 0));
        assert_eq!(notifier.calls.borrow()[0].1, "proj");
    }

    #[test]
    fn session_label_falls_back_to_harness_id_when_no_cwd_available() {
        let mut cfg = Config::default();
        cfg.session.include_name = true;
        let notifier = FakeNotifier::default();
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, NotifyContext::default(), &notifier, t(12, 0));
        let calls = notifier.calls.borrow();
        assert_eq!(calls[0].1, "claude-code");
    }

    #[test]
    fn no_session_label_when_include_name_is_off() {
        let cfg = Config::default();
        assert!(!cfg.session.include_name);
        let notifier = FakeNotifier::default();
        let ctx = NotifyContext { message: Some("done"), cwd: Some("/home/op/harness-notify"), ..Default::default() };
        let _ = handle_notify(&cfg, "claude-code", CanonicalEvent::Done, ctx, &notifier, t(12, 0));
        let calls = notifier.calls.borrow();
        assert_eq!(calls[0].1, "done");
    }

    #[test]
    fn refines_permission_prompt_to_attention() {
        assert_eq!(
            refine_event_from_notification_type("permission_prompt", CanonicalEvent::Done),
            CanonicalEvent::Attention
        );
    }

    #[test]
    fn refines_agent_completed_and_agent_needs_input_to_subagent_done() {
        assert_eq!(
            refine_event_from_notification_type("agent_completed", CanonicalEvent::Attention),
            CanonicalEvent::SubagentDone
        );
        assert_eq!(
            refine_event_from_notification_type("agent_needs_input", CanonicalEvent::Attention),
            CanonicalEvent::SubagentDone
        );
    }

    #[test]
    fn refines_all_known_attention_types() {
        for nt in ["idle_prompt", "auth_success", "elicitation_dialog", "elicitation_complete", "elicitation_response"] {
            assert_eq!(
                refine_event_from_notification_type(nt, CanonicalEvent::Done),
                CanonicalEvent::Attention,
                "notification_type {nt} must map to Attention"
            );
        }
    }

    #[test]
    fn unrecognized_notification_type_keeps_the_fallback_event() {
        assert_eq!(
            refine_event_from_notification_type("some_future_type_we_dont_know_yet", CanonicalEvent::Attention),
            CanonicalEvent::Attention
        );
    }
}
