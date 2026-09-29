use super::*;
use crate::{
    client::types::{
        ClientUpdate, ConnectionState, EventCursor, HelloState, KnownAdmission, MutationIdentity,
        QuarantinedRecoveryRecord, RecoveryQuarantineReason, RecoveryRecord, ReplyKind,
    },
    presentation::{
        AxisOptions, ConfigurationOwner, Plot, PresentationDocument, RuntimeRef, Trace, TraceStyle,
    },
};
use serde_json::json;

fn presentation() -> PresentationDocument {
    let mut document = PresentationDocument::empty("doc");
    document.plots.push(Plot {
        id: "plot".into(),
        title: "Plot".into(),
        time_window_seconds: 30.0,
        axes: AxisOptions::default(),
        traces: vec![Trace {
            id: "trace".into(),
            source: RuntimeRef::Signal {
                instrument: "1".into(),
                parameter: "1".into(),
            },
            display_label: "Signal".into(),
            visible: true,
            style: TraceStyle {
                color: "red".into(),
                width: 1.0,
            },
            display_unit: None,
        }],
    });
    document
}

fn hello(boot: &str) -> HelloState {
    HelloState {
        boot_id: boot.into(),
        scope: "scope".into(),
        next_seq: 1,
        operations: vec!["reference".into()],
        capabilities: json!([]),
        limits: json!({}),
        event_oldest: EventCursor {
            boot_id: boot.into(),
            seq: 1,
        },
        event_latest: EventCursor {
            boot_id: boot.into(),
            seq: 2,
        },
    }
}

fn result_reply(command_id: u64, op: &str, result: serde_json::Value) -> ClientUpdate {
    ClientUpdate::Reply {
        command_id,
        msg_id: command_id.to_string(),
        op: op.into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":result}),
        recovery: None,
    }
}

#[test]
fn unresolved_references_remain_in_valid_presentation() {
    let mut model = WorkbenchModel::new(presentation());
    model.refresh_unresolved();
    assert!(model.unresolved.contains(&RuntimeRef::Signal {
        instrument: "1".into(),
        parameter: "1".into(),
    }));
    assert!(model.presentation.validate().is_ok());
}

#[test]
fn only_a_fresh_current_observation_resolves_a_runtime_reference() {
    let mut model = WorkbenchModel::new(presentation());
    let signal = RuntimeRef::Signal {
        instrument: "1".into(),
        parameter: "1".into(),
    };

    model.apply_client_update(ClientUpdate::Hello(hello("boot-a")));
    model.apply_client_update(ClientUpdate::Reply {
        command_id: 1,
        msg_id: "1".into(),
        op: "measurements_current".into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":{"records":[{
            "signal":{"instrument":"1","parameter":"1"},"value":1.0
        }]}}),
        recovery: None,
    });
    assert_eq!(
        model.observations.entities[&signal].freshness,
        Freshness::Fresh
    );
    assert!(!model.unresolved.contains(&signal));

    model.apply_client_update(ClientUpdate::State(ConnectionState::Disconnected));
    assert_eq!(
        model.observations.entities[&signal].freshness,
        Freshness::Stale
    );
    assert!(model.unresolved.contains(&signal));

    model.apply_client_update(ClientUpdate::Hello(hello("boot-b")));
    model.complete_rebuild();
    assert_eq!(model.observations.freshness, Freshness::Fresh);
    assert_eq!(
        model.observations.entities[&signal].freshness,
        Freshness::Stale
    );
    assert!(model.unresolved.contains(&signal));

    model.apply_client_update(ClientUpdate::Reply {
        command_id: 2,
        msg_id: "2".into(),
        op: "measurements_current".into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":{"records":[{
            "signal":{"instrument":"1","parameter":"1"},"value":2.0
        }]}}),
        recovery: None,
    });
    assert_eq!(
        model.observations.entities[&signal].freshness,
        Freshness::Fresh
    );
    assert!(!model.unresolved.contains(&signal));
}

