# rust-service image. Build metadata is passed in as ARGs by the build/release
# workflows and baked into ENV so the app can report it at /info (design §16).
# The alpine runtime ships busybox wget, which the platform health check + /info
# scrape use to probe the container.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --locked

FROM alpine:3
WORKDIR /app
COPY --from=build /app/target/release/app /usr/local/bin/app

ARG VERSION=dev
ARG GIT_COMMIT=unknown
ARG BUILD_TIME=
ENV APP_VERSION=$VERSION \
    GIT_COMMIT=$GIT_COMMIT \
    BUILD_TIME=$BUILD_TIME \
    PORT=8080

EXPOSE 8080
CMD ["app"]
