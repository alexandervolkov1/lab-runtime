//! One bounded owner turn and its safety-first native-work ordering.
//!
//! The methods here remain implementations of the single HostCore owner declared
//! in the parent module; this module introduces no additional mutable state owner.

use super::*;

impl HostCore {
    /// Drive one bounded turn, checking safety before lower-priority units.
    pub fn service(&mut self, clock: &impl Clock) -> Result<ServiceReport, Error> {
        let now = clock.now();
        self.poll_recorder(now);
        let result = self.service_inner(clock);
        self.admit_recording_facts(clock.now());
        self.poll_recorder(clock.now());
        result
    }

    fn service_inner(&mut self, clock: &impl Clock) -> Result<ServiceReport, Error> {
        let mut report = ServiceReport::default();
        let now = clock.now();
        if now < self.last_now {
            return Err(Error::InvalidConfiguration(
                "scheduler clock moved backward",
            ));
        }
        self.last_now = now;
        if let Some(skipped) = self.plan.safety.take(now)? {
            self.runtime.command(if self.stopping {
                Command::ServiceSafety { at: clock.now() }
            } else {
                Command::PollComponents { at: clock.now() }
            })?;
            self.observe(clock.now(), None)?;
            report.safety += 1;
            report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
        }
        if self.stopping && self.outputs_safe_for_transport_close() {
            for resource in self.resources.clone() {
                if self.closed_resources.contains(&resource) {
                    continue;
                }
                if self.runtime.shutdown_transport(resource, clock.now())?
                    == lab_core::transport::TransportShutdown::Complete
                {
                    self.closed_resources.insert(resource);
                }
            }
        }
        if self.stopping {
            return Ok(report);
        }
        if self.configuration_quiesced {
            return Ok(report);
        }
        for (plant, slot) in &mut self.plan.plants {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                let outcome = self.runtime.command(Command::RefreshMeasurement {
                    instrument: *plant,
                    parameter: lab_core::TEMPERATURE,
                    at: clock.now(),
                });
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
                if let Err(error) = outcome
                    && !matches!(error, Error::MeasurementUnavailable { .. })
                {
                    return Err(error);
                }
                report.measurements += 1;
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
            }
        }
        for read in &mut self.plan.metakon_reads {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if self
                .runtime
                .metakon_binding(read.instrument)
                .is_some_and(|binding| {
                    self.reconnect_quiesced_resources
                        .contains(&binding.resource)
                })
            {
                continue;
            }
            if let Some(skipped) = read.slot.take(now)? {
                let outcome = self.runtime.command(Command::QueueMetakonRead {
                    instrument: read.instrument,
                    parameter: read.parameter,
                    at: clock.now(),
                    queue_ttl: read.queue_ttl,
                    timeout: read.timeout,
                });
                if let Err(error) = outcome
                    && !matches!(error, Error::Transport(_))
                {
                    return Err(error);
                }
                report.measurements += 1;
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
            }
        }
        for (reference, slot) in &mut self.plan.references {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                self.runtime.command(Command::EvaluateReference {
                    reference: *reference,
                    at: clock.now(),
                })?;
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
                report.references += 1;
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
            }
        }
        for (controller, input, slot) in &mut self.plan.controllers {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
                let QueryResult::Controller(state) =
                    self.runtime.query(Query::Controller(*controller))?
                else {
                    unreachable!()
                };
                if !matches!(
                    state.state,
                    ControllerState::Warming | ControllerState::Running
                ) {
                    continue;
                }
                let QueryResult::Latest(latest) =
                    self.runtime.query(Query::GetLatestSignal(*input))?
                else {
                    unreachable!()
                };
                if latest
                    .as_ref()
                    .is_some_and(|sample| sample.quality() == SampleQuality::Good)
                    && latest != self.consumed.get(controller).cloned().flatten()
                {
                    self.runtime.command(Command::TickController {
                        controller: *controller,
                        at: clock.now(),
                    })?;
                    self.events
                        .observe(&self.runtime, clock.now(), None)
                        .map_err(event_domain_error)?;
                    self.consumed.insert(*controller, latest);
                    report.controller_ticks += 1;
                }
            }
        }
        for (source, slot) in &mut self.plan.sources {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            if let Some(skipped) = slot.take(now)? {
                report.skipped_deadlines = report.skipped_deadlines.saturating_add(skipped);
                let status = self.runtime.query(Query::Component(*source));
                if !matches!(status,Ok(QueryResult::Component(s)) if s.state!=ComponentState::Failed && s.pending.is_none())
                {
                    continue;
                }
                let outcome = self.runtime.command(Command::InvokeComponent {
                    component: *source,
                    at: clock.now(),
                });
                self.events
                    .observe(&self.runtime, clock.now(), None)
                    .map_err(event_domain_error)?;
                if let Err(error) = outcome
                    && !matches!(
                        error,
                        Error::Component(ComponentError::Busy | ComponentError::InputUnavailable)
                    )
                {
                    return Err(error);
                }
            }
        }
        for (transform, input) in &self.plan.transforms {
            let now = clock.now();
            if self.plan.safety.due(now) {
                return Ok(report);
            }
            let status = self.runtime.query(Query::Component(*transform));
            if !matches!(status,Ok(QueryResult::Component(s)) if s.state!=ComponentState::Failed && s.pending.is_none())
            {
                continue;
            }
            let latest = match self.runtime.query(Query::GetLatestSignal(*input))? {
                QueryResult::Latest(s) => s,
                _ => unreachable!(),
            };
            if latest.is_none() || latest == self.consumed_managed.get(transform).cloned().flatten()
            {
                continue;
            }
            let outcome = self.runtime.command(Command::InvokeComponent {
                component: *transform,
                at: clock.now(),
            });
            self.events
                .observe(&self.runtime, clock.now(), None)
                .map_err(event_domain_error)?;
            if let Err(error) = outcome
                && !matches!(
                    error,
                    Error::Component(ComponentError::Busy | ComponentError::InputUnavailable)
                )
            {
                return Err(error);
            }
            self.consumed_managed.insert(*transform, latest);
        }
        Ok(report)
    }
}
