# web-app image. Build metadata is passed in as ARGs by the build/release
# workflows and baked into ENV so the app can report it at /info (design §16).
# node:22-alpine ships busybox wget, which the platform health check + /info
# scrape use to probe the container.
FROM node:22-alpine
WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci --omit=dev

COPY . .

ARG VERSION=dev
ARG GIT_COMMIT=unknown
ARG BUILD_TIME=
ENV APP_VERSION=$VERSION \
    GIT_COMMIT=$GIT_COMMIT \
    BUILD_TIME=$BUILD_TIME \
    PORT=8080

EXPOSE 8080
CMD ["node", "server.js"]