#[test]
fn live_display_window_evicts_only_oldest_display_points() {
    let mut model = WorkbenchModel::new(presentation());
    let source = RuntimeRef::Signal {
        instrument: "1".into(),
        parameter: "1".into(),
    };
    for value in 0..=LIVE_TRACE_POINTS {
        model
            .push_live_point(
                source.clone(),
                LivePoint {
                    time_seconds: value as f64,
                    value: value as f64,
                },
            )
            .unwrap();
    }
    let buffer = &model.observations.live[&source];
    assert_eq!(buffer.points().len(), LIVE_TRACE_POINTS);
    assert_eq!(buffer.points().front().unwrap().value, 1.0);
    assert_eq!(buffer.dropped(), 1);
}

#[test]
fn only_good_finite_signal_events_add_authoritative_time_plot_points() {
    let mut model = WorkbenchModel::new(presentation());
    let cursor = |seq| EventCursor {
        boot_id: "boot".into(),
        seq,
    };
    let signal = RuntimeRef::Signal {
        instrument: "1".into(),
        parameter: "1".into(),
    };
    let event = |seq, quality: &str, value: serde_json::Value, observed: serde_json::Value| {
        ClientUpdate::Event {
            cursor: cursor(seq),
            envelope: json!({
                "kind":"signal",
                "target":{"instrument":"1","parameter":"1"},
                "data":{"signal":{"instrument":"1","parameter":"1"},
                    "quality":quality,"value":value,"observed_at_ns":observed}
            }),
        }
    };

    model.apply_client_update(event(1, "good", json!(12.5), json!("2500000000")));
    model.apply_client_update(event(
        2,
        "unavailable",
        serde_json::Value::Null,
        serde_json::Value::Null,
    ));
    model.apply_client_update(event(
        3,
        "good",
        serde_json::Value::Null,
        json!("3000000000"),
    ));

    let points = model.observations.live[&signal].points();
    assert_eq!(points.len(), 1);
    assert_eq!(
        points[0],
        LivePoint {
            time_seconds: 2.5,
            value: 12.5
        }
    );
}

#[test]
fn lost_transport_resnapshot_is_not_ready_and_rebuild_starts_a_new_live_epoch() {
    let signal = RuntimeRef::Signal {
        instrument: "1".into(),
        parameter: "1".into(),
    };
    let mut model = WorkbenchModel::new(presentation());
    model.apply_client_update(ClientUpdate::Hello(hello("boot-a")));
    model
        .push_live_point(
            signal.clone(),
            LivePoint {
                time_seconds: 1.0,
                value: 2.0,
            },
        )
        .unwrap();

    model.apply_client_update(ClientUpdate::ResnapshotRequired {
        reason: "ordered_update_queue_full".into(),
        envelope: None,
        connection_lost: true,
    });
    assert_eq!(model.connection, ConnectionState::Stale);
    assert_eq!(model.observations.live[&signal].points().len(), 1);

    model.apply_client_update(ClientUpdate::State(ConnectionState::Reattaching));
    assert!(model.observations.live.is_empty());
    model
        .push_live_point(
            signal.clone(),
            LivePoint {
                time_seconds: 4.0,
                value: 5.0,
            },
        )
        .unwrap();
    assert_eq!(model.observations.live[&signal].points().len(), 1);
    assert_eq!(model.observations.live[&signal].dropped(), 0);
}

