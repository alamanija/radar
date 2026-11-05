# syntax=docker/dockerfile:1
#
# Build the `radar-server` Axum binary into a small Debian runtime image.
# The desktop app (`radar-app`) and `radar-core` are part of the same Cargo
# workspace but are never compiled here — `cargo build -p radar-server` only
# pulls the server's dependency subtree. Every member's manifest must still be
# in the build context, so don't exclude them in `.dockerignore`.
#
# `migrations/` is embedded into the binary at compile time via
# `sqlx::migrate!()`, so the folder must be present while cargo runs.

# --------------------------------------------------------------------------
# builder
# --------------------------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /app

# Copy the whole workspace. `.dockerignore` excludes build artefacts, secrets
# and the desktop app's assets so the context stays small.
COPY . .

# `-p radar-server` makes cargo ignore the desktop-app members entirely.
RUN cargo build --release -p radar-server --bin radar-server

# --------------------------------------------------------------------------
# runtime
# --------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Drop root. Render / Fly / Railway all accept this, and it's
# belt-and-suspenders for any future local Docker runs.
RUN useradd --system --uid 10001 --create-home --shell /usr/sbin/nologin radar
USER radar
WORKDIR /home/radar

COPY --from=builder --chown=radar:radar /app/target/release/radar-server /usr/local/bin/radar-server

ENV RUST_LOG=radar_server=info,tower_http=info

# The server reads $PORT (Render / Fly / Heroku convention) first, then
# falls back to $SERVER_PORT, then 8787. EXPOSE is documentation only —
# Render/Fly route traffic to whatever $PORT the process actually binds.
EXPOSE 8787

ENTRYPOINT ["radar-server"]
