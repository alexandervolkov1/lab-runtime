//! Finite service shutdown progression and evidence-preserving worker retirement.
//!
//! These methods advance the single ServiceHost owner declared in the parent
//! module; no reconnect, deployment, or shutdown state is copied into a manager.

use super::*;

impl ServiceHost {
    /// Raise the producer stop barrier before subsequent network or worker work.
    pub fn request_shutdown(&mut self) -> Result<(), DomainError> {
        if self.stopping_since.is_some() {
            return Ok(());
        }
        self.stopping_since = Some(std::time::Instant::now());
        if let Some((_, candidate)) = self.quarantined_reconnect_candidate.as_mut() {
            candidate.retire();
        }
        let clock = self.clock;
        if let Err(error) = self.host.begin_shutdown(&clock) {
            self.fatal = true;
            return Err(error);
        }
        let at = self.clock.now();
        self.host
            .event_log_mut()
            .host_state(at, "stopping", serde_json::json!({}))
            .map_err(|_| {
                self.fatal = true;
                DomainError::InvalidConfiguration("host event limit")
            })?;
        Ok(())
    }
    /// Record a fatal owner fault and enter the same bounded evidence-preserving
    /// shutdown path; a failed stop step still leaves the grace state active.
    pub fn request_fatal_shutdown(&mut self) {
        self.fatal = true;
        let _ = self.request_shutdown();
    }
    /// Stop barrier is raised before further client mutation admission.
    pub const fn is_stopping(&self) -> bool {
        self.stopping_since.is_some()
    }

    /// Progress trusted safe work once; never sleep or join on the owner lane.
    /// The caller keeps sweeping clients/requests between these bounded turns.
    pub fn shutdown_step(&mut self) -> Result<Option<ShutdownStatus>, DomainError> {
        if let Some(terminal) = self.terminal {
            return Ok(Some(terminal));
        }
        let Some(started) = self.stopping_since else {
            return Ok(None);
        };
        let clock = self.clock;
        if self.host.service(&clock).is_err() {
            self.fatal = true;
        }
        let candidate_pending =
            if let Some((_, candidate)) = self.quarantined_reconnect_candidate.as_mut() {
                if candidate.try_shutdown() == lab_core::transport::TransportShutdown::Complete {
                    self.quarantined_reconnect_candidate = None;
                    false
                } else {
                    true
                }
            } else {
                false
            };
        let mut status = self.host.shutdown_status();
        if candidate_pending {
            status.unfinished_transports = status.unfinished_transports.saturating_add(1);
            status.transports_closed = false;
            status.exit_success = false;
        }
        status.fatal_error = self.fatal;
        status.exit_success &= !self.fatal;
        let now = std::time::Instant::now();
        let safety_finished = if status.safe_confirmed {
            self.safe_since.get_or_insert(now);
            status.unfinished_workers == 0
                || self.safe_since.is_some_and(|safe_at| {
                    now.duration_since(safe_at) >= std::time::Duration::from_millis(200)
                })
        } else {
            now.duration_since(started) >= std::time::Duration::from_secs(2)
        };
        if safety_finished {
            self.recorder_flush_since.get_or_insert(now);
            self.host.shutdown_recorder_step(self.clock.now());
            status = self.host.shutdown_status();
            if self.quarantined_reconnect_candidate.is_some() {
                status.unfinished_transports = status.unfinished_transports.saturating_add(1);
                status.transports_closed = false;
                status.exit_success = false;
            }
            status.fatal_error = self.fatal;
            status.exit_success &= !self.fatal;
            let flush_expired = self
                .recorder_flush_since
                .is_some_and(|at| now.duration_since(at) >= std::time::Duration::from_secs(2));
            // A nonblocking transport retirement normally needs another owner
            // turn to observe the worker's terminal completion. Do not freeze
            // that ordinary Pending state as a failed terminal merely because
            // Recorder is already flushed. The existing absolute flush grace
            // remains the finite bound for a genuinely stuck transport.
            if (status.recorder_flushed && (status.transports_closed || !status.safe_confirmed))
                || flush_expired
            {
                self.terminal = Some(status);
            }
        }
        Ok(self.terminal)
    }
}
