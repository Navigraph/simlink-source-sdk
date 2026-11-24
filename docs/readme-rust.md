Connect a flight simulator, add-on, or tool to Navigraph Simlink v2 as a **source**: emit telemetry and answer commands without implementing the Simlink protocol.

## Installing

```sh
cargo add simlink-source-sdk
```

Requires Rust 1.95+.

## Quickstart

<!-- snippet: crates/simlink-source-sdk/examples/quickstart.rs -->
```rust
//! Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.

use std::time::Duration;

use simlink_source_sdk::*;

fn main() -> Result<(), Error> {
    let options = SourceOptions {
        id: "quickstart".into(),
        display_name: "Quickstart".into(),
    };
    let source = Source::new(options, vec![Event::TelemetryOwnship]);
    source.start()?;

    let mut latitude = 59.6519;
    loop {
        latitude += 0.0001;
        source.emit_telemetry_ownship(ownship(latitude, 17.9186))?;
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// In a real source these values come from the simulator.
fn ownship(latitude: f64, longitude: f64) -> TelemetryOwnshipEvent {
    TelemetryOwnshipEvent {
        aircraft: OwnshipAircraft {
            position: Coordinates {
                latitude,
                longitude,
            },
            altitude: OwnshipAltitude {
                true_: 1200.0,
                agl: 900.0,
                indicated: 1200.0,
                pressure: None,
            },
            speed: OwnshipSpeed {
                gs: 140.0,
                ias: 135.0,
                tas: 138.0,
            },
            heading: OwnshipHeading {
                true_: 32.0,
                magnetic: 25.0,
            },
            track: OwnshipTrack { true_: 31.0 },
            attitude: OwnshipAttitude {
                bank: 0.0,
                pitch: 2.5,
            },
            vertical_speed: 0.0,
            turn_rate: 0.0,
            gforce: 1.0,
            transponder: Some("7000".into()),
            category: Some(AircraftCategory {
                type_: AircraftCategoryType::Piston,
                operator_type: None,
            }),
        },
        simulation: OwnshipSimulation {
            paused: false,
            slew: false,
            time: None,
        },
    }
}
```

## Learn more

- [Key concepts](https://github.com/Navigraph/simlink-source-sdk#key-concepts) and [errors](https://github.com/Navigraph/simlink-source-sdk#errors)
- [Examples](https://github.com/Navigraph/simlink-source-sdk/tree/main/crates/simlink-source-sdk/examples)
