//! Connection-local frozen projections, subscriptions, and event delivery.
//!
//! This module owns no event facts or experiment state. [`crate::events::EventLog`]
//! remains the bounded replay owner; `Application` retains only per-connection
//! cursors, filters, page tokens, and expiry.

use super::{Application, FilterTarget, FrozenProjection, Subscription, common::id_field};
use crate::{
    host::Clock,
    protocol::{self, PublicError},
    service::ServiceHost,
    wire::decimal_u64,
};
use serde_json::{Value, json};
use std::time::Duration;

impl Application {
    pub(super) fn delivery_query(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        operation: &str,
        args: &Value,
    ) -> Option<Result<Value, &'static str>> {
        let result = match operation {
            "discovery_page" | "measurements_page" | "configuration_page" => {
                let token = match args.get("projection").and_then(Value::as_str) {
                    Some(token) => token,
                    None => return Some(Err("invalid_args")),
                };
                let index = match id_field(args, "index") {
                    Ok(index) => index as usize,
                    Err(error) => return Some(Err(error)),
                };
                let expected = match operation {
                    "discovery_page" => "discovery",
                    "measurements_page" => "measurements",
                    _ => "configuration",
                };
                self.projection_page(service, connection, token, index, expected)
            }
            "subscribe" => self.subscribe(service, connection, args),
            "unsubscribe" => self.unsubscribe(connection, args),
            _ => return None,
        };
        Some(result)
    }

    pub(super) fn begin_projection(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        kind: &'static str,
        records: Vec<Value>,
    ) -> Result<Value, &'static str> {
        let token = self.issue_token(service.boot_id())?;
        self.projections.insert(
            connection,
            FrozenProjection {
                token: token.clone(),
                kind,
                cursor: service.owner().event_log().latest_cursor(),
                records,
                expires: service.clock().now() + Duration::from_secs(5),
            },
        );
        self.projection_page(service, connection, &token, 0, kind)
    }

    fn issue_token(&mut self, boot: &str) -> Result<String, &'static str> {
        let counter = self.next_token;
        self.next_token = self.next_token.checked_add(1).ok_or("counter_exhausted")?;
        Ok(format!("{boot}:{counter}"))
    }

    fn projection_page(
        &self,
        service: &ServiceHost,
        connection: u64,
        token: &str,
        index: usize,
        kind: &str,
    ) -> Result<Value, &'static str> {
        let projection = self
            .projections
            .get(&connection)
            .ok_or("snapshot_expired")?;
        if projection.token != token
            || projection.kind != kind
            || service.clock().now() >= projection.expires
            || index > projection.records.len()
        {
            return Err("snapshot_expired");
        }
        let mut records = Vec::new();
        let mut next = index;
        while next < projection.records.len() && records.len() < 64 {
            let candidate = &projection.records[next];
            let mut trial = records.clone();
            trial.push(candidate.clone());
            if serde_json::to_vec(&trial).map_or(true, |bytes| bytes.len() > 8 * 1024) {
                break;
            }
            records.push(candidate.clone());
            next += 1;
        }
        if next == index && next < projection.records.len() {
            return Err("snapshot_capacity");
        }
        Ok(
            json!({"projection":token,"revision":{"boot_id":service.boot_id(),
            "event_seq":projection.cursor.to_string()},"records":records,
            "next_index":(next<projection.records.len()).then(||next.to_string()),"complete":next==projection.records.len()}),
        )
    }

    fn subscribe(
        &mut self,
        service: &ServiceHost,
        connection: u64,
        args: &Value,
    ) -> Result<Value, &'static str> {
        if self.subscriptions.contains_key(&connection) {
            return Err("subscription_busy");
        }
        let after = args.get("after").ok_or("invalid_args")?;
        let boot = after
            .get("boot_id")
            .and_then(Value::as_str)
            .ok_or("invalid_args")?;
        if boot != service.boot_id() {
            return Err("instance_changed");
        }
        let seq = id_field(after, "seq")?;
        match service.owner().event_log().scan_after(seq, 1) {
            Ok(_) => {}
            Err(crate::events::EventError::Gap) => return Err("event_gap"),
            Err(crate::events::EventError::Future) => return Err("invalid_cursor"),
            Err(_) => return Err("event_error"),
        }
        let filter = args.get("filter").ok_or("invalid_args")?;
        let kinds = filter
            .get("kinds")
            .and_then(Value::as_array)
            .ok_or("invalid_args")?;
        let targets = filter
            .get("targets")
            .and_then(Value::as_array)
            .ok_or("invalid_args")?;
        if kinds.len() > 8 || targets.len() > 16 {
            return Err("invalid_args");
        }
        let mut selected = Vec::new();
        for kind in kinds {
            let name = kind.as_str().ok_or("invalid_args")?;
            if ![
                "signal",
                "controller",
                "reference",
                "component",
                "output",
                "operation",
                "host",
                "recorder",
                "resource",
                "configuration",
            ]
            .contains(&name)
            {
                return Err("invalid_args");
            }
            selected.push(name.to_string());
        }
        let selected_targets = targets
            .iter()
            .map(parse_filter_target)
            .collect::<Result<Vec<_>, _>>()?;
        let token = self.issue_token(service.boot_id())?;
        self.subscriptions.insert(
            connection,
            Subscription {
                token: token.clone(),
                scan: seq,
                kinds: selected,
                targets: selected_targets,
            },
        );
        Ok(
            json!({"subscription":token,"accepted_cursor":{"boot_id":service.boot_id(),"seq":seq.to_string()}}),
        )
    }

    fn unsubscribe(&mut self, connection: u64, args: &Value) -> Result<Value, &'static str> {
        let token = args
            .get("subscription")
            .and_then(Value::as_str)
            .ok_or("invalid_args")?;
        let removed = self
            .subscriptions
            .get(&connection)
            .is_some_and(|subscription| subscription.token == token);
        if removed {
            self.subscriptions.remove(&connection);
        }
        Ok(json!({"removed":removed}))
    }

    /// Offer at most four matching retained semantic events after scanning 32.
    /// A filtered scan emits progress so a client can advance its applied cursor.
    pub fn pump_events(&mut self, service: &ServiceHost, connection: u64) -> Vec<Value> {
        let Some(subscription) = self.subscriptions.get_mut(&connection) else {
            return Vec::new();
        };
        let initial_scan = subscription.scan;
        let events = match service
            .owner()
            .event_log()
            .scan_after(subscription.scan, 32)
        {
            Ok(events) => events,
            Err(_) => {
                self.subscriptions.remove(&connection);
                let mut response = json!({"v":protocol::PROTOCOL_VERSION,
                    "type":"error","accepted":false});
                PublicError::from_code("event_gap").apply_to(&mut response);
                return vec![response];
            }
        };
        let mut offered = Vec::new();
        let mut scanned = subscription.scan;
        for event in events {
            let seq = event["seq"]
                .as_str()
                .and_then(decimal_u64)
                .unwrap_or(scanned);
            scanned = seq;
            if (subscription.kinds.is_empty()
                || event["kind"]
                    .as_str()
                    .is_some_and(|kind| subscription.kinds.iter().any(|selected| selected == kind)))
                && (subscription.targets.is_empty()
                    || subscription.targets.iter().any(|target| {
                        event["kind"] == target.kind && event["target"] == target.target
                    }))
            {
                offered.push(event);
                if offered.len() >= 4 {
                    break;
                }
            }
        }
        subscription.scan = scanned;
        if offered.is_empty() && scanned > initial_scan {
            offered.push(json!({"v":protocol::PROTOCOL_VERSION,"type":"subscription_progress","subscription":subscription.token,
                "boot_id":service.boot_id(),"seq":scanned.to_string()}));
        }
        offered
    }
}

fn parse_filter_target(value: &Value) -> Result<FilterTarget, &'static str> {
    let object = value.as_object().ok_or("invalid_args")?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("invalid_args")?;
    let target = match kind {
        "instrument" | "controller" | "reference" | "component" | "resource" => {
            if object.len() != 2 || !object.contains_key("id") {
                return Err("invalid_args");
            }
            json!({"id":id_field(value,"id")?.to_string()})
        }
        "configuration" => {
            if object.len() != 2 || value.get("id").and_then(Value::as_str) != Some("runtime") {
                return Err("invalid_args");
            }
            json!({"id":"runtime"})
        }
        "signal" | "output" => {
            if object.len() != 3
                || !object.contains_key("instrument")
                || !object.contains_key("parameter")
            {
                return Err("invalid_args");
            }
            json!({"instrument":id_field(value,"instrument")?.to_string(),"parameter":id_field(value,"parameter")?.to_string()})
        }
        _ => return Err("invalid_args"),
    };
    Ok(FilterTarget {
        kind: kind.into(),
        target,
    })
}
