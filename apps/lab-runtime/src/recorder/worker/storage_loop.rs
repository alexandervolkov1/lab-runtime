//! Exclusive SQLite worker loop and command execution.

use super::*;

// Keep the immutable clock source and each bounded mailbox explicit at the one
// storage thread boundary; none is shared with the Runtime state owner.
#[expect(
    clippy::too_many_arguments,
    reason = "explicit storage thread ownership"
)]
pub(super) fn worker_loop(
    mut store: SqliteStore,
    receiver: mpsc::Receiver<Message>,
    receipt: &Arc<Mutex<Receipt>>,
    history: &Arc<Mutex<BTreeMap<u64, Result<HistoryPage, StorageError>>>>,
    runs: &Arc<Mutex<BTreeMap<u64, Result<RunsPage, StorageError>>>>,
    active_jobs: &Arc<Mutex<BTreeSet<u64>>>,
    barrier: Option<&WriterBarrier>,
    source: MonotonicSource,
) -> bool {
    let mut deferred: Option<Message> = None;
    loop {
        let message = if let Some(message) = deferred.take() {
            message
        } else {
            match receiver.try_recv() {
                Ok(message) => message,
                Err(mpsc::TryRecvError::Disconnected) => return false,
                Err(mpsc::TryRecvError::Empty) => {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(message) => message,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                    }
                }
            }
        };
        let message = match message {
            Message::History {
                job,
                filter,
                cursor,
                limit,
            } => {
                let result = store.read_history_measurements(&filter, cursor.as_ref(), limit);
                // Hold the short active-slot check through mailbox publication.
                // A cancellation cannot slip between the two and retain a late row.
                let active = active_jobs.lock().unwrap_or_else(|p| p.into_inner());
                if active.contains(&job) {
                    history
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(job, result);
                }
                continue;
            }
            Message::Runs { job, cursor, limit } => {
                let result = store.read_history_runs(cursor.as_ref(), limit);
                let active = active_jobs.lock().unwrap_or_else(|p| p.into_inner());
                if active.contains(&job) {
                    runs.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(job, result);
                }
                continue;
            }
            other => other,
        };
        if !matches!(
            message,
            Message::Finish(_, _, _) | Message::Activation { .. }
        ) && (!matches!(message, Message::Start(_, _, _, _, _))
            || barrier.is_some_and(|barrier| barrier.0.hold_start))
            && !barrier.is_some_and(|barrier| barrier.0.hold_after_fact_commit)
            && (!barrier.is_some_and(|barrier| barrier.0.hold_terminal_operation)
                || matches!(message, Message::Operation(ref operation, _, _)
                    if operation.phase != "accepted"))
            && let Some(barrier) = barrier
        {
            barrier.await_release();
        }
        if matches!(message, Message::Facts(..))
            && barrier
                .is_some_and(|fault| fault.0.fail_checkpoint_once.swap(false, Ordering::AcqRel))
        {
            store.fail_next_checkpoint_for_testing();
        }
        let result = match message {
            Message::Activation {
                entries,
                objects,
                lifecycle,
            } => {
                let root = if let Some((record, _, assigned, generation)) = &lifecycle {
                    store.commit_activation_lifecycle(
                        &entries,
                        &objects,
                        record,
                        *generation,
                        *assigned,
                    )
                } else {
                    store.commit_activation(&entries, &objects)
                };
                root.and_then(|root| {
                    let mut receipt = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    let expected = receipt
                        .activation_generation
                        .checked_add(1)
                        .ok_or_else(|| StorageError("activation generation exhausted".into()))?;
                    if lifecycle
                        .as_ref()
                        .is_some_and(|(_, _, _, generation)| *generation != expected)
                    {
                        return Err(StorageError("activation generation mismatch".into()));
                    }
                    receipt.activation_root = Some(root);
                    receipt.activation_generation = expected;
                    if let Some((record, bytes, assigned, _)) = lifecycle {
                        receipt.persisted = assigned;
                        // Resource-scoped activation may follow probe facts whose
                        // owner submission time is later than the lifecycle's
                        // original start. A later FIFO commit must not move the
                        // Required progress receipt backward.
                        receipt.confirmed_submission = Some(
                            receipt
                                .confirmed_submission
                                .map_or(record.at, |confirmed| confirmed.max(record.at)),
                        );
                        receipt.released_records += 1;
                        receipt.released_bytes += bytes;
                        receipt.released_groups += 1;
                    }
                    Ok(())
                })
            }
            Message::Start(label, policy, submitted_at, boundary, first_record) => {
                TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()))
                    .and_then(|anchor| {
                        store.start_run_with_boundary_anchor_assigned(
                            &label,
                            policy,
                            &boundary,
                            &anchor,
                            first_record,
                        )
                    })
                    .and_then(|_| {
                        let health = store.storage_health()?;
                        let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                        status.state = RecordingState::Recording;
                        status.persisted = store.current_record_sequence();
                        status.confirmed_submission = Some(submitted_at);
                        status.run_no = store.current_run_no();
                        status.interval_no = store.current_interval_no();
                        status.storage = Some(health);
                        Ok(())
                    })
            }
            Message::Facts(facts, bytes, submitted_at, queued_at, first_record) => {
                if let Some(barrier) = barrier.filter(|barrier| barrier.0.panic_before_fact_sql) {
                    barrier.mark_reached();
                    panic!("injected recorder worker panic before fact SQL");
                }
                let mut batch = vec![(facts, bytes, submitted_at, first_record)];
                let mut records = batch[0].0.len();
                let mut accounted_bytes = bytes;
                let deadline = queued_at + Duration::from_millis(100);
                while batch.len() < MAX_GROUPS && records < 256 && accounted_bytes < MAX_GROUP_BYTES
                {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match receiver.recv_timeout(remaining) {
                        Ok(Message::Facts(
                            next_facts,
                            next_bytes,
                            next_at,
                            next_queued,
                            next_first,
                        )) => {
                            if next_facts.len() > 256 - records
                                || next_bytes > MAX_GROUP_BYTES - accounted_bytes
                            {
                                deferred = Some(Message::Facts(
                                    next_facts,
                                    next_bytes,
                                    next_at,
                                    next_queued,
                                    next_first,
                                ));
                                break;
                            }
                            records += next_facts.len();
                            accounted_bytes += next_bytes;
                            batch.push((next_facts, next_bytes, next_at, next_first));
                        }
                        Ok(other) => {
                            deferred = Some(other);
                            break;
                        }
                        Err(
                            mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected,
                        ) => break,
                    }
                }
                let views: Vec<(&[RecordingFact], Duration)> = batch
                    .iter()
                    .map(|(facts, _, at, _)| (facts.as_slice(), *at))
                    .collect();
                let mut expected = batch[0].3;
                let aligned = batch.iter().all(|(facts, _, _, first)| {
                    if *first != expected {
                        return false;
                    }
                    let Some(next) = expected.checked_add(facts.len() as u64) else {
                        return false;
                    };
                    expected = next;
                    true
                });
                let last_submission = batch.last().expect("nonempty batch").2;
                let committed = if aligned {
                    store.append_fact_groups_assigned(&views, batch[0].3)
                } else {
                    Err(StorageError(
                        "noncontiguous owner record reservation".into(),
                    ))
                };
                committed.map(|sequence| {
                    if let Some(barrier) =
                        barrier.filter(|barrier| barrier.0.hold_after_fact_commit)
                    {
                        // The archive is already committed. A killed process
                        // can lose this receipt without losing the WAL record.
                        barrier.await_release();
                    }
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.confirmed_submission = Some(last_submission);
                    status.released_records += records;
                    status.released_bytes += accounted_bytes;
                    status.released_groups += batch.len();
                })
            }
            Message::Operation(operation, bytes, assigned) => store
                .append_operation_assigned(&operation, assigned)
                .map(|sequence| {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.confirmed_submission = Some(operation.at);
                    status.released_records += 1;
                    status.released_bytes += bytes;
                    status.released_groups += 1;
                }),
            Message::Annotation(annotation, bytes, assigned) => store
                .append_annotation_assigned(&annotation, assigned)
                .map(|sequence| {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.confirmed_submission = Some(annotation.at);
                    status.released_records += 1;
                    status.released_bytes += bytes;
                    status.released_groups += 1;
                }),
            Message::GapSeal(gap, assigned) => {
                store.fail_run_assigned(&gap, assigned).map(|sequence| {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.persisted = sequence;
                    status.failure_persisted = true;
                    status.run_no = None;
                    status.interval_no = None;
                })
            }
            Message::Probe(submitted_at) => store.probe(submitted_at).map(|_| {
                receipt
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .confirmed_submission = Some(submitted_at);
            }),
            Message::ClockAnchor(assigned) => TimeAnchor::capture(
                || source.now(),
                || {
                    if barrier.is_some_and(|held| {
                        held.0.fail_periodic_wall_read.swap(false, Ordering::AcqRel)
                    }) {
                        Err("wall_read_failed")
                    } else {
                        Ok(SystemTime::now())
                    }
                },
            )
            .and_then(|anchor| store.append_clock_anchor_assigned("periodic", &anchor, assigned))
            .map(|_| {
                receipt.lock().unwrap_or_else(|p| p.into_inner()).persisted =
                    store.current_record_sequence();
            }),
            Message::History { .. } => unreachable!("history was handled before lifecycle match"),
            Message::Runs { .. } => unreachable!("runs were handled before lifecycle match"),
            Message::Stop(summary, requested_at, assigned) => {
                TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()))
                    .and_then(|anchor| {
                        store.stop_run_with_anchor_summary_assigned(
                            &anchor,
                            &summary,
                            requested_at,
                            assigned,
                        )
                    })
                    .map(|_| {
                        let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                        status.state = RecordingState::Idle;
                        status.persisted = store.current_record_sequence();
                        status.run_no = None;
                        status.interval_no = None;
                    })
            }
            Message::Finish(summary, at, assigned) => {
                if let Some(barrier) = barrier.filter(|barrier| barrier.0.hold_finish) {
                    barrier.await_release();
                }
                let anchor = TimeAnchor::capture(|| source.now(), || Ok(SystemTime::now()));
                let result = anchor.and_then(|anchor| {
                    store.finish_boot_with_summary_assigned(at, &summary, &anchor, assigned)
                });
                if let Err(error) = result {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.state = RecordingState::Failed;
                    status
                        .first_error
                        .get_or_insert_with(|| error.to_string().chars().take(512).collect());
                    return false;
                }
                {
                    receipt
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .terminal_seal_committed = true;
                }
                if barrier.is_some_and(|fault| fault.0.fail_close_after_seal) {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.state = RecordingState::Failed;
                    status.first_error.get_or_insert_with(|| {
                        "injected sqlite close failure after terminal seal".into()
                    });
                    return false;
                }
                if let Err(error) = store.close() {
                    let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
                    status.state = RecordingState::Failed;
                    status
                        .first_error
                        .get_or_insert_with(|| error.to_string().chars().take(512).collect());
                    return false;
                }
                return true;
            }
        };
        let result = result.and_then(|()| {
            let health = store.storage_health()?;
            receipt
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .storage = Some(health);
            Ok(())
        });
        if let Err(error) = result {
            let mut status = receipt.lock().unwrap_or_else(|p| p.into_inner());
            status.state = RecordingState::Failed;
            status
                .first_error
                .get_or_insert_with(|| error.to_string().chars().take(512).collect());
            return false;
        }
    }
}
