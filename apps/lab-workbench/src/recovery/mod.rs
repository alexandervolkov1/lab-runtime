//! Durable, bounded mutation recovery state separate from presentation.
#![allow(
    dead_code,
    unused_imports,
    reason = "journal inspection APIs precede the M14.4 GUI consumer"
)]

mod journal;

pub(crate) use journal::{
    JournalAdmission, JournalDisposition, JournalError, JournalRecord, RecoveryJournal,
    default_journal_path, load_journal, retire_journal, save_journal,
};

#[cfg(test)]
mod tests;
