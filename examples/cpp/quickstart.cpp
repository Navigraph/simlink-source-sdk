// Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.
#include <chrono>
#include <thread>

#include <simlink_source_sdk.hpp>

// In a real source these values come from the simulator.
simlink_source_sdk::TelemetryOwnship ownship(double latitude, double longitude) {
    simlink_source_sdk::TelemetryOwnship ownship{};
    auto& aircraft = ownship.aircraft;
    aircraft.position = {latitude, longitude};
    aircraft.altitude.true_ = 1200;
    aircraft.altitude.agl = 900;
    aircraft.altitude.indicated = 1200;
    aircraft.speed = {.gs = 140, .ias = 135, .tas = 138};
    aircraft.heading = {.magnetic = 25, .true_ = 32};
    aircraft.track.true_ = 31;
    aircraft.attitude = {.bank = 0, .pitch = 2.5};
    aircraft.gforce = 1;
    aircraft.transponder = "7000";
    return ownship;
}

int main() {
    simlink_source_sdk::SourceOptions options{};
    options.id = "quickstart";
    options.display_name = "Quickstart";

    auto source = simlink_source_sdk::Source::init(options, {simlink_source_sdk::Event::kTelemetryOwnship});
    source->start();

    double latitude = 59.6519;
    for (;;) {
        latitude += 0.0001;
        source->emit_telemetry_ownship(ownship(latitude, 17.9186));
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
    }
}
