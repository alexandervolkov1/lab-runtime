use super::*;
use crate::{
    client::types::{
        ClientUpdate, ConnectionState, EventCursor, HelloState, KnownAdmission, MutationIdentity,
        RecoveryRecord, ReplyKind,
    },
    presentation::{AxisOptions, Plot, PresentationDocument, RuntimeRef, Trace, TraceStyle},
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
    model.track_operator_intent(7);
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
    model.apply_client_update(ClientUpdate::RecoveryState {
        records: vec![recovery.clone()],
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
