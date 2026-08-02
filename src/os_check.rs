// Detects whether the OS will actually display a notification, independent
// of whether the underlying show() call reports success. On Windows, a
// disabled "Notifications" master toggle makes notify-rust's WinRT call
// return Ok(()) while Windows silently drops the toast - this exists to
// catch exactly that case and warn the operator instead of leaving them
// wondering why nothing ever appeared.
//
// macOS is deliberately not checked: the legacy NSUserNotificationCenter
// backend we use has the same "always looks like Ok" problem as Windows,
// but the reliable fix (UNUserNotificationCenter's real authorization
// status) requires a proper bundle identifier that a bare `cargo install`
// binary doesn't have - notify-rust itself only exposes that path behind
// an experimental feature for exactly this reason. The alternative
// (reading com.apple.ncprefs / the Notification Center SQLite db) means
// first resolving which *terminal emulator's* bundle id is actually
// relevant, then parsing an undocumented, OS-version-dependent Apple
// format - not verifiable without a live Mac. Left unimplemented rather
// than guessed, same as Antigravity's unconfirmed attention event.

/// `Some(true)`/`Some(false)` when the platform's setting is known;
/// `None` when this platform isn't checked yet (assume enabled - never
/// warn on a platform we haven't verified a real signal for).
pub fn os_notifications_enabled() -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        windows::toast_enabled()
    }
    #[cfg(target_os = "linux")]
    {
        linux::daemon_status()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    /// Interprets the raw `ToastEnabled` DWORD read from the registry.
    /// Pulled out so the decision logic is testable without touching a real
    /// registry: a missing key/value is `None` (unknown, don't warn) rather
    /// than assumed disabled, since some Windows configurations may not have
    /// written this value at all.
    fn interpret_toast_enabled(raw: Option<u32>) -> Option<bool> {
        match raw {
            Some(0) => Some(false),
            Some(_) => Some(true),
            None => None,
        }
    }

    pub fn toast_enabled() -> Option<bool> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\PushNotifications")
            .ok()?;
        let raw: Option<u32> = key.get_value("ToastEnabled").ok();
        interpret_toast_enabled(raw)
    }

    #[cfg(test)]
    mod tests {
        use super::interpret_toast_enabled;

        #[test]
        fn zero_means_disabled() {
            assert_eq!(interpret_toast_enabled(Some(0)), Some(false));
        }

        #[test]
        fn nonzero_means_enabled() {
            assert_eq!(interpret_toast_enabled(Some(1)), Some(true));
            assert_eq!(interpret_toast_enabled(Some(42)), Some(true));
        }

        #[test]
        fn missing_value_is_unknown_not_disabled() {
            assert_eq!(interpret_toast_enabled(None), None);
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    /// Whether a notification daemon is registered on the session D-Bus at
    /// all (org.freedesktop.Notifications). Returns `Some(true)` when a
    /// daemon answered, `Some(false)` when the well-known name has no owner
    /// (no daemon running), and `None` when the D-Bus session itself is
    /// unreachable (permission error, bus not running, etc.) - in that
    /// last case the daemon *might* still be reachable through another
    /// transport path, so do not warn.
    pub fn daemon_status() -> Option<bool> {
        match notify_rust::get_server_information() {
            Ok(_) => Some(true),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Name has no owner") || msg.contains("name has no owner") {
                    Some(false)
                } else {
                    None
                }
            }
        }
    }
}
