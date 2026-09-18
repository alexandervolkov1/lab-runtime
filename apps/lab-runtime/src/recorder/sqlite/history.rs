//! Bounded durable-history and run-list queries over committed schema-v1 data.

use super::*;

// Reset the callback before the worker resumes fact commits. A budget failure
// must never turn an ordinary SQLite transaction into a history interrupt.
struct HistoryProgress<'a>(&'a Connection);
impl<'a> HistoryProgress<'a> {
    fn install(connection: &'a Connection, budget: HistoryBudget) -> Result<Self, StorageError> {
        let started = Instant::now();
        let mut executed = 0u32;
        connection.progress_handler(
            budget.stride,
            Some(move || {
                executed = executed.saturating_add(budget.stride as u32);
                executed >= budget.vm_ops || started.elapsed() >= budget.elapsed
            }),
        )?;
        Ok(Self(connection))
    }
}
impl Drop for HistoryProgress<'_> {
    fn drop(&mut self) {
        let _ = self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

impl SqliteStore {
    /// Discover archived runs through the primary-key index without COUNT/OFFSET.
    /// The first page freezes the upper boot/run key; summaries are time-of-page.
    pub fn read_history_runs(
        &self,
        cursor: Option<&RunsCursor>,
        limit: usize,
    ) -> Result<RunsPage, StorageError> {
        let _progress = HistoryProgress::install(&self.connection, HistoryBudget::default())?;
        if !(1..=32).contains(&limit) {
            return Err(StorageError("run page limit must be 1..=32".into()));
        }
        let (last, upper) = if let Some(cursor) = cursor {
            if cursor.database_id != self.database_id || cursor.last > cursor.upper {
                return Err(StorageError("run cursor does not match database".into()));
            }
            (Some(cursor.last), Some(cursor.upper))
        } else {
            let upper = self
                .connection
                .query_row(
                    "SELECT boot_id,run_no FROM runs ORDER BY boot_id DESC,run_no DESC LIMIT 1",
                    [],
                    |row| Ok((read_boot_blob(row.get(0)?)?, read_u64_blob(row.get(1)?)?)),
                )
                .optional()?;
            (None, upper)
        };
        let Some(upper) = upper else {
            return Ok(RunsPage {
                runs: Vec::new(),
                next_cursor: None,
            });
        };
        let last_boot = last.map_or([0u8; 16], |key| key.0);
        let last_run = u64_blob(last.map_or(0, |key| key.1));
        let upper_run = u64_blob(upper.1);
        let mut statement = self.connection.prepare(
            "SELECT boot_id,run_no,label,policy,state,coverage FROM runs
             WHERE (?1=0 OR boot_id>?2 OR (boot_id=?2 AND run_no>?3))
               AND (boot_id<?4 OR (boot_id=?4 AND run_no<=?5))
             ORDER BY boot_id,run_no LIMIT ?6",
        )?;
        let selected = statement.query_map(
            params![
                i64::from(last.is_some()),
                last_boot.as_slice(),
                last_run.as_slice(),
                upper.0.as_slice(),
                upper_run.as_slice(),
                i64::try_from(limit + 1).expect("bounded run lookahead")
            ],
            |row| {
                let boot = read_boot_blob(row.get(0)?)?;
                Ok(RunSummary {
                    boot_id: hex_boot(boot),
                    run_no: read_u64_blob(row.get(1)?)?,
                    label: row.get(2)?,
                    policy: row.get(3)?,
                    state: row.get(4)?,
                    coverage: row.get(5)?,
                })
            },
        )?;
        let mut runs = Vec::with_capacity(limit);
        let mut bytes = 0usize;
        let mut has_more = false;
        for selected_run in selected {
            let run = selected_run?;
            let charge =
                192 + run.label.len() + run.policy.len() + run.state.len() + run.coverage.len();
            if runs.len() == limit || bytes.saturating_add(charge) > 8 * 1024 {
                if runs.is_empty() {
                    return Err(StorageError("whole run exceeds page budget".into()));
                }
                has_more = true;
                break;
            }
            bytes += charge;
            runs.push(run);
        }
        let next_cursor = if has_more {
            let last_run = runs.last().expect("lookahead follows returned run");
            Some(RunsCursor {
                database_id: self.database_id.clone(),
                last: (parse_boot_id(&last_run.boot_id)?, last_run.run_no),
                upper,
            })
        } else {
            None
        };
        Ok(RunsPage { runs, next_cursor })
    }

    /// Select one indexed keyset page at a frozen durable checkpoint.
    /// The caller retains the returned cursor in a bounded connection slot;
    /// no read transaction or SQLite statement remains open between pages.
    pub fn read_history_measurements(
        &self,
        filter: &HistoryFilter,
        cursor: Option<&HistoryCursor>,
        limit: usize,
    ) -> Result<HistoryPage, StorageError> {
        self.read_history_measurements_with_budget(filter, cursor, limit, HistoryBudget::default())
    }

    /// Trusted smaller test budget; the production worker always uses defaults.
    pub fn read_history_measurements_with_budget(
        &self,
        filter: &HistoryFilter,
        cursor: Option<&HistoryCursor>,
        limit: usize,
        budget: HistoryBudget,
    ) -> Result<HistoryPage, StorageError> {
        let _progress = HistoryProgress::install(&self.connection, budget)?;
        if !(1..=MAX_RAW_PAGE).contains(&limit) || filter.from >= filter.to {
            return Err(StorageError(
                "invalid bounded history range or page limit".into(),
            ));
        }
        let archive_boot = parse_boot_id(&filter.boot_id)?;
        let run_no = u64_blob(filter.run_no);
        let instrument_id = u64_blob(filter.instrument.get());
        let parameter_id = u64_blob(filter.parameter.get());
        let from = duration_blob(filter.from)?;
        let to = duration_blob(filter.to)?;
        let coverage: String = self
            .connection
            .query_row(
                "SELECT coverage FROM runs WHERE boot_id=?1 AND run_no=?2",
                params![archive_boot.as_slice(), run_no.as_slice()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StorageError("unknown archived run".into()))?;
        let loss = if coverage == "gap" {
            let mut statement = self.connection.prepare(
                "SELECT loss_summary FROM recording_intervals INDEXED BY intervals_by_run
                 WHERE boot_id=?1 AND run_no=?2 LIMIT 2",
            )?;
            let summaries = statement
                .query_map(params![archive_boot.as_slice(), run_no.as_slice()], |row| {
                    row.get::<_, Option<String>>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if summaries.len() != 1 {
                return Err(StorageError("ambiguous archived gap interval".into()));
            }
            Some(parse_history_loss(summaries[0].as_deref().ok_or_else(
                || StorageError("archived gap has no loss summary".into()),
            )?)?)
        } else {
            None
        };
        let loss_charge = loss.as_ref().map_or(0, |loss| {
            256usize.saturating_add(loss.reason.len().saturating_mul(6))
        });
        let current_watermark = self.connection.query_row(
            "SELECT persisted_through_seq FROM durable_checkpoints WHERE boot_id=?1",
            params![archive_boot.as_slice()],
            |row| read_u64_blob(row.get(0)?),
        )?;
        let (watermark, last, upper) = if let Some(cursor) = cursor {
            if cursor.filter != *filter
                || cursor.database_id != self.database_id
                || cursor.watermark > current_watermark
                || cursor.last > cursor.upper
            {
                return Err(StorageError(
                    "history cursor does not match archive/filter".into(),
                ));
            }
            (cursor.watermark, Some(cursor.last), Some(cursor.upper))
        } else {
            let frozen = u64_blob(current_watermark);
            let upper = self
                .connection
                .query_row(
                    "SELECT published_at,record_seq FROM measurements
                 WHERE boot_id=?1 AND run_no=?2 AND instrument_id=?3 AND parameter_id=?4
                   AND published_at>=?5 AND published_at<?6 AND record_seq<=?7
                 ORDER BY published_at DESC,record_seq DESC LIMIT 1",
                    params![
                        archive_boot.as_slice(),
                        run_no.as_slice(),
                        instrument_id.as_slice(),
                        parameter_id.as_slice(),
                        from.as_slice(),
                        to.as_slice(),
                        frozen.as_slice()
                    ],
                    |row| Ok((read_u64_blob(row.get(0)?)?, read_u64_blob(row.get(1)?)?)),
                )
                .optional()?;
            (current_watermark, None, upper)
        };
        let Some(upper) = upper else {
            return Ok(HistoryPage {
                rows: Vec::new(),
                selected_rows: 0,
                watermark,
                next_cursor: None,
                coverage,
                loss,
            });
        };
        let frozen = u64_blob(watermark);
        let upper_at = u64_blob(upper.0);
        let upper_seq = u64_blob(upper.1);
        let last_at = u64_blob(last.map_or(0, |key| key.0));
        let last_seq = u64_blob(last.map_or(0, |key| key.1));
        let mut statement = self.connection.prepare(
            "SELECT record_seq,published_at,observed_at,unit_key,quality,failure,
                    value_kind,float_value,integer_value,bool_value,text_value,generation,revision,
                    state_revision,lineage
             FROM measurements
             WHERE boot_id=?1 AND run_no=?2 AND instrument_id=?3 AND parameter_id=?4
               AND published_at>=?5 AND published_at<?6 AND record_seq<=?7
               AND (?8=0 OR published_at>?9 OR (published_at=?9 AND record_seq>?10))
               AND (published_at<?11 OR (published_at=?11 AND record_seq<=?12))
             ORDER BY published_at,record_seq LIMIT ?13",
        )?;
        let selected = statement.query_map(
            params![
                archive_boot.as_slice(),
                run_no.as_slice(),
                instrument_id.as_slice(),
                parameter_id.as_slice(),
                from.as_slice(),
                to.as_slice(),
                frozen.as_slice(),
                i64::from(last.is_some()),
                last_at.as_slice(),
                last_seq.as_slice(),
                upper_at.as_slice(),
                upper_seq.as_slice(),
                i64::try_from(limit + 1).expect("bounded lookahead")
            ],
            decode_measurement_row,
        )?;
        let mut rows = Vec::with_capacity(limit);
        let mut selected_rows = 0usize;
        let mut page_bytes = 0usize;
        let mut has_more = false;
        for selected_row in selected {
            let row = selected_row?;
            selected_rows += 1;
            // UTF-8 control bytes can become six JSON bytes (e.g. `\u0000`).
            // Leave room for the page envelope and cursor before app encoding.
            let escaped = |value: &str| value.len().saturating_mul(6);
            let bytes = 256usize
                .saturating_add(escaped(&row.unit))
                .saturating_add(row.failure.as_ref().map_or(0, |value| escaped(value)))
                .saturating_add(match &row.value {
                    Some(Value::Text(text) | Value::Enum(text)) => escaped(text),
                    _ => 0,
                })
                .saturating_add(usize::from(row.state_revision.is_some()) * 32)
                .saturating_add(usize::from(row.lineage.is_some()) * 512);
            if rows.len() == limit
                || page_bytes.saturating_add(bytes).saturating_add(loss_charge) > 8 * 1024 - 512
            {
                if rows.is_empty() {
                    return Err(StorageError("whole history row exceeds page budget".into()));
                }
                has_more = true;
                break;
            }
            page_bytes += bytes;
            rows.push(row);
        }
        let next_cursor = if has_more {
            let last_row = rows.last().expect("lookahead follows at least one row");
            Some(HistoryCursor {
                filter: filter.clone(),
                database_id: self.database_id.clone(),
                watermark,
                last: (
                    duration_blob(last_row.published_at).map(u64::from_be_bytes)?,
                    last_row.record_sequence,
                ),
                upper,
            })
        } else {
            None
        };
        Ok(HistoryPage {
            rows,
            selected_rows,
            watermark,
            next_cursor,
            coverage,
            loss,
        })
    }

    /// Read a bounded raw page for one signal using its indexed publication order.
    /// This worker-only primitive is not an application Query or an owner-lane call.
    pub fn read_measurements(
        &self,
        instrument: InstrumentId,
        parameter: ParameterId,
        limit: usize,
    ) -> Result<Vec<MeasurementRow>, StorageError> {
        if !(1..=MAX_RAW_PAGE).contains(&limit) {
            return Err(StorageError("history page limit must be 1..=128".into()));
        }
        let mut statement = self.connection.prepare(
            "SELECT record_seq,published_at,observed_at,unit_key,quality,failure,\
             value_kind,float_value,integer_value,bool_value,text_value,generation,revision,\
             state_revision,lineage \
             FROM measurements WHERE instrument_id=?1 AND parameter_id=?2 \
             ORDER BY published_at,record_seq LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                u64_blob(instrument.get()).as_slice(),
                u64_blob(parameter.get()).as_slice(),
                i64::try_from(limit).expect("bounded limit")
            ],
            decode_measurement_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn parse_history_loss(summary: &str) -> Result<HistoryLoss, StorageError> {
    // A 512-byte reason may require six JSON bytes per source byte.
    if summary.len() > 4096 {
        return Err(StorageError("archived loss summary exceeds bound".into()));
    }
    let document: serde_json::Value = serde_json::from_str(summary)
        .map_err(|_| StorageError("malformed archived loss summary".into()))?;
    let text = |key: &str| -> Result<&str, StorageError> {
        document
            .get(key)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| StorageError(format!("archived loss field {key} is invalid")))
    };
    let optional_id = |key: &str| -> Result<Option<u64>, StorageError> {
        match document.get(key) {
            Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(value)) => value
                .parse()
                .map(Some)
                .map_err(|_| StorageError(format!("archived loss field {key} is invalid"))),
            _ => Err(StorageError(format!(
                "archived loss field {key} is invalid"
            ))),
        }
    };
    let reason = text("reason")?;
    if reason.is_empty() || reason.len() > 512 {
        return Err(StorageError("archived loss reason is invalid".into()));
    }
    let required_id = |key: &str| -> Result<u64, StorageError> {
        text(key)?
            .parse()
            .map_err(|_| StorageError(format!("archived loss field {key} is invalid")))
    };
    Ok(HistoryLoss {
        reason: reason.to_owned(),
        first_missing_fact: optional_id("first_missing_fact")?,
        known_missing_count: optional_id("known_missing_count")?,
        last_accepted_fact: optional_id("last_accepted_fact")?,
        cutoff: Duration::from_nanos(required_id("cutoff_monotonic_ns")?),
        last_confirmed_record: required_id("last_confirmed_record_seq")?,
    })
}

// An invalid archive must be rejected before WAL conversion or new-boot
// recovery. Indexed LIMIT 2 proves the at-most-one invariant without scanning
// the history of all earlier sealed runs.
