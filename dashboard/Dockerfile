# syntax=docker/dockerfile:1
# Dashboard image: build the Vite/React app, serve the static bundle via nginx
# (which also proxies /api/bot and /api/recon to the WG-internal APIs).
FROM node:22-slim AS build
WORKDIR /app
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM nginx:alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
