FROM docker.io/rust:1-slim AS chef 
RUN cargo install cargo-chef 
WORKDIR app

FROM chef AS planner
COPY . .
RUN cargo chef prepare  --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin server

FROM docker.io/debian:stable-slim AS runtime
WORKDIR app
RUN apt-get update --yes && apt-get install --yes fuse3 sqlite3 ca-certificates
COPY --from=docker.io/flyio/litefs:0.5 /usr/local/bin/litefs /usr/local/bin/litefs
COPY litefs.yml /etc/litefs.yml
COPY --from=builder /app/target/release/server /usr/local/bin
ENTRYPOINT [ "litefs", "mount" ]
