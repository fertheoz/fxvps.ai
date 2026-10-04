# syntax=docker/dockerfile:1.7
# Statik web imajı (apps/terminal = Vite, apps/backoffice = Next.js `output: "export"`).
# docker build -f infra/docker/static-web.Dockerfile --build-arg APP=terminal --build-arg OUT=dist -t fxvps-terminal .
# docker build -f infra/docker/static-web.Dockerfile --build-arg APP=backoffice --build-arg OUT=out -t fxvps-backoffice .
ARG NODE_VERSION=22

FROM node:${NODE_VERSION}-bookworm-slim AS build
ARG APP=terminal
ENV PNPM_HOME=/pnpm PATH=/pnpm:$PATH
RUN corepack enable
WORKDIR /src
COPY . .
# Her uygulamanın kendi pnpm-lock.yaml dosyası var; kök workspace yok sayılır.
WORKDIR /src/apps/${APP}
RUN pnpm install --ignore-workspace --frozen-lockfile && pnpm build

FROM nginxinc/nginx-unprivileged:1.27-alpine AS runtime
ARG APP=terminal
ARG OUT=dist
COPY infra/docker/nginx-spa.conf /etc/nginx/conf.d/default.conf
COPY --from=build /src/apps/${APP}/${OUT}/ /usr/share/nginx/html/
USER 101
EXPOSE 8080
