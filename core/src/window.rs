//! The action window: Maya announces what she is about to do, waits
//! `WAIT_MS`, then runs it unless the user says stop or corrects her. Pure
//! state, no I/O and no clock of its own: the caller starts the clock once the
//! announcement has been said and asks for the batch when its timer fires.

/// How long a batch waits after its announcement, in milliseconds.
pub const WAIT_MS: u64 = 5_000;

/// Who a request or a batch came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Voice,
    Chat,
    Phone,
    Maya,
}

/// Actions announced together and run together.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub id: u64,
    /// What Maya says to announce it.
    pub say: String,
    pub actions: Vec<serde_json::Value>,
    pub origin: Via,
    /// The deadline in milliseconds; `u64::MAX` until `start_clock`.
    pub until: u64,
}

#[derive(Default)]
pub struct Window {
    batch: Option<Batch>,
    next: u64,
}

impl Window {
    /// Opens a batch, or joins the open one (its actions appended, its `say`
    /// joined with " "); the deadline is unset (`u64::MAX`) until
    /// `start_clock`. Returns the batch id.
    pub fn open(&mut self, say: String, actions: Vec<serde_json::Value>, origin: Via) -> u64 {
        if let Some(b) = &mut self.batch {
            b.actions.extend(actions);
            if !say.is_empty() {
                if !b.say.is_empty() {
                    b.say.push(' ');
                }
                b.say.push_str(&say);
            }
            return b.id;
        }
        self.next += 1;
        let id = self.next;
        self.batch = Some(Batch { id, say, actions, origin, until: u64::MAX });
        id
    }

    /// Starts the wait of batch `id`, once its announcement has been said, and
    /// returns the deadline. A batch whose clock already runs keeps its
    /// deadline; a stale `id` gives None.
    pub fn start_clock(&mut self, id: u64, now_ms: u64) -> Option<u64> {
        let b = self.batch.as_mut().filter(|b| b.id == id)?;
        if b.until == u64::MAX {
            b.until = now_ms + WAIT_MS;
        }
        Some(b.until)
    }

    /// Drops the open batch and returns it.
    pub fn cancel(&mut self) -> Option<Batch> {
        self.batch.take()
    }

    /// Takes the open batch now, whatever its deadline.
    pub fn take_now(&mut self) -> Option<Batch> {
        self.batch.take()
    }

    /// Takes batch `id` when its deadline has come. A timer of a batch that
    /// was cancelled or replaced finds nothing.
    pub fn take_due(&mut self, id: u64, now_ms: u64) -> Option<Batch> {
        match &self.batch {
            Some(b) if b.id == id && now_ms >= b.until => self.batch.take(),
            _ => None,
        }
    }

    pub fn pending(&self) -> Option<&Batch> {
        self.batch.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_batch_waits_five_seconds_from_its_clock_start() {
        let mut w = Window::default();
        let id = w.open("Telling api-fix: open the PR.".into(), vec![json!({"kind":"reply"})], Via::Voice);
        assert!(w.take_due(id, 1_000_000).is_none()); // no clock yet
        assert_eq!(w.start_clock(id, 10_000), Some(15_000));
        assert!(w.take_due(id, 14_999).is_none());
        assert_eq!(w.take_due(id, 15_000).unwrap().actions.len(), 1);
    }

    #[test]
    fn a_second_open_joins_the_running_batch_and_keeps_its_deadline() {
        let mut w = Window::default();
        let a = w.open("One.".into(), vec![json!(1)], Via::Voice);
        w.start_clock(a, 0);
        let b = w.open("Two.".into(), vec![json!(2)], Via::Voice);
        assert_eq!(a, b);
        let due = w.take_due(a, WAIT_MS).unwrap();
        assert_eq!(due.actions, vec![json!(1), json!(2)]);
        assert_eq!(due.say, "One. Two.");
    }

    #[test]
    fn a_stale_timer_does_not_run_a_newer_batch() {
        let mut w = Window::default();
        let a = w.open("A.".into(), vec![json!(1)], Via::Chat);
        w.cancel();
        let b = w.open("B.".into(), vec![json!(2)], Via::Chat);
        w.start_clock(b, 0);
        assert!(w.take_due(a, WAIT_MS).is_none());
    }

    #[test]
    fn take_now_runs_the_batch_before_its_deadline() {
        let mut w = Window::default();
        let id = w.open("A.".into(), vec![json!(1)], Via::Phone);
        w.start_clock(id, 0);
        assert_eq!(w.take_now().unwrap().origin, Via::Phone);
        assert!(w.pending().is_none());
        assert!(w.take_now().is_none());
    }

    #[test]
    fn via_is_written_in_lowercase() {
        assert_eq!(serde_json::to_string(&Via::Voice).unwrap(), "\"voice\"");
        assert_eq!(serde_json::from_str::<Via>("\"maya\"").unwrap(), Via::Maya);
    }
}
