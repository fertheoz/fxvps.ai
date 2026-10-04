# syntax=docker/dockerfile:1.7
# Statik web imajı (apps/terminal = Vite, apps/backoffice = Next.js `output: "export"`).
# docker build -f infra/docker/static-web.Dockerfile --build-arg APP=terminal --build-arg OUT=dist -t fxvps-terminal .
# docker build -f infra/docker/static-web.Dockerfile --build-arg APP=backoffice --build-arg OUT=out -t fxvps-backoffice .
ARG NODE_VERSION=22

FROM node:${NODE_VERSION}-bookworm-slim AS build
ARG APP=terminal
# Build-time public config (inlined into the bundle; not secrets).
# Terminal: gateway origins allowed to receive the identity token (finding G3).
ARG VITE_IDENTITY_URL=""
ARG VITE_ALLOWED_WS_ORIGINS=""
# Back office: admin API origin (also the CSP connect-src).
ARG NEXT_PUBLIC_API_URL=""
ENV VITE_IDENTITY_URL=${VITE_IDENTITY_URL} VITE_ALLOWED_WS_ORIGINS=${VITE_ALLOWED_WS_ORIGINS} NEXT_PUBLIC_API_URL=${NEXT_PUBLIC_API_URL}
ENV PNPM_HOME=/pnpm PATH=/pnpm:$PATH
RUN corepack enable
WORKDIR /src
COPY . .
# Her uygulamanın kendi pnpm-lock.yaml dosyası var; kök workspace yok sayılır.
WORKDIR /src/apps/${APP}
RUN pnpm install --ignore-workspace --frozen-lockfile && pnpm build

FROM nginxinc/nginx-unprivileged:1.31-alpine@sha256:26b0bf6fbf07297983cb341998d79c831508787de26627dd2a112321b9c3a4af AS runtime
ARG APP=terminal
ARG OUT=dist
COPY infra/docker/nginx-spa.conf /etc/nginx/conf.d/default.conf
COPY infra/docker/nginx-security-headers.conf /etc/nginx/fxvps-security-headers.conf
COPY --from=build /src/apps/${APP}/${OUT}/ /usr/share/nginx/html/
USER 101
EXPOSE 8080