#[test]
fn disconnect_retains_cached_plot_but_gap_and_boot_change_start_new_live_epochs() {
    let signal = RuntimeRef::Signal {
        instrument: "1".into(),
        parameter: "1".into(),
    };
    let mut model = WorkbenchModel::new(presentation());
    model.apply_client_update(ClientUpdate::Hello(hello("boot-a")));
    for value in 0..=LIVE_TRACE_POINTS {
        model
            .push_live_point(
                signal.clone(),
                LivePoint {
                    time_seconds: value as f64,
                    value: value as f64,
                },
            )
            .unwrap();
    }
    model.apply_client_update(ClientUpdate::State(ConnectionState::Disconnected));
    assert_eq!(
        model.observations.live[&signal].points().len(),
        LIVE_TRACE_POINTS
    );
    assert_eq!(model.observations.live[&signal].dropped(), 1);

    model.apply_client_update(ClientUpdate::ResnapshotRequired {
        reason: "event_gap".into(),
        envelope: None,
        connection_lost: false,
    });
    assert!(model.observations.live.is_empty());
    model
        .push_live_point(
            signal.clone(),
            LivePoint {
                time_seconds: 3.0,
                value: 4.0,
            },
        )
        .unwrap();
    assert_eq!(model.observations.live[&signal].points().len(), 1);
    assert_eq!(model.observations.live[&signal].dropped(), 0);
    model.apply_client_update(ClientUpdate::Hello(hello("boot-b")));
    assert!(model.observations.live.is_empty());
}

#[test]
fn rebuild_completion_is_explicit_and_disconnect_stales_every_entity() {
    let mut model = WorkbenchModel::new(presentation());
    model.apply_client_update(ClientUpdate::Hello(hello("boot")));
    assert_eq!(model.observations.freshness, Freshness::Rebuilding);
    model.apply_client_update(ClientUpdate::ReferenceBootstrap {
        command_id: 1,
        snapshot: json!({"reference":"1","revision":"1","target":2.0}),
        subsequent_events: Vec::new(),
    });
    let reference = RuntimeRef::Reference {
        reference: "1".into(),
    };
    assert_eq!(
        model.observations.entities[&reference].freshness,
        Freshness::Fresh
    );
    assert_eq!(model.observations.freshness, Freshness::Rebuilding);

    model.apply_client_update(ClientUpdate::Reply {
        command_id: 2,
        msg_id: "2".into(),
        op: "resource".into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":{"resource":"resource-1"}}),
        recovery: None,
    });
    let resource = RuntimeRef::Resource {
        resource: "resource-1".into(),
    };
    assert_eq!(
        model.observations.entities[&resource].freshness,
        Freshness::Fresh
    );
    assert_eq!(model.observations.freshness, Freshness::Rebuilding);

    model.complete_rebuild();
    assert_eq!(model.observations.freshness, Freshness::Fresh);

    model.apply_client_update(ClientUpdate::State(ConnectionState::Disconnected));
    assert_eq!(model.observations.freshness, Freshness::Stale);
    assert!(
        model
            .observations
            .entities
            .values()
            .all(|observation| observation.freshness == Freshness::Stale)
    );
    model.apply_client_update(ClientUpdate::ResnapshotRequired {
        reason: "event_gap".into(),
        envelope: None,
        connection_lost: false,
    });
    assert_eq!(model.observations.freshness, Freshness::Rebuilding);

    model.apply_client_update(ClientUpdate::ReferenceBootstrap {
        command_id: 2,
        snapshot: json!({"reference":"1","revision":"2","target":3.0}),
        subsequent_events: Vec::new(),
    });
    assert_eq!(
        model.observations.entities[&reference].value["revision"],
        "2"
    );
    assert_eq!(
        model.observations.entities[&reference].freshness,
        Freshness::Fresh
    );
    assert_eq!(model.observations.freshness, Freshness::Rebuilding);
}

#[test]
fn reconciliation_identity_never_overwrites_command_identity() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    model.track_operator_intent(7).unwrap();
    model.apply_client_update(ClientUpdate::Reply {
        command_id: 7,
        msg_id: "7".into(),
        op: "reference_retune".into(),
        kind: ReplyKind::MutationAccepted,
        envelope: json!({"type":"accepted"}),
        recovery: None,
    });
    assert_eq!(model.action_state(7), Some(OperatorActionState::Accepted));

    let recovery = RecoveryRecord {
        boot_id: "boot".into(),
        identity: MutationIdentity {
            scope: "scope".into(),
            seq: 7,
        },
        op: "reference_retune".into(),
        args: json!({"reference":"1","target":2.0,"expected_revision":"1"}),
        admission: KnownAdmission::Ambiguous,
    };
    model.apply_client_update(ClientUpdate::RecoveryProjection {
        active: vec![recovery.clone()],
        quarantined: Vec::new(),
    });
    model.apply_client_update(ClientUpdate::ReconciliationRequired {
        records: vec![recovery.clone()],
    });

    assert_eq!(model.action_state(7), Some(OperatorActionState::Accepted));
    assert_eq!(model.actions.len(), 1);
    assert_eq!(
        model.recovery.reconciliation_required,
        vec![recovery.identity.clone()]
    );
    assert_eq!(model.recovery.mutations, vec![recovery]);
}

