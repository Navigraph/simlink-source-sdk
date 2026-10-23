// Emits the traffic around the aircraft once a second and answers flight plan queries for it. Stop with Ctrl+C.
#include <chrono>
#include <thread>

#include <simlink.hpp>

// In a real source these come from the simulator, each with a key that stays the same while it is in range.
std::vector<simlink::TrafficPosition> traffic() {
    // Identified by ICAO type designator, with every optional field filled in.
    simlink::TrafficPositionAircraft::kType airliner{};
    airliner.type = "A320";
    airliner.callsign = "EZY382";
    airliner.position = {.latitude = 59.70, .longitude = 17.95};
    airliner.altitude = {.agl = 4300, .indicated = 4480, .pressure = 4520, .true_ = 4500};
    airliner.speed = {.gs = 250, .ias = 220, .tas = 236};
    airliner.vertical_speed = -800;
    airliner.heading = {.magnetic = 184, .true_ = 190};
    airliner.attitude = {.bank = 0, .pitch = -1.5};
    airliner.transponder = "4521";

    // Identified by category when the exact type is unknown, with only the required fields.
    // An unset transponder means "1200".
    simlink::TrafficPositionAircraft::kCategory helicopter{};
    helicopter.category = simlink::TrafficCategory::kHelicopter;
    helicopter.position = {.latitude = 59.64, .longitude = 17.90};
    helicopter.altitude = {.true_ = 800};
    helicopter.speed = {.gs = 90};
    helicopter.heading = {.true_ = 45};

    return {
        {.aircraft = airliner, .key = "EZY382"},
        {.aircraft = helicopter, .key = "SEJKL"},
    };
}

// The plan filed for a traffic key, if any.
std::optional<simlink::FlightPlan> flight_plan(const std::string& key) {
    if (key != "EZY382") return std::nullopt;

    simlink::FlightPlan plan{};
    plan.key = key;
    plan.callsign = "EZY382";
    plan.pilot_name = "Jane Doe";
    plan.aircraft = "A320";
    plan.origin = {.coords = simlink::FlightPlanAirportCoords{.lat = 51.148, .lon = -0.190}, .ident = "EGKK"};
    // Coordinates are optional.
    plan.destination = {.ident = "ESSA"};
    plan.route = "EGKK DCT ESSA";
    plan.stats = {.dep_dist = 712, .eta = "2026-10-19T11:40:00Z", .etd = "2026-10-19T09:05:00Z", .to_go_dist = 18};
    plan.status = simlink::FlightPlanStatus::kArriving;
    return plan;
}

struct FlightPlans : simlink::FlightplanGetHandler {
    void handle(const simlink::FlightplanGetInput& input,
                const std::shared_ptr<simlink::FlightplanGetReply>& reply) override {
        // Every requested key gets an entry; std::nullopt when this source has no plan for it.
        std::unordered_map<std::string, std::optional<simlink::FlightPlan>> plans;
        for (const auto& key : input.keys) plans[key] = flight_plan(key);
        reply->ok(plans);
    }
};

int main() {
    simlink::SourceOptions options{};
    options.id = "traffic";
    options.display_name = "Traffic";

    auto source = simlink::Source::init(options, {simlink::Event::kTelemetryTraffic});
    source->on_flightplan_get(std::make_shared<FlightPlans>());
    source->start();

    for (;;) {
        // The whole picture each time: aircraft missing from it have left.
        source->emit_telemetry_traffic(traffic());
        std::this_thread::sleep_for(std::chrono::seconds(1));
    }
}
