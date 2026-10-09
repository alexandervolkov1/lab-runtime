//! Pre-side-effect credit for rebind baselines and compatibility probe results.

use super::*;

const RECORDS: usize = 256;

impl RecorderWorker {
    /// Reserve the existing maximum causal-group envelope, not a new queue slot.
    /// Missing capacity is bounded backpressure before the producer runs.
    pub(crate) fn reserve_fact_group(&mut self) -> Result<Option<u64>, StorageError> {
        if self.poll().state != RecordingState::Recording {
            return Err(StorageError(
                "fact reservation requires active recording".into(),
            ));
        }
        if !self.ordinary_capacity_available(RECORDS, MAX_GROUP_BYTES) {
            return Ok(None);
        }
        let token = self.next_fact_reservation;
        self.next_fact_reservation = token
            .checked_add(1)
            .ok_or_else(|| StorageError("fact reservation identity exhausted".into()))?;
        self.charged_records += RECORDS;
        self.charged_bytes += MAX_GROUP_BYTES;
        self.charged_groups += 1;
        self.reserved_fact_groups.insert(token);
        Ok(Some(token))
    }

    /// Cancel an unused envelope. No FIFO identity or submission time was assigned.
    pub(crate) fn cancel_fact_group(&mut self, token: u64) -> Result<(), StorageError> {
        if !self.reserved_fact_groups.remove(&token) {
            return Err(StorageError("unknown fact reservation".into()));
        }
        self.charged_records -= RECORDS;
        self.charged_bytes -= MAX_GROUP_BYTES;
        self.charged_groups -= 1;
        Ok(())
    }

    /// Convert one reservation into one exact FIFO transfer on the sole owner.
    /// No producer/query can interleave between cancelling the unused envelope
    /// and charging the actual group. Transferred credits still await SQL receipt.
    pub(crate) fn admit_reserved_facts(
        &mut self,
        token: u64,
        facts: Vec<RecordingFact>,
        at: Duration,
    ) -> Result<(), StorageError> {
        self.cancel_fact_group(token)?;
        if facts.is_empty() {
            return Ok(());
        }
        self.try_admit_at(facts, at).map(|_| ())
    }
}