#[test]
fn quarantine_is_bounded_separate_evidence_and_never_reconciliation_authority() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    let record = RecoveryRecord {
        boot_id: "old-boot".into(),
        identity: MutationIdentity {
            scope: "old-scope".into(),
            seq: 7,
        },
        op: "reference_retune".into(),
        args: json!({"reference":"1","target":2.0,"expected_revision":"1"}),
        admission: KnownAdmission::Ambiguous,
    };
    model.apply_client_update(ClientUpdate::RecoveryProjection {
        active: vec![record.clone()],
        quarantined: Vec::new(),
    });
    model.apply_client_update(ClientUpdate::ReconciliationRequired {
        records: vec![record.clone()],
    });
    model.apply_client_update(ClientUpdate::RecoveryProjection {
        active: Vec::new(),
        quarantined: vec![QuarantinedRecoveryRecord {
            record: record.clone(),
            reason: RecoveryQuarantineReason::AttachedBootMismatch,
        }],
    });

    assert!(model.recovery.mutations.is_empty());
    assert!(model.recovery.reconciliation_required.is_empty());
    assert_eq!(model.recovery.quarantined.len(), 1);
    assert_eq!(model.recovery.quarantined[0].record, record);
    assert!(model.recovery_problem.is_none());

    model.apply_client_update(ClientUpdate::RecoveryJournalProblem {
        reason: "journal read failure".into(),
    });
    assert_eq!(model.recovery.quarantined.len(), 1);
    assert_eq!(
        model.recovery_problem.as_deref(),
        Some("journal read failure")
    );
}

#[test]
fn ordinary_query_results_update_projections_without_creating_operator_actions() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    let replies = [
        (
            "discover",
            json!({"records":[{"kind":"instrument","id":"1"}]}),
        ),
        (
            "measurements_current",
            json!({"records":[{"signal":{"instrument":"1","parameter":"temperature"},
                "quality":"good","value":20.0,"observed_at_ns":"1"}]}),
        ),
        (
            "reference",
            json!({"reference":"1","revision":"1","target":20.0}),
        ),
        (
            "controller",
            json!({"controller":"1","state":"ready","revision":"1","config":{}}),
        ),
        (
            "resource",
            json!({"resource":"1","binding_generation":"1",
                "transport_generation":"1","capabilities":{"reconnect":true}}),
        ),
        (
            "recording_status",
            json!({"state":"idle","active_run":null}),
        ),
        (
            "configuration_properties",
            json!({"records":[{"owner":{"kind":"instrument","id":"1"},
                "property":"sample_period","revision":"1","current":100}]}),
        ),
    ];

    for command_id in 0..256 {
        let (op, result) = &replies[command_id as usize % replies.len()];
        model.apply_client_update(result_reply(command_id, op, result.clone()));
    }

    assert!(model.actions.is_empty());
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Reference {
                reference: "1".into(),
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Controller {
                controller: "1".into(),
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Resource {
                resource: "1".into(),
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Recorder)
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Signal {
                instrument: "1".into(),
                parameter: "temperature".into(),
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::ConfigurationProperty {
                owner: ConfigurationOwner::Instrument {
                    instrument: "1".into(),
                },
                property: "sample_period".into(),
            })
    );
}

