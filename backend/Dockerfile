FROM docker.io/rust:1-slim AS chef 
RUN cargo install cargo-chef 

FROM chef AS planner
WORKDIR /flylinks
COPY . .
RUN cargo chef prepare  --recipe-path recipe.json

FROM chef AS builder
WORKDIR /flylinks
COPY --from=planner /flylinks/recipe.json .
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin server

FROM docker.io/debian:stable-slim AS runtime
# We need to talk to AWS over TLS, so we need some baseline certificate authority info
RUN apt-get update && apt-get install -y ca-certificates
COPY --from=builder /flylinks/target/release/server /flylinks/bin/flylinks-backend
ENTRYPOINT [                                             \
    "/flylinks/bin/flylinks-backend",                    \
    "--s3-bucket=flylinks-us-west-2",                    \
    "--s3-region=us-west-2",                             \
    "--s3-path=flylinks.sqlite",                         \
    "--db-path=/flylinks/data/flylinks.sqlite",          \
    "--backup-staging-path=/flylinks/data/backup.sqlite" \
]
