//! Answers Simlink's ATIS and flight plan commands. Stop with Ctrl+C.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use simlink::*;

struct Atis;

impl AtisGetHandler for Atis {
    fn handle(&self, input: AtisGetInput, reply: Arc<AtisGetReply>) {
        let answered = if input.icao != "ESSA" {
            reply.refuse(AtisGetError::AtisNotAvailable)
        } else {
            reply.ok(vec![AtisReport {
                icao: input.icao,
                type_: AtisReportType::Arr,
                letter: Some("A".into()),
                phonetic: Some("Alpha".into()),
                message: "Stockholm Arlanda information Alpha".into(),
                date_issued: "2026-10-19T12:00:00Z".into(),
                arriving_runways: vec!["19R".into()],
                departing_runways: vec!["19L".into()],
            }])
        };
        if let Err(error) = answered {
            eprintln!("Could not answer atis.get: {error}");
        }
    }
}

struct FlightPlans;

impl FlightplanGetHandler for FlightPlans {
    fn handle(&self, input: FlightplanGetInput, reply: Arc<FlightplanGetReply>) {
        // No flight plans here; a real source looks each key up.
        let plans: HashMap<String, Option<FlightPlan>> =
            input.keys.into_iter().map(|key| (key, None)).collect();
        if let Err(error) = reply.ok(plans) {
            eprintln!("Could not answer flightplan.get: {error}");
        }
    }
}

fn main() -> Result<(), Error> {
    let options = SourceOptions {
        id: "commands".into(),
        display_name: "Commands".into(),
    };
    let source = Source::new(options, vec![]);
    source.on_atis_get(Arc::new(Atis))?;
    source.on_flightplan_get(Arc::new(FlightPlans))?;
    source.start()?;

    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
