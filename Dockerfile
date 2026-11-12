# Linux build and test environment, matching the ubuntu-26.04 CI job. See docs/development.md.
FROM ubuntu:26.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential ca-certificates cmake curl git libicu78 \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:/usr/share/dotnet:$PATH \
    DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
# The versions rust-toolchain.toml and global.json pin.
COPY rust-toolchain.toml global.json /tmp/
RUN cd /tmp \
    && curl -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none \
    && rustup toolchain install \
    && curl -fsSL https://dot.net/v1/dotnet-install.sh | bash -s -- --jsonfile global.json --install-dir /usr/share/dotnet \
    && rm rust-toolchain.toml global.json

# MSBuild reads these as properties: .NET output goes to the container's target volume, not obj and bin folders the
# host shares.
ENV UseArtifactsOutput=true ArtifactsPath=/work/target/dotnet

WORKDIR /work
