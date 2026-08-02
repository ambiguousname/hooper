FROM rust:1.97.1-trixie AS build
WORKDIR /app

# From https://docs.docker.com/guides/rust/

ARG APP_NAME=hooper

RUN --mount=type=bind,source=src,target=src \
    --mount=type=bind,source=Cargo.toml,target=Cargo.toml \
    --mount=type=bind,source=Cargo.lock,target=Cargo.lock \
    --mount=type=cache,target=/app/target/ \
    --mount=type=cache,target=/var/cache/cargo \
    CARGO_HOME=/var/cache/cargo cargo build --locked --release && \
	cp /app/target/release/$APP_NAME /bin/server

FROM debian:trixie AS final

ENV PORT=4000

RUN apt-get update

RUN apt-get install adduser

ARG UID=10001
RUN adduser \
    --disabled-password \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    appuser

USER appuser

WORKDIR /app

COPY --from=build /bin/server /bin/

ADD ./public /app/public

EXPOSE ${PORT}

# TODO: TLS cert

ENTRYPOINT ["/bin/server"]