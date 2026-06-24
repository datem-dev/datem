# Build a Lambda container image for either the ingest or billing function.
#
# Usage (from workspace root):
#   docker build -f lambda/Dockerfile --build-arg FUNCTION=ingest  -t datem-ingest  .
#   docker build -f lambda/Dockerfile --build-arg FUNCTION=billing -t datem-billing .
#
# FUNCTION must be one of: ingest | billing
# The resulting image targets the AWS Lambda provided.al2023 custom runtime.

ARG FUNCTION=ingest

FROM rust:1.82 AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# Compute the minimal recipe (dependency manifest) from the workspace.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Compile dependencies only — cached as long as Cargo.toml/Cargo.lock don't change.
FROM chef AS builder
ARG FUNCTION
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --package lambda --recipe-path recipe.json

# Build the real lambda binaries on top of the cached deps.
COPY . .
RUN cargo build --release --package lambda

# Rename the chosen binary to 'bootstrap' — required by the Lambda custom runtime.
RUN cp /app/target/release/datem-${FUNCTION} /app/bootstrap

FROM public.ecr.aws/lambda/provided:al2023
COPY --from=builder /app/bootstrap ${LAMBDA_RUNTIME_DIR}/bootstrap
CMD ["bootstrap"]
