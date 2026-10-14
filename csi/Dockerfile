# SPDX-License-Identifier: Apache-2.0
FROM golang:1.22-bookworm AS builder
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
ARG VERSION=dev
RUN CGO_ENABLED=0 go build -ldflags "-X main.version=${VERSION}" -o /cdf-csi ./cmd/cdf-csi

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends util-linux ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /cdf-csi /usr/local/bin/cdf-csi
# cdf-gpt must be supplied on the host or baked into a custom image.
ENTRYPOINT ["/usr/local/bin/cdf-csi"]
