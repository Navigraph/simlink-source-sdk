//! The generated protocol types against the manifest they come from: every schema-valid sample must deserialize,
//! and serialize back to the same, still valid, JSON.

mod support;

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use simlink_source_sdk::*;
use support::manifest::{branches, channels, issues, sample, schema};

#[test]
fn generated_types_round_trip_through_the_manifest() {
    let mut checked = BTreeSet::new();
    let mut check = |part: &str, round_trip: fn(&Value) -> Value| {
        check_schema(part, &schema(part), round_trip);
        checked.insert(part.to_owned());
    };

    check(
        "events/telemetry.ownship/payload",
        round_trip::<TelemetryOwnshipEvent>,
    );
    check(
        "events/telemetry.traffic/payload",
        round_trip::<Vec<TrafficPosition>>,
    );
    check("commands/atis.get/input", round_trip::<AtisGetCommandInput>);
    check("commands/atis.get/output", round_trip::<Vec<AtisReport>>);
    check(
        "commands/atis.get/errors",
        round_trip::<AtisGetCommandRefusal>,
    );
    check(
        "commands/flightplan.get/input",
        round_trip::<FlightplanGetCommandInput>,
    );
    check(
        "commands/flightplan.get/output",
        round_trip::<HashMap<String, Option<FlightPlan>>>,
    );
    check(
        "commands/flightplan.get/errors",
        round_trip::<FlightplanGetCommandRefusal>,
    );
    check("descriptor", round_trip_descriptor);
    check("handshakeErrors", round_trip::<HandshakeError>);

    // A channel added to the manifest fails here until its types are listed above.
    let expected: BTreeSet<String> = channels("events")
        .iter()
        .map(|name| format!("events/{name}/payload"))
        .chain(channels("commands").iter().flat_map(|name| {
            ["input", "output", "errors"].map(|part| format!("commands/{name}/{part}"))
        }))
        .chain(["descriptor", "handshakeErrors"].map(str::to_owned))
        .collect();
    assert_eq!(checked, expected);
}

fn round_trip<T: Serialize + DeserializeOwned>(sample: &Value) -> Value {
    let value: T =
        serde_json::from_value(sample.clone()).unwrap_or_else(|e| panic!("{e}: {sample}"));
    serde_json::to_value(value).unwrap()
}

/// The handshake descriptor is `SourceOptions` plus the `capabilities` that `Source::start` fills in.
fn round_trip_descriptor(sample: &Value) -> Value {
    let mut options = sample.clone();
    let capabilities = options
        .as_object_mut()
        .unwrap()
        .remove("capabilities")
        .unwrap();
    let mut descriptor = round_trip::<SourceOptions>(&options);
    descriptor["capabilities"] = capabilities;
    descriptor
}

fn check_schema(part: &str, schema: &Value, round_trip: fn(&Value) -> Value) {
    for full in [false, true] {
        for branch in 0..branches(schema) {
            let sample = sample(schema, schema, full, branch);
            let found = issues(schema, &sample);
            assert!(
                found.is_empty(),
                "{part}: the sample itself is invalid: {found:?}\n{sample}"
            );

            let back = round_trip(&sample);
            let found = issues(schema, &back);
            assert!(
                found.is_empty(),
                "{part}: serialized JSON is invalid: {found:?}\n{back}"
            );
            assert_eq!(
                without_nulls(back),
                without_nulls(sample),
                "{part}: lost or changed data"
            );
        }
    }
}

/// Absent and `null` optional fields mean the same to Simlink.
fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, without_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_nulls).collect()),
        value => value,
    }
}