#[test]
fn bootstrap_and_untracked_rejection_never_create_operator_actions() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    model.apply_client_update(ClientUpdate::ReferenceBootstrap {
        command_id: 123,
        snapshot: json!({"reference":"1","revision":"1","target":2.0}),
        subsequent_events: Vec::new(),
    });
    assert!(model.actions.is_empty());
    assert_eq!(
        model.observations.entities[&RuntimeRef::Reference {
            reference: "1".into(),
        }]
            .value["revision"],
        "1"
    );

    model.apply_client_update(ClientUpdate::LocalRejected {
        command_id: 124,
        reason: "client_not_ready".into(),
    });
    assert_eq!(model.client_error.as_deref(), Some("client_not_ready"));
    assert!(model.actions.is_empty());
}

#[test]
fn only_tracked_operator_commands_advance_action_lifecycle() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    model.track_operator_intent(7).unwrap();
    assert_eq!(
        model.action_state(7),
        Some(OperatorActionState::PendingAdmission)
    );

    model.apply_client_update(ClientUpdate::Reply {
        command_id: 7,
        msg_id: "7".into(),
        op: "reference_retune".into(),
        kind: ReplyKind::MutationAccepted,
        envelope: json!({"type":"accepted"}),
        recovery: None,
    });
    assert_eq!(model.action_state(7), Some(OperatorActionState::Accepted));
    model.apply_client_update(ClientUpdate::Reply {
        command_id: 7,
        msg_id: "7".into(),
        op: "reference_retune".into(),
        kind: ReplyKind::MutationCompleted,
        envelope: json!({"type":"completed"}),
        recovery: None,
    });
    assert_eq!(model.action_state(7), Some(OperatorActionState::Completed));

    model.track_operator_intent(8).unwrap();
    model.apply_client_update(result_reply(
        8,
        "reference",
        json!({"reference":"1","revision":"2"}),
    ));
    assert_eq!(
        model.action_state(8),
        Some(OperatorActionState::PendingAdmission),
        "ordinary Result cannot alter even an explicitly tracked action"
    );
    model.apply_client_update(ClientUpdate::LocalRejected {
        command_id: 8,
        reason: "queue_full".into(),
    });
    assert_eq!(model.action_state(8), Some(OperatorActionState::Failed));

    model.apply_client_update(ClientUpdate::Reply {
        command_id: 999,
        msg_id: "999".into(),
        op: "reference_retune".into(),
        kind: ReplyKind::MutationAccepted,
        envelope: json!({"type":"accepted"}),
        recovery: None,
    });
    assert_eq!(model.actions.len(), 2);
}

#[test]
fn operator_action_history_is_absolutely_bounded_and_never_evicts_active_work() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    for command_id in 0..(MAX_OPERATOR_ACTIONS as u64 + 128) {
        model.track_operator_intent(command_id).unwrap();
        model.apply_client_update(ClientUpdate::Reply {
            command_id,
            msg_id: command_id.to_string(),
            op: "reference_retune".into(),
            kind: ReplyKind::MutationAccepted,
            envelope: json!({"type":"accepted"}),
            recovery: None,
        });
        model.apply_client_update(ClientUpdate::Reply {
            command_id,
            msg_id: command_id.to_string(),
            op: "reference_retune".into(),
            kind: if command_id % 2 == 0 {
                ReplyKind::MutationCompleted
            } else {
                ReplyKind::MutationFailed
            },
            envelope: json!({"type":"terminal"}),
            recovery: None,
        });
        assert!(model.actions.len() <= MAX_OPERATOR_ACTIONS);
    }
    let newest = MAX_OPERATOR_ACTIONS as u64 + 127;
    assert!(model.action_state(0).is_none());
    assert!(model.action_state(newest).is_some());
    assert_eq!(model.actions.len(), MAX_OPERATOR_ACTIONS);

    let mut active = WorkbenchModel::new(PresentationDocument::empty("doc"));
    for command_id in 0..MAX_OPERATOR_ACTIONS as u64 {
        active.track_operator_intent(command_id).unwrap();
    }
    assert!(!active.operator_action_capacity_available());
    assert_eq!(
        active.track_operator_intent(MAX_OPERATOR_ACTIONS as u64),
        Err(OPERATOR_ACTION_CAPACITY_ERROR)
    );
    assert_eq!(active.actions.len(), MAX_OPERATOR_ACTIONS);
    assert_eq!(
        active.action_state(0),
        Some(OperatorActionState::PendingAdmission)
    );
    assert!(active.action_state(MAX_OPERATOR_ACTIONS as u64).is_none());
}

