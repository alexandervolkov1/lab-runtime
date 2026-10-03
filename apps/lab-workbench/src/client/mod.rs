//! Private bounded Application client used only by `lab-workbench`.

mod framing;
pub(crate) mod types;
mod worker;

pub(crate) use types::ClientUpdate;
pub(crate) use worker::ClientHandle;

#[cfg(test)]
mod contract_tests {
    use super::*;
    use std::sync::mpsc::{TrySendError, sync_channel};

    #[test]
    fn accepted_client_bounds_are_frozen() {
        assert_eq!(types::COMMAND_QUEUE, 32);
        assert_eq!(types::MAX_IN_FLIGHT, 8);
        assert_eq!(types::UPDATE_QUEUE, 64);
        assert_eq!(types::APPLICATION_JSON_LIMIT, 16_383);
        assert_eq!(types::FRAME_LIMIT, 16_384);
    }

    #[test]
    fn worker_control_query_classification_is_canonical_and_frozen() {
        assert_eq!(
            types::WorkerControlQuery::ALL.map(types::WorkerControlQuery::operation),
            ["operation_status", "subscribe", "unsubscribe"]
        );
        for operation in ["reference", "recording_status", "future_runtime_query"] {
            assert_eq!(types::WorkerControlQuery::from_operation(operation), None);
        }
    }

    #[test]
    fn command_mailbox_rejects_the_thirty_third_item_without_blocking() {
        let (sender, _receiver) = sync_channel(types::COMMAND_QUEUE);
        for value in 0..types::COMMAND_QUEUE {
            sender.try_send(value).unwrap();
        }
        assert_eq!(
            sender.try_send(types::COMMAND_QUEUE),
            Err(TrySendError::Full(types::COMMAND_QUEUE))
        );
    }
}
