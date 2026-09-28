use super::*;
use crate::storage::sibling_files;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static TEST_ID: AtomicU64 = AtomicU64::new(1);

fn test_dir(name: &str) -> PathBuf {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "lab-workbench-m14-3-{name}-{}-{id}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn trace(id: &str) -> Trace {
    Trace {
        id: id.into(),
        source: RuntimeRef::Signal {
            instrument: "1".into(),
            parameter: "1".into(),
        },
        display_label: "temperature".into(),
        visible: true,
        style: TraceStyle {
            color: "#ff0000".into(),
            width: 1.0,
        },
        display_unit: Some("K".into()),
    }
}

fn document() -> PresentationDocument {
    PresentationDocument {
        format_version: 1,
        document_id: "doc".into(),
        windows: vec![PresentationWindow {
            id: "window".into(),
            title: "Workbench".into(),
            tabs: vec![PresentationTab {
                id: "tab".into(),
                title: "Live".into(),
                panels: vec![Panel {
                    id: "panel".into(),
                    title: "Temperature".into(),
                    kind: PanelKind::Plot {
                        plot_id: "plot".into(),
                    },
                }],
            }],
        }],
        plots: vec![Plot {
            id: "plot".into(),
            title: "Temperature".into(),
            time_window_seconds: 30.0,
            axes: AxisOptions::default(),
            traces: vec![trace("trace")],
        }],
        controls: vec![Control {
            id: "control".into(),
            label: "Set reference".into(),
            target: RuntimeRef::Reference {
                reference: "1".into(),
            },
            kind: ControlKind::SetReference,
            confirm: true,
        }],
    }
}

