//! Emits the traffic around the aircraft once a second and answers flight plan queries for it. Stop with Ctrl+C.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use simlink::*;

struct FlightPlans;

impl FlightplanGetHandler for FlightPlans {
    fn handle(&self, input: FlightplanGetInput, reply: Arc<FlightplanGetReply>) {
        // Every requested key gets an entry; `None` when this source has no plan for it.
        let plans: HashMap<String, Option<FlightPlan>> = input
            .keys
            .into_iter()
            .map(|key| {
                let plan = flight_plan(&key);
                (key, plan)
            })
            .collect();
        if let Err(error) = reply.ok(plans) {
            eprintln!("Could not answer flightplan.get: {error}");
        }
    }
}

fn main() -> Result<(), Error> {
    let options = SourceOptions {
        id: "traffic".into(),
        display_name: "Traffic".into(),
    };
    let source = Source::new(options, vec![Event::TelemetryTraffic]);
    source.on_flightplan_get(Arc::new(FlightPlans))?;
    source.start()?;

    loop {
        // The whole picture each time: aircraft missing from it have left.
        source.emit_telemetry_traffic(traffic())?;
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// In a real source these come from the simulator, each with a key that stays the same while it is in range.
fn traffic() -> Vec<TrafficPosition> {
    vec![
        // Identified by ICAO type designator, with every optional field filled in.
        TrafficPosition {
            key: "EZY382".into(),
            aircraft: TrafficPositionAircraft::Type {
                type_: "A320".into(),
                callsign: Some("EZY382".into()),
                position: Coordinates {
                    latitude: 59.70,
                    longitude: 17.95,
                },
                altitude: TrafficAltitude {
                    true_: 4500.0,
                    agl: Some(4300.0),
                    indicated: Some(4480.0),
                    pressure: Some(4520.0),
                },
                speed: TrafficSpeed {
                    gs: 250.0,
                    ias: Some(220.0),
                    tas: Some(236.0),
                },
                vertical_speed: Some(-800.0),
                heading: TrafficHeading {
                    true_: 190.0,
                    magnetic: Some(184.0),
                },
                attitude: TrafficAttitude {
                    bank: Some(0.0),
                    pitch: Some(-1.5),
                },
                transponder: Some("4521".into()),
            },
        },
        // Identified by category when the exact type is unknown, with only the required fields.
        TrafficPosition {
            key: "SEJKL".into(),
            aircraft: TrafficPositionAircraft::Category {
                category: TrafficCategory::Helicopter,
                type_: None,
                callsign: None,
                position: Coordinates {
                    latitude: 59.64,
                    longitude: 17.90,
                },
                altitude: TrafficAltitude {
                    true_: 800.0,
                    agl: None,
                    indicated: None,
                    pressure: None,
                },
                speed: TrafficSpeed {
                    gs: 90.0,
                    ias: None,
                    tas: None,
                },
                vertical_speed: None,
                heading: TrafficHeading {
                    true_: 45.0,
                    magnetic: None,
                },
                attitude: TrafficAttitude {
                    bank: None,
                    pitch: None,
                },
                // Unset means "1200".
                transponder: None,
            },
        },
    ]
}

/// The plan filed for a traffic key, if any.
fn flight_plan(key: &str) -> Option<FlightPlan> {
    (key == "EZY382").then(|| FlightPlan {
        key: key.into(),
        callsign: "EZY382".into(),
        pilot_name: "Jane Doe".into(),
        aircraft: "A320".into(),
        origin: FlightPlanAirport {
            ident: "EGKK".into(),
            coords: Some(FlightPlanAirportCoords {
                lat: 51.148,
                lon: -0.190,
            }),
        },
        // Coordinates are optional.
        destination: FlightPlanAirport {
            ident: "ESSA".into(),
            coords: None,
        },
        route: "EGKK DCT ESSA".into(),
        stats: FlightPlanStats {
            dep_dist: Some(712.0),
            to_go_dist: Some(18.0),
            etd: Some("2026-10-19T09:05:00Z".into()),
            eta: Some("2026-10-19T11:40:00Z".into()),
        },
        status: FlightPlanStatus::Arriving,
    })
}
