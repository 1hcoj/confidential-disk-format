# SPDX-License-Identifier: Apache-2.0

REGISTRY ?= localhost
IMAGE ?= cdf-csi
TAG ?= dev
VERSION ?= $(TAG)

CMD = cdf-csi
PKG = github.com/confidential-containers/confidential-disk-format/csi/cmd/cdf-csi

.PHONY: all build test image clean

all: build

build:
	go build -ldflags "-X main.version=$(VERSION)" -o bin/$(CMD) ./cmd/cdf-csi

test:
	go test ./...

image:
	docker build -t $(REGISTRY)/$(IMAGE):$(TAG) .

clean:
	rm -rf bin/
