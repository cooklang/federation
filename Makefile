# Variables
IMAGE_NAME := $(shell yq '.services.app.image' docker-compose.yml)

# Targets
.PHONY: build push all

# Build the Docker image
build:
	docker build -t $(IMAGE_NAME) .

# Push the Docker image to registry
push:
	docker push $(IMAGE_NAME)

# Build and push in one command
all: build push