#[test]
fn repeated_rebuild_results_do_not_grow_operator_actions() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    for cycle in 0..32 {
        model.apply_client_update(ClientUpdate::Hello(hello(&format!("boot-{cycle}"))));
        model.apply_client_update(result_reply(
            cycle * 3,
            "discover",
            json!({"records":[{"kind":"reference","id":"1"}]}),
        ));
        model.apply_client_update(result_reply(
            cycle * 3 + 1,
            "reference",
            json!({"reference":"1","revision":cycle.to_string()}),
        ));
        model.apply_client_update(result_reply(
            cycle * 3 + 2,
            "recording_status",
            json!({"state":"idle","active_run":null}),
        ));
        model.apply_client_update(ClientUpdate::ResnapshotRequired {
            reason: "event_gap".into(),
            envelope: None,
            connection_lost: false,
        });
        assert!(model.actions.is_empty());
    }
}

#[test]
fn boot_change_never_leaves_old_observations_fresh() {
    let mut model = WorkbenchModel::new(presentation());
    model.apply_client_update(ClientUpdate::Hello(hello("boot-a")));
    model.apply_client_update(ClientUpdate::ReferenceBootstrap {
        command_id: 1,
        snapshot: json!({"reference":"1","revision":"1"}),
        subsequent_events: Vec::new(),
    });
    model.apply_client_update(ClientUpdate::Hello(hello("boot-b")));
    assert_eq!(model.observations.freshness, Freshness::Rebuilding);
    assert!(
        model
            .observations
            .entities
            .values()
            .all(|observation| observation.freshness == Freshness::Stale)
    );
}

#[test]
fn ui_commands_validate_transactionally_and_share_one_boundary() {
    let mut document = presentation();
    apply_ui_command(
        &mut document,
        UiCommand::SetTraceVisibility {
            plot_id: "plot".into(),
            trace_id: "trace".into(),
            visible: false,
        },
    )
    .unwrap();
    assert!(!document.plots[0].traces[0].visible);

    let before = document.clone();
    assert!(
        apply_ui_command(
            &mut document,
            UiCommand::SetTimeWindow {
                plot_id: "plot".into(),
                seconds: f64::INFINITY,
            }
        )
        .is_err()
    );
    assert_eq!(document, before);

    let lab = WorkbenchCommand::Lab(LabCommand::Mutation {
        op: "reference_retune".into(),
        args: json!({}),
    });
    let ui = WorkbenchCommand::Ui(UiCommand::RenamePresentationItem {
        item_id: "plot".into(),
        label: "Renamed".into(),
    });
    assert!(matches!(lab, WorkbenchCommand::Lab(_)));
    assert!(matches!(ui, WorkbenchCommand::Ui(_)));
}

#[test]
fn initial_gui_projection_kinds_normalize_in_one_model_owner() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    let replies = [
        (
            "discover",
            json!({"records":[
                {"kind":"instrument","id":"1"},
                {"kind":"signal","id":{"instrument":"1","parameter":"1"}},
                {"kind":"controller","id":{"id":"2"}},
                {"kind":"resource","id":"3"}
            ]}),
        ),
        (
            "measurements_current",
            json!({"records":[{"signal":{"instrument":"1","parameter":"1"},
                "value":12.0,"quality":"good"}]}),
        ),
        ("recording_status", json!({"state":"recording"})),
    ];
    for (index, (op, result)) in replies.into_iter().enumerate() {
        model.apply_client_update(ClientUpdate::Reply {
            command_id: index as u64 + 1,
            msg_id: (index + 1).to_string(),
            op: op.into(),
            kind: ReplyKind::Result,
            envelope: json!({"type":"result","result":result}),
            recovery: None,
        });
    }
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Instrument {
                instrument: "1".into()
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Signal {
                instrument: "1".into(),
                parameter: "1".into()
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Controller {
                controller: "2".into()
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Resource {
                resource: "3".into()
            })
    );
    assert!(
        model
            .observations
            .entities
            .contains_key(&RuntimeRef::Recorder)
    );
}

