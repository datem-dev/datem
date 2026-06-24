FROM rust:1.82 AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# Compute the minimal recipe (dependency manifest) from the workspace.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Compile dependencies only — this layer is cached as long as Cargo.toml/Cargo.lock don't change.
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --package server --recipe-path recipe.json

# Build the real server binary on top of the cached deps.
COPY . .
RUN cargo build --release --package server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/datem /usr/local/bin/datem
EXPOSE 3000
CMD ["datem"]
