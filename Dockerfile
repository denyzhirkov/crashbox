# syntax=docker/dockerfile:1.7
# Multi-stage build for Crashbox:
#   1. Build the SolidJS frontend → /frontend/dist
#   2. Build the Rust backend (rust-embed includes the frontend dist at build time)
#   3. Copy the single statically-linked-SQLite binary into a minimal runtime image
#
# Final image is debian:bookworm-slim + one binary, ~70 MB.

# ----------------------------------------------------------------------------
# Stage 1 — frontend
# ----------------------------------------------------------------------------
FROM node:22-alpine AS frontend
WORKDIR /app

# Install pnpm via corepack — matches what's in the repo.
RUN corepack enable

# Cache deps separately from sources for faster rebuilds.
COPY frontend/package.json frontend/pnpm-lock.yaml ./
# Disable pnpm's minimum-release-age supply-chain check inside the build — the lockfile is
# already vetted at commit time on the developer's machine. Without this, freshly-published
# transitive deps fail the build.
ENV PNPM_CONFIG_MINIMUM_RELEASE_AGE=0
RUN pnpm install --frozen-lockfile

COPY frontend/ ./
RUN pnpm build

# ----------------------------------------------------------------------------
# Stage 2 — backend
# ----------------------------------------------------------------------------
FROM rust:1-bookworm AS backend
WORKDIR /build

# Cache cargo registry between builds via a named volume mount if available; otherwise this
# just copies sources and runs cargo. Workspaceless single-crate keeps it simple.
COPY backend/Cargo.toml backend/Cargo.lock* ./backend/
COPY backend/migrations ./backend/migrations
COPY backend/src ./backend/src
COPY backend/tests ./backend/tests

# rust-embed pulls in frontend/dist at compile time (path is `../frontend/dist/` relative to
# backend/src/http/assets.rs).
COPY --from=frontend /app/dist ./frontend/dist

# SQLx offline mode would speed this up further, but the macros we use (query, query_as) don't
# need precompile metadata at build time, so a plain build works.
RUN cd backend && cargo build --release --bin crashbox

# ----------------------------------------------------------------------------
# Stage 3 — runtime
# ----------------------------------------------------------------------------
# distroless/cc-debian12: bare glibc + ca-certs + a non-root `nonroot` user, no shell, no apt.
# ~24 MB base. Distroless is the right call here — Crashbox is a single static-ish binary
# that talks to /data via SQLite; we don't need debugging tools inside the container.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

COPY --from=backend /build/backend/target/release/crashbox /usr/local/bin/crashbox

USER nonroot:nonroot
WORKDIR /data
EXPOSE 8080
VOLUME ["/data"]

ENV CRASHBOX_HOST=0.0.0.0 \
    CRASHBOX_PORT=8080 \
    CRASHBOX_DATABASE_URL=sqlite:///data/crashbox.db \
    CRASHBOX_DATA_DIR=/data

# Healthchecks intentionally not declared — orchestrators (docker-compose, k8s) probe /healthz
# on the exposed port directly. Embedding curl/wget would bloat the image and break the
# distroless guarantee.

ENTRYPOINT ["/usr/local/bin/crashbox"]
