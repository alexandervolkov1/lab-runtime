//! Stable, typed references from presentation state to Runtime entities.

use serde::{Deserialize, Serialize};

/// One stable Runtime identity used by presentation objects.
///
/// These values select observations obtained through the Application API. They do
/// not assert that the selected entity currently exists or that any cached value is
/// authoritative.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RuntimeRef {
    /// Instrument descriptor/state by Application instrument ID.
    Instrument { instrument: String },
    /// Signal by its compound instrument/parameter identity.
    Signal {
        instrument: String,
        parameter: String,
    },
    /// Reference projection by Application reference ID.
    Reference { reference: String },
    /// Controller projection by Application controller ID.
    Controller { controller: String },
    /// Resource projection by Application resource ID.
    Resource { resource: String },
    /// Managed-component projection by Application component ID.
    Component { component: String },
    /// The process-owned Recorder projection.
    Recorder,
    /// One exposed configuration property identified by owner and property key.
    ConfigurationProperty {
        owner: ConfigurationOwner,
        property: String,
    },
}

/// Non-recursive owner identity for an exposed configuration property.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ConfigurationOwner {
    /// Instrument-owned property.
    Instrument { instrument: String },
    /// Managed-component-owned property.
    Component { component: String },
    /// Resource-owned property.
    Resource { resource: String },
}

impl RuntimeRef {
    /// Visits every string carried by this identity, including nested owners.
    pub(crate) fn strings(&self, visit: &mut impl FnMut(&str)) {
        match self {
            Self::Instrument { instrument } => visit(instrument),
            Self::Signal {
                instrument,
                parameter,
            } => {
                visit(instrument);
                visit(parameter);
            }
            Self::Reference { reference } => visit(reference),
            Self::Controller { controller } => visit(controller),
            Self::Resource { resource } => visit(resource),
            Self::Component { component } => visit(component),
            Self::Recorder => {}
            Self::ConfigurationProperty { owner, property } => {
                match owner {
                    ConfigurationOwner::Instrument { instrument } => visit(instrument),
                    ConfigurationOwner::Component { component } => visit(component),
                    ConfigurationOwner::Resource { resource } => visit(resource),
                }
                visit(property);
            }
        }
    }
}