#[test]
fn presentation_v1_round_trip_and_atomic_replacement() {
    let directory = test_dir("round-trip");
    let path = directory.join("presentation.json");
    let first = document();
    save_presentation(&path, &first).unwrap();
    assert_eq!(load_presentation(&path).unwrap(), first);

    let mut second = first.clone();
    second.plots[0].title = "Retuned".into();
    save_presentation(&path, &second).unwrap();
    assert_eq!(load_presentation(&path).unwrap(), second);
    assert!(sibling_files(&path).is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn validation_freezes_cardinality_string_id_and_number_bounds() {
    let mut candidate = document();
    candidate.windows = (0..=super::document::MAX_WINDOWS)
        .map(|index| PresentationWindow {
            id: format!("w{index}"),
            title: "w".into(),
            tabs: Vec::new(),
        })
        .collect();
    assert_eq!(candidate.validate(), Err(DocumentError::Limit("windows")));

    let mut candidate = document();
    candidate.windows[0].tabs = (0..=super::document::MAX_TABS_PER_WINDOW)
        .map(|index| PresentationTab {
            id: format!("tab{index}"),
            title: "t".into(),
            panels: Vec::new(),
        })
        .collect();
    assert_eq!(
        candidate.validate(),
        Err(DocumentError::Limit("tabs_per_window"))
    );

    let mut candidate = document();
    candidate.windows[0].tabs[0].panels = (0..=super::document::MAX_PANELS)
        .map(|index| Panel {
            id: format!("p{index}"),
            title: "p".into(),
            kind: PanelKind::Status {
                source: RuntimeRef::Recorder,
            },
        })
        .collect();
    assert_eq!(candidate.validate(), Err(DocumentError::Limit("panels")));

    let mut candidate = document();
    candidate.plots = (0..=super::document::MAX_PLOTS)
        .map(|index| Plot {
            id: format!("plot{index}"),
            title: "p".into(),
            time_window_seconds: 1.0,
            axes: AxisOptions::default(),
            traces: Vec::new(),
        })
        .collect();
    assert_eq!(candidate.validate(), Err(DocumentError::Limit("plots")));

    let mut candidate = document();
    candidate.controls = (0..=super::document::MAX_CONTROLS)
        .map(|index| Control {
            id: format!("control{index}"),
            label: "c".into(),
            target: RuntimeRef::Recorder,
            kind: ControlKind::RecordingLifecycle,
            confirm: false,
        })
        .collect();
    assert_eq!(candidate.validate(), Err(DocumentError::Limit("controls")));

    let mut candidate = document();
    candidate.plots[0].traces = (0..=super::document::MAX_TRACES_PER_PLOT)
        .map(|index| trace(&format!("trace{index}")))
        .collect();
    assert_eq!(
        candidate.validate(),
        Err(DocumentError::Limit("traces_per_plot"))
    );

    let mut candidate = document();
    candidate.plots[0].title = "x".repeat(super::document::MAX_STRING_BYTES + 1);
    assert_eq!(candidate.validate(), Err(DocumentError::InvalidString));

    let mut candidate = document();
    candidate.plots[0].time_window_seconds = f64::NAN;
    assert_eq!(
        candidate.validate(),
        Err(DocumentError::InvalidNumber("time_window_seconds"))
    );

    let mut candidate = document();
    candidate.plots[0].traces[0].id = "plot".into();
    assert_eq!(
        candidate.validate(),
        Err(DocumentError::DuplicateId("plot".into()))
    );
}

#[test]
fn invalid_files_and_future_versions_do_not_replace_active_document() {
    let directory = test_dir("invalid-load");
    let path = directory.join("presentation.json");
    let original = document();
    let mut active = original.clone();

    for bytes in [
        Vec::new(),
        b"{".to_vec(),
        vec![0xff, 0xfe],
        b"[]".to_vec(),
        br#"{"format_version":2,"document_id":"x","windows":[],"plots":[],"controls":[]}"#
            .to_vec(),
        br#"{"format_version":1,"document_id":"x","windows":[],"plots":[],"controls":[],"unknown":true}"#
            .to_vec(),
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(replace_presentation(&mut active, &path).is_err());
        assert_eq!(active, original);
    }

    let mut invalid_tag = serde_json::to_value(&original).unwrap();
    invalid_tag["windows"][0]["tabs"][0]["panels"][0]["kind"]["kind"] =
        serde_json::Value::String("widget".into());
    fs::write(&path, serde_json::to_vec(&invalid_tag).unwrap()).unwrap();
    assert!(replace_presentation(&mut active, &path).is_err());
    assert_eq!(active, original);

    fs::write(
        &path,
        vec![b' '; super::document::PRESENTATION_FILE_BYTES + 1],
    )
    .unwrap();
    assert!(replace_presentation(&mut active, &path).is_err());
    assert_eq!(active, original);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_save_preserves_prior_valid_file() {
    let directory = test_dir("failed-save");
    let path = directory.join("presentation.json");
    let original = document();
    save_presentation(&path, &original).unwrap();
    let before = fs::read(&path).unwrap();

    let mut invalid = original.clone();
    invalid.format_version = 2;
    assert!(save_presentation(&path, &invalid).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(load_presentation(&path).unwrap(), original);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn serialized_file_bound_is_checked_before_replacement() {
    let directory = test_dir("serialized-bound");
    let path = directory.join("presentation.json");
    let original = document();
    save_presentation(&path, &original).unwrap();
    let before = fs::read(&path).unwrap();

    let long = "x".repeat(super::document::MAX_STRING_BYTES);
    let mut candidate = PresentationDocument::empty("large");
    candidate.plots = (0..super::document::MAX_PLOTS)
        .map(|plot_index| Plot {
            id: format!("plot{plot_index}"),
            title: long.clone(),
            time_window_seconds: 1.0,
            axes: AxisOptions::default(),
            traces: (0..super::document::MAX_TRACES_PER_PLOT)
                .map(|trace_index| Trace {
                    id: format!("trace{plot_index}-{trace_index}"),
                    source: RuntimeRef::Signal {
                        instrument: long.clone(),
                        parameter: long.clone(),
                    },
                    display_label: long.clone(),
                    visible: true,
                    style: TraceStyle {
                        color: long.clone(),
                        width: 1.0,
                    },
                    display_unit: Some(long.clone()),
                })
                .collect(),
        })
        .collect();
    assert!(candidate.validate().is_ok());
    assert!(matches!(
        save_presentation(&path, &candidate),
        Err(PresentationLoadError::Validation(DocumentError::Limit(
            "file_bytes"
        )))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir_all(directory).unwrap();
}
