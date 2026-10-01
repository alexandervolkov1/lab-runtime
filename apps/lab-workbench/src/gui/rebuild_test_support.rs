//! Shared fake client plumbing for rebuild-coordinator unit tests.

use super::{CommandSendError, EventCursor, RebuildClient};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Sent {
    Query(u64, String, Value),
    Subscribe(u64, EventCursor, Value),
}

#[derive(Default)]
pub(super) struct FakeClient {
    pub(super) next: RefCell<u64>,
    pub(super) sent: RefCell<VecDeque<Sent>>,
    pub(super) subscriptions: Cell<usize>,
}

impl RebuildClient for FakeClient {
    fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
        let id = next_id(&self.next);
        self.sent
            .borrow_mut()
            .push_back(Sent::Query(id, op.to_owned(), args));
        Ok(id)
    }

    fn subscribe(&self, after: EventCursor, filter: Value) -> Result<u64, CommandSendError> {
        let id = next_id(&self.next);
        self.subscriptions.set(self.subscriptions.get() + 1);
        self.sent
            .borrow_mut()
            .push_back(Sent::Subscribe(id, after, filter));
        Ok(id)
    }
}

fn next_id(next: &RefCell<u64>) -> u64 {
    let mut value = next.borrow_mut();
    *value += 1;
    *value
}
