FROM rust:1.91-slim

RUN USER=root mkdir -p /app
RUN USER=root groupadd app && useradd --home-dir /app -g app app
RUN USER=root chown -R app:app /app

USER app
WORKDIR /app
RUN cargo new --bin lwproxy

# Create a dummy image to build dependencies
# This makes rebuilding the docker image faster,
# since the build of dependencies can be cached.
COPY --chown=app:app ./Cargo.lock /app/lwproxy/Cargo.lock
COPY --chown=app:app ./Cargo.toml /app/lwproxy/Cargo.toml

WORKDIR /app/lwproxy
RUN cargo build --release
RUN rm -rf src && rm target/release/lwproxy

# Now copy the lwproxy source and build for real
COPY --chown=app:app ./src /app/lwproxy/src
RUN touch src/main.rs && cargo build --release

CMD ["./target/release/lwproxy"]
