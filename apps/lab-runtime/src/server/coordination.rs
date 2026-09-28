//! Transport-neutral bounded connection and Application-delivery coordination.
//!
//! This module owns process connection identities, the one global admission pool,
//! per-connection exchange/queue state, and fair owner-side request selection. It
//! deliberately knows nothing about sockets, NDJSON delimiters, or partial writes.

use crate::wire::WireRequest;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{CLIENT_EVENTS, CLIENT_IN, CLIENT_OUT, MAX_CLIENTS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AdmissionError {
    Capacity,
    Exhausted,
}

/// One process-global identity and admission owner for every network transport.
pub(super) struct ConnectionCoordinator {
    next_id: u64,
    active: BTreeSet<u64>,
    pending_detach: VecDeque<u64>,
}

impl ConnectionCoordinator {
    pub(super) fn new() -> Self {
        Self {
            next_id: 1,
            active: BTreeSet::new(),
            pending_detach: VecDeque::new(),
        }
    }

    pub(super) fn admit(&mut self) -> Result<u64, AdmissionError> {
        if self.capacity_full() {
            return Err(AdmissionError::Capacity);
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(AdmissionError::Exhausted)?;
        let inserted = self.active.insert(id);
        debug_assert!(inserted, "connection IDs are never reused");
        Ok(id)
    }

    pub(super) fn capacity_full(&self) -> bool {
        self.active.len() + self.pending_detach.len() >= MAX_CLIENTS
    }

    pub(super) fn begin_detach(&mut self, id: u64) -> bool {
        if self.active.remove(&id) {
            self.pending_detach.push_back(id);
            true
        } else {
            false
        }
    }

    pub(super) fn pending_detach(&self) -> Option<u64> {
        self.pending_detach.front().copied()
    }

    pub(super) fn detach_delivered(&mut self, id: u64) {
        debug_assert_eq!(self.pending_detach.front(), Some(&id));
        let removed = self.pending_detach.pop_front();
        debug_assert_eq!(removed, Some(id));
    }

    pub(super) fn active_count(&self) -> usize {
        self.active.len()
    }

    pub(super) fn active_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.active.iter().copied()
    }
}

/// Per-connection Application exchange and bounded outgoing message state.
pub(super) struct ClientDelivery {
    replied_hello: bool,
    replies: VecDeque<Vec<u8>>,
    events: VecDeque<Vec<u8>>,
    last_reply: bool,
    pending: usize,
    pending_ids: BTreeSet<String>,
    rejection: Option<Vec<u8>>,
    closing: bool,
}

impl ClientDelivery {
    pub(super) fn new() -> Self {
        Self {
            replied_hello: false,
            replies: VecDeque::new(),
            events: VecDeque::new(),
            last_reply: false,
            pending: 0,
            pending_ids: BTreeSet::new(),
            rejection: None,
            closing: false,
        }
    }

    pub(super) fn pending_full(&self) -> bool {
        self.pending >= CLIENT_IN
    }

    pub(super) fn request_is_in_flight(&self, msg_id: &str) -> bool {
        self.pending_ids.contains(msg_id)
    }

    pub(super) fn request_admitted(&mut self, msg_id: String) {
        let inserted = self.pending_ids.insert(msg_id);
        debug_assert!(inserted);
        self.pending += 1;
    }

    pub(super) fn queued(&self, writing: Option<bool>) -> usize {
        self.replies.len()
            + self.events.len()
            + usize::from(writing.is_some())
            + usize::from(self.rejection.is_some())
    }

    pub(super) fn reply_queued(&self, writing: Option<bool>) -> usize {
        self.replies.len() + usize::from(writing == Some(true))
    }

    pub(super) fn event_queued(&self, writing: Option<bool>) -> usize {
        self.events.len() + usize::from(writing == Some(false))
    }

    pub(super) fn reject_and_close(&mut self, message: Vec<u8>) {
        self.rejection = Some(message);
        self.closing = true;
    }

    pub(super) fn stage_rejection(&mut self, writing: Option<bool>) {
        if self.pending_ids.is_empty()
            && self.reply_queued(writing) < CLIENT_OUT
            && let Some(message) = self.rejection.take()
        {
            self.replies.push_back(message);
        }
    }

    pub(super) fn push_reply(
        &mut self,
        message: Vec<u8>,
        consumed: bool,
        hello: bool,
        writing: Option<bool>,
    ) -> bool {
        if self.reply_queued(writing) >= CLIENT_OUT {
            return false;
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&message)
            && let Some(msg_id) = value["msg_id"].as_str()
            && !(value["type"] == "operation" && value["state"] == "accepted")
        {
            self.pending_ids.remove(msg_id);
        }
        self.replied_hello |= hello;
        self.replies.push_back(message);
        if consumed {
            self.pending = self.pending.saturating_sub(1);
        }
        true
    }

    pub(super) fn push_event(&mut self, message: Vec<u8>, writing: Option<bool>) -> bool {
        if self.event_queued(writing) >= CLIENT_EVENTS {
            return false;
        }
        self.events.push_back(message);
        true
    }

    pub(super) fn next_message(&mut self) -> Option<(Vec<u8>, bool)> {
        let pick_event = self.last_reply && !self.events.is_empty();
        if pick_event {
            self.events.pop_front().map(|message| (message, false))
        } else {
            self.replies
                .pop_front()
                .map(|message| (message, true))
                .or_else(|| self.events.pop_front().map(|message| (message, false)))
        }
    }

    pub(super) fn message_written(&mut self, reply: bool) {
        self.last_reply = reply;
    }

    pub(super) fn replied_hello(&self) -> bool {
        self.replied_hello
    }

    pub(super) fn close(&mut self) {
        self.closing = true;
    }

    pub(super) fn closing(&self) -> bool {
        self.closing
    }
}

/// Serialized owner-side request queues and connection close lifecycle.
pub(super) struct OwnerDelivery {
    queued: BTreeMap<u64, VecDeque<WireRequest>>,
    closing: BTreeSet<u64>,
    close_sent: BTreeSet<u64>,
    rotation: usize,
}

impl OwnerDelivery {
    pub(super) fn new() -> Self {
        Self {
            queued: BTreeMap::new(),
            closing: BTreeSet::new(),
            close_sent: BTreeSet::new(),
            rotation: 0,
        }
    }

    pub(super) fn admit_request(&mut self, id: u64, request: WireRequest) {
        if self.closing.contains(&id) {
            return;
        }
        let queue = self.queued.entry(id).or_default();
        if queue.len() < CLIENT_IN {
            queue.push_back(request);
        }
    }

    pub(super) fn next_requests(&mut self) -> Vec<(u64, WireRequest)> {
        let ids: Vec<_> = self.queued.keys().copied().collect();
        let mut requests = Vec::with_capacity(ids.len().min(4));
        if !ids.is_empty() {
            for offset in 0..ids.len().min(4) {
                let id = ids[(self.rotation + offset) % ids.len()];
                if let Some(request) = self.queued.get_mut(&id).and_then(VecDeque::pop_front) {
                    requests.push((id, request));
                }
            }
            self.rotation = (self.rotation + 1) % ids.len();
        }
        requests
    }

    pub(super) fn connection_ids(&self) -> Vec<u64> {
        self.queued.keys().copied().collect()
    }

    /// Mark the Application detached and the transport close pending exactly once.
    pub(super) fn begin_close(&mut self, id: u64) -> bool {
        self.queued.remove(&id);
        self.closing.insert(id)
    }

    /// Finish network detach state and report whether Application detach is needed.
    pub(super) fn network_detached(&mut self, id: u64) -> bool {
        self.queued.remove(&id);
        self.close_sent.remove(&id);
        !self.closing.remove(&id)
    }

    pub(super) fn unsent_closes(&self) -> Vec<u64> {
        self.closing.difference(&self.close_sent).copied().collect()
    }

    pub(super) fn close_sent(&mut self, id: u64) {
        self.queued.remove(&id);
        self.close_sent.insert(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(msg_id: &str) -> WireRequest {
        WireRequest {
            v: 1,
            msg_id: msg_id.to_owned(),
            op: "discover".to_owned(),
            args: json!({}),
            request_id: None,
        }
    }

    #[test]
    fn global_ids_capacity_and_pending_detach_are_checked_once() {
        let mut connections = ConnectionCoordinator::new();
        for expected in 1..=MAX_CLIENTS as u64 {
            assert_eq!(connections.admit(), Ok(expected));
        }
        assert_eq!(connections.admit(), Err(AdmissionError::Capacity));
        assert!(connections.begin_detach(3));
        assert!(!connections.begin_detach(3));
        assert!(!connections.begin_detach(99));
        assert_eq!(connections.admit(), Err(AdmissionError::Capacity));
        assert_eq!(connections.pending_detach(), Some(3));
        connections.detach_delivered(3);
        assert_eq!(connections.admit(), Ok(9));
    }

    #[test]
    fn connection_id_exhaustion_is_explicit_and_never_wraps() {
        let mut connections = ConnectionCoordinator::new();
        connections.next_id = u64::MAX;
        assert_eq!(connections.admit(), Err(AdmissionError::Exhausted));
        assert_eq!(connections.active_count(), 0);
    }

    #[test]
    fn owner_fairness_and_detach_lifecycle_remain_bounded_and_exactly_once() {
        let mut owner = OwnerDelivery::new();
        for id in 1..=5 {
            owner.admit_request(id, request(&id.to_string()));
        }
        assert_eq!(
            owner
                .next_requests()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        for id in 1..=5 {
            owner.admit_request(id, request(&format!("again-{id}")));
        }
        assert_eq!(
            owner
                .next_requests()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
        );
        assert!(owner.begin_close(2));
        assert!(!owner.begin_close(2));
        assert!(!owner.network_detached(2));
        assert!(owner.network_detached(3));
    }

    #[test]
    fn client_delivery_preserves_exchange_queue_and_rejection_policy() {
        let mut delivery = ClientDelivery::new();
        delivery.request_admitted("pending".to_owned());
        assert!(delivery.request_is_in_flight("pending"));
        delivery.reject_and_close(br#"{"type":"error"}"#.to_vec());
        delivery.stage_rejection(None);
        assert!(delivery.next_message().is_none());
        assert!(delivery.push_reply(
            br#"{"msg_id":"pending","type":"result"}"#.to_vec(),
            true,
            false,
            None,
        ));
        delivery.stage_rejection(None);
        assert!(delivery.next_message().is_some());
        delivery.message_written(true);
        assert!(delivery.next_message().is_some());

        let mut bounded = ClientDelivery::new();
        for _ in 0..CLIENT_OUT {
            assert!(bounded.push_reply(Vec::new(), false, false, None));
        }
        assert!(!bounded.push_reply(Vec::new(), false, false, None));
        let mut bounded = ClientDelivery::new();
        for _ in 0..CLIENT_EVENTS {
            assert!(bounded.push_event(Vec::new(), None));
        }
        assert!(!bounded.push_event(Vec::new(), None));
    }
}