#[test]
fn recorder_event_refreshes_recorder_while_configuration_event_stales_only_properties() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    let property = RuntimeRef::ConfigurationProperty {
        owner: ConfigurationOwner::Instrument {
            instrument: "1".into(),
        },
        property: "poll_period_ms".into(),
    };
    model.observations.observe(
        property.clone(),
        json!({"owner":{"kind":"instrument","id":"1"},"property":"poll_period_ms",
            "current":100,"value_type":"integer","access":"read_write","revision":"1"}),
        None,
    );
    model.observations.complete_rebuild();
    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 4,
        },
        envelope: json!({"kind":"recorder","target":{"id":"archive"},
            "data":{"state":"recording","active_run":{"boot_id":"boot","run_no":"1"}}}),
    });
    assert_eq!(
        model.observations.entities[&RuntimeRef::Recorder].value["state"],
        "recording"
    );
    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 5,
        },
        envelope: json!({"kind":"configuration","target":{"id":"runtime"},
            "data":{"revision":"2"}}),
    });
    assert_eq!(
        model.observations.entities[&property].freshness,
        Freshness::Stale
    );
    assert_eq!(
        model.observations.entities[&RuntimeRef::Recorder].freshness,
        Freshness::Fresh
    );
}

#[test]
fn controller_events_preserve_same_revision_detail_and_stale_changed_detail() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    let controller = RuntimeRef::Controller {
        controller: "2".into(),
    };
    model.apply_client_update(ClientUpdate::Reply {
        command_id: 1,
        msg_id: "1".into(),
        op: "controller".into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":{"controller":"2","kind":"pid",
            "state":"ready","status":"valid","failure":null,"active":false,"paused":false,
            "revision":"7","last_tick":null,"latest_output":null,
            "bindings":{"input":"1"},"config":{"pid":{"kp":1.0}}}}),
        recovery: None,
    });
    assert!(model.controller_detail_is_fresh("2"));

    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 2,
        },
        envelope: json!({"kind":"controller","target":{"id":"2"},
            "data":{"state":"running","status":"valid","failure":null,"active":true,
                "paused":false,"revision":"7","last_tick":"4","latest_output":2.0}}),
    });
    let running = &model.observations.entities[&controller].value;
    assert_eq!(running["state"], "running");
    assert_eq!(running["status"], "valid");
    assert_eq!(running["failure"], Value::Null);
    assert_eq!(running["active"], true);
    assert_eq!(running["paused"], false);
    assert_eq!(running["last_tick"], "4");
    assert_eq!(running["latest_output"], 2.0);
    assert_eq!(running["config"]["pid"]["kp"], 1.0);
    assert_eq!(running["bindings"]["input"], "1");
    assert!(model.controller_detail_is_fresh("2"));

    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 3,
        },
        envelope: json!({"kind":"controller","target":{"id":"2"},
            "data":{"state":"paused","status":"valid","failure":null,"active":false,
                "paused":true,"revision":"7","last_tick":"5","latest_output":0.0}}),
    });
    let paused = &model.observations.entities[&controller].value;
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["status"], "valid");
    assert_eq!(paused["failure"], Value::Null);
    assert_eq!(paused["active"], false);
    assert_eq!(paused["paused"], true);
    assert!(model.controller_detail_is_fresh("2"));

    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 4,
        },
        envelope: json!({"kind":"controller","target":{"id":"2"},
            "data":{"state":"failed","status":"failed","failure":"latched","active":false,
                "paused":false,"revision":"7","last_tick":"6","latest_output":null}}),
    });
    let failed = &model.observations.entities[&controller].value;
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["failure"], "latched");
    assert_eq!(failed["active"], false);
    assert_eq!(failed["paused"], false);
    assert!(model.controller_detail_is_fresh("2"));

    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 5,
        },
        envelope: json!({"kind":"controller","target":{"id":"2"},
            "data":{"state":"paused","status":"valid","failure":null,"active":false,
                "paused":true,"revision":"8","last_tick":"7","latest_output":0.0}}),
    });
    assert_eq!(
        model.observations.entities[&controller].freshness,
        Freshness::Fresh
    );
    assert_eq!(
        model.observations.entities[&controller].value["config"]["pid"]["kp"],
        1.0
    );
    assert!(!model.controller_detail_is_fresh("2"));

    model.apply_client_update(ClientUpdate::Reply {
        command_id: 2,
        msg_id: "2".into(),
        op: "controller".into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":{"controller":"2","state":"paused",
            "revision":"8","bindings":{"input":"1"},"config":{"pid":{"kp":2.0}}}}),
        recovery: None,
    });
    assert!(model.controller_detail_is_fresh("2"));
    assert_eq!(
        model.observations.entities[&controller].value["config"]["pid"]["kp"],
        2.0
    );
}

