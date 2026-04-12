# Deployment

This guide covers packaging the application for production using Docker,
Kubernetes, and environment-based configuration.

## Docker

A minimal multi-stage `Dockerfile`:

```dockerfile
FROM rust:1.75-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/myapp /usr/local/bin/myapp
EXPOSE 8080
CMD ["myapp"]
```

Build and run:

```bash
docker build -t myapp:latest .
docker run -p 8080:8080 -e MYLIB_API_KEY=sk-live-xxx myapp:latest
```

## Docker Compose

For local development with a database:

```yaml
services:
  app:
    build: .
    ports: ["8080:8080"]
    environment:
      MYLIB_API_KEY: ${MYLIB_API_KEY}
    depends_on: [db]
  db:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD: secret
```

## Kubernetes

A minimal deployment manifest:

```yaml
apiVersion: apps/v1
kind: Deployment
spec:
  replicas: 3
  template:
    spec:
      containers:
        - name: myapp
          image: myapp:latest
          env:
            - name: MYLIB_API_KEY
              valueFrom:
                secretKeyRef:
                  name: myapp-secrets
                  key: api-key
```

## Health Checks

The application exposes `/healthz` (liveness) and `/readyz` (readiness)
endpoints.  Configure your orchestrator to probe these before routing traffic.

## Environment Setup

Set the following environment variables in production:

- `MYLIB_API_KEY` — required for authenticated requests
- `MYLIB_LOG_LEVEL=info` — recommended log verbosity
- `RUST_BACKTRACE=0` — suppress backtraces in production logs
