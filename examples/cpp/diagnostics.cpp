// Prints the SDK's logs, connection changes and anything Simlink rejects. Stop with Ctrl+C.
#include <chrono>
#include <cstdio>
#include <thread>

#include <simlink.hpp>

struct Log : simlink::LogListener {
    void log(const simlink::LogLevel& level, const std::string& message) override {
        std::fprintf(stderr, "[%d] %s\n", static_cast<int>(level), message.c_str());
    }
};

struct State : simlink::StateListener {
    void changed(simlink::ConnectionState state) override {
        using simlink::ConnectionState;
        const auto& variant = state.get_variant();
        if (const auto* connected = std::get_if<ConnectionState::kConnected>(&variant)) {
            std::printf("Connected to %s\n", connected->address.c_str());
        } else if (const auto* rejected = std::get_if<ConnectionState::kRejected>(&variant)) {
            std::printf("Refused (%s): %s%s\n", rejected->code.c_str(), rejected->message.c_str(),
                        rejected->retrying ? ", retrying" : "");
        } else {
            std::printf("Searching for Simlink\n");
        }
        std::fflush(stdout);
    }
};

struct Rejections : simlink::RejectionListener {
    void rejected(const simlink::Rejection& rejection) override {
        std::printf("Simlink rejected %s (%s): %s\n", rejection.channel.c_str(),
                    rejection.code.c_str(), rejection.message.c_str());
        for (const auto& issue : rejection.issues) {
            std::printf("  %s: %s\n", issue.path.c_str(), issue.message.c_str());
        }
        std::fflush(stdout);
    }
};

int main() {
    simlink::set_logger(std::make_shared<Log>(), simlink::LogLevel::kDebug);

    simlink::SourceOptions options{};
    options.id = "diagnostics";
    options.display_name = "Diagnostics";

    auto source = simlink::Source::init(options, {simlink::Event::kTelemetryTraffic});
    source->on_state(std::make_shared<State>());
    source->on_rejected(std::make_shared<Rejections>());
    source->start();

    for (;;) {
        // An empty traffic picture is valid; a real source sends the aircraft around the user.
        // Emitting fails for an undeclared event, before `start`, or over Simlink's message size limit.
        try {
            source->emit_telemetry_traffic({});
        } catch (const simlink::SimlinkError& error) {
            std::printf("Could not emit: %s\n", error.what());
        }
        std::this_thread::sleep_for(std::chrono::seconds(1));
    }
}
