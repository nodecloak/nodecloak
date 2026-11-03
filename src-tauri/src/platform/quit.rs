//! Normal quit is asynchronous. A missing AppKit registration is not dispatch.
use std::time::{Duration, Instant};

pub(super) fn wait(
    timeout: Duration,
    mut discover: impl FnMut() -> Result<Vec<u32>, String>,
    mut request: impl FnMut(u32) -> Result<bool, String>,
    mut closed: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    // Dispatch acknowledgement is not exit/consent. A save prompt may still be
    // open, so only lookups that have not sent a request may be retried.
    let mut dispatched = std::collections::HashSet::new();
    loop {
        match closed() {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        for pid in discover()? {
            if !dispatched.contains(&pid) && request(pid)? {
                dispatched.insert(pid);
            }
        }
        match closed() {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        std::thread::sleep(
            Duration::from_millis(500).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn unregistered_application_is_retried_until_quit_can_be_dispatched() {
        let calls = Cell::new(0);
        let delivered = Cell::new(false);
        let result = wait(
            Duration::from_secs(2),
            || Ok(vec![17]),
            |pid| {
                assert_eq!(pid, 17);
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    return Ok(false);
                }
                delivered.set(true);
                Ok(true)
            },
            || {
                if delivered.get() {
                    Ok(())
                } else {
                    Err("profile remains open".into())
                }
            },
        );
        assert!(
            result.is_ok(),
            "missed AppKit registration must not discard quit: {result:?}"
        );
        assert_eq!(calls.get(), 2);
    }
    #[test]
    fn dispatched_quit_is_not_reissued_while_user_may_be_deciding() {
        let calls = Cell::new(0);
        let result = wait(
            Duration::from_millis(750),
            || Ok(vec![17]),
            |_| {
                calls.set(calls.get() + 1);
                Ok(true)
            },
            || Err("specific busy state".into()),
        );
        assert_eq!(result, Err("specific busy state".into()));
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn explicit_refusal_from_a_live_process_is_preserved() {
        let calls = Cell::new(0);
        let result = wait(
            Duration::from_millis(10),
            || Ok(vec![17]),
            |_| {
                calls.set(calls.get() + 1);
                Err("normal quit refused".into())
            },
            || Err("profile remains open".into()),
        );
        assert_eq!(result, Err("normal quit refused".into()));
        assert_eq!(calls.get(), 1);
    }
}
