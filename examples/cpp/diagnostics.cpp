// Prints the SDK's logs, connection changes and anything Simlink rejects. Stop with Ctrl+C.
#include <chrono>
#include <cstdio>
#include <thread>
#include <variant>

#include <simlink_source_sdk.hpp>

const char* name(simlink_source_sdk::LogLevel level) {
    using simlink_source_sdk::LogLevel;
    switch (level) {
        case LogLevel::kError: return "Error";
        case LogLevel::kWarn: return "Warn";
        case LogLevel::kInfo: return "Info";
        case LogLevel::kDebug: return "Debug";
        case LogLevel::kTrace: return "Trace";
    }
    return "?";
}

struct Log : simlink_source_sdk::LogListener {
    void log(const simlink_source_sdk::LogLevel& level, const std::string& message) override {
        std::fprintf(stderr, "[%s] %s\n", name(level), message.c_str());
    }
};

struct State : simlink_source_sdk::StateListener {
    void changed(simlink_source_sdk::ConnectionState state) override {
        using simlink_source_sdk::ConnectionState;
        const auto& variant = state.get_variant();
        if (std::holds_alternative<ConnectionState::kSearching>(variant)) {
            std::printf("Searching for Simlink\n");
        } else if (const auto* connected = std::get_if<ConnectionState::kConnected>(&variant)) {
            std::printf("Connected to %s\n", connected->address.c_str());
        } else if (const auto* rejected = std::get_if<ConnectionState::kRejected>(&variant)) {
            std::printf("Refused (%s): %s%s\n", rejected->code.c_str(), rejected->message.c_str(),
                        rejected->retrying ? ", retrying" : "");
            using Invalid = simlink_source_sdk::HandshakeError::kHandshakeIdentificationInvalid;
            const auto* invalid = rejected->error ? std::get_if<Invalid>(&rejected->error->get_variant()) : nullptr;
            if (invalid) {
                for (const auto& issue : invalid->issues) {
                    std::printf("  %s\n", issue.c_str());
                }
            }
        } else {
            std::printf("Unknown connection state\n");
        }
        std::fflush(stdout);
    }
};

struct Rejections : simlink_source_sdk::RejectionListener {
    void rejected(const simlink_source_sdk::Rejection& rejection) override {
        std::printf("Simlink rejected %s (%s): %s\n", rejection.channel.wire_name().c_str(),
                    rejection.code.c_str(), rejection.message.value_or("").c_str());
        for (const auto& issue : rejection.issues) {
            std::printf("  %s: %s\n", issue.path.c_str(), issue.message.c_str());
        }
        std::fflush(stdout);
    }
};

int main() {
    simlink_source_sdk::set_logger(std::make_shared<Log>(), simlink_source_sdk::LogLevel::kDebug);

    simlink_source_sdk::SourceOptions options{};
    options.id = "diagnostics";
    options.display_name = "Diagnostics";

    auto source = simlink_source_sdk::Source::init(options, {simlink_source_sdk::Event::kTelemetryTraffic});
    source->on_state(std::make_shared<State>());
    source->on_rejected(std::make_shared<Rejections>());
    source->start();

    for (;;) {
        // An empty traffic picture is valid; a real source sends the aircraft around the user.
        // Emitting fails for an undeclared event, before `start`, or over Simlink's message size limit.
        try {
            source->emit_telemetry_traffic({});
        } catch (const simlink_source_sdk::SimlinkError& error) {
            std::printf("Could not emit: %s\n", error.what());
        }
        std::this_thread::sleep_for(std::chrono::seconds(1));
    }
}