#[test]
fn resource_events_preserve_full_reconnect_detail_and_stale_changed_generation() {
    let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
    let resource = RuntimeRef::Resource {
        resource: "3".into(),
    };
    model.apply_client_update(ClientUpdate::Reply {
        command_id: 1,
        msg_id: "1".into(),
        op: "resource".into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":{"resource":"3","state":"idle",
            "binding_generation":"4","transport_generation":"9",
            "capabilities":{"reconnect":true,"configuration":true},
            "configuration_revision":"2","deployment":{"port":"COM5"}}}),
        recovery: None,
    });
    assert!(model.resource_detail_is_fresh("3"));

    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 2,
        },
        envelope: json!({"kind":"resource","target":{"id":"3"},"data":{
            "state":"in_flight","queue_len":1,"generation":"9","active":"12",
            "latest":{"id":"11","outcome":"completed","started":true,"generation":"9"}}}),
    });
    let same = &model.observations.entities[&resource].value;
    assert_eq!(same["state"], "in_flight");
    assert_eq!(same["queue_len"], 1);
    assert_eq!(same["binding_generation"], "4");
    assert_eq!(same["transport_generation"], "9");
    assert_eq!(same["capabilities"]["reconnect"], true);
    assert_eq!(same["deployment"]["port"], "COM5");
    assert!(model.resource_detail_is_fresh("3"));

    model.apply_client_update(ClientUpdate::Event {
        cursor: EventCursor {
            boot_id: "boot".into(),
            seq: 3,
        },
        envelope: json!({"kind":"resource","target":{"id":"3"},"data":{
            "state":"recovering","queue_len":0,"generation":"10","active":null,
            "latest":null}}),
    });
    let changed = &model.observations.entities[&resource].value;
    assert_eq!(changed["transport_generation"], "10");
    assert_eq!(changed["binding_generation"], "4");
    assert_eq!(changed["capabilities"]["reconnect"], true);
    assert_eq!(
        model.observations.entities[&resource].freshness,
        Freshness::Fresh
    );
    assert!(!model.resource_detail_is_fresh("3"));

    model.apply_client_update(ClientUpdate::Reply {
        command_id: 2,
        msg_id: "2".into(),
        op: "resource".into(),
        kind: ReplyKind::Result,
        envelope: json!({"type":"result","result":{"resource":"3","state":"idle",
            "binding_generation":"5","transport_generation":"10",
            "capabilities":{"reconnect":true,"configuration":true},
            "configuration_revision":"3","deployment":{"port":"COM5"}}}),
        recovery: None,
    });
    assert!(model.resource_detail_is_fresh("3"));
    assert_eq!(
        model.observations.entities[&resource].value["binding_generation"],
        "5"
    );
}
